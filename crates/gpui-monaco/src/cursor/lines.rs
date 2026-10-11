//! Whole-line editing commands.
//!
//! Ports Monaco's `src/vs/editor/contrib/linesOperations/browser/linesOperations.ts`
//! (indent/outdent lines, copy/move/delete lines, insert line above/below),
//! `copyLinesCommand.ts`, `moveLinesCommand.ts` and
//! `EnterOperation.lineInsertBefore`
//! / `lineInsertAfter` from
//! `src/vs/editor/common/cursor/cursorTypeEditOperations.ts` (MIT).

use super::auto_indent_paste::{generate_indent, space_count};
use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType,
    ReplaceCommandWithoutChangingPosition, TrackedId, execute_commands,
};
use super::language::{
    CompleteEnterAction, IndentConsts, LanguageContext, LanguageSupport, VirtualModel,
    enter_action, indent_metadata, rules_good_indent_for_line, unshift_indent,
};
use super::typing::{enter_command, shift};
use super::{
    AutoIndent, CursorCollection, CursorConfig, CursorModel, Position, Selection, last_row,
    line_units, strings,
};
use syntax::IndentAction;

type Commands = Vec<Option<Box<dyn Command>>>;

/// Monaco `Selection.setEndPosition`, which keeps the direction.
fn set_end_position(selection: Selection, end: Position) -> Selection {
    Selection::from_range(selection.start()..end, selection.is_reversed())
}

/// Indents the lines of every selection (Monaco `editor.action.indentLines`,
/// Cmd+]).
pub fn indent_lines(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    shift(config, model, cursors, false)
}

/// Outdents the lines of every selection (Monaco
/// `editor.action.outdentLines`, Cmd+[; same as Shift+Tab).
pub fn outdent_lines(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    shift(config, model, cursors, true)
}

// ---------------------------------------------------------------------------
// Move lines

struct MoveLinesCommand<'a> {
    selection: Selection,
    down: bool,
    config: &'a CursorConfig,
    /// The language, for `autoIndent: full` reindentation.
    lang: Option<&'a LanguageSupport<'a>>,
    tracked: Option<TrackedId>,
    move_end_position_down: bool,
    move_end_line_selection_shrink: bool,
}

/// JavaScript `str.replace(/^\s+/, '')`.
fn trim_start(s: &str) -> &str {
    s.trim_start_matches(char::is_whitespace)
}

impl MoveLinesCommand<'_> {
    fn space_count(&self, indentation: &str) -> i64 {
        space_count(indentation, self.config.tab_size)
    }

    fn generate_indent(&self, spaces: i64) -> String {
        generate_indent(spaces, self.config.tab_size, self.config.insert_spaces)
    }

    /// Monaco `MoveLinesCommand.parseEnterResult`: how far row `row`'s
    /// indentation moves for `enter`, when the row's text contains the
    /// enter action's prefix.
    fn parse_enter_result(
        &self,
        lang: &LanguageSupport,
        model: &dyn CursorModel,
        row: u32,
        enter: Option<CompleteEnterAction>,
    ) -> Option<i64> {
        let enter = enter?;
        let enter_prefix = match enter.indent_action {
            IndentAction::None | IndentAction::Indent => {
                format!("{}{}", enter.indentation, enter.append_text)
            }
            IndentAction::IndentOutdent => enter.indentation.clone(),
            IndentAction::Outdent => format!(
                "{}{}",
                unshift_indent(self.config, &enter.indentation),
                enter.append_text
            ),
        };
        let moving_line_text = model.line_content(row);
        if !trim_start(&moving_line_text).contains(trim_start(&enter_prefix)) {
            return None;
        }
        let old_indentation = strings::leading_whitespace(&moving_line_text);
        let mut new_indentation = strings::leading_whitespace(&enter_prefix).to_string();
        if indent_metadata(lang, model, row)
            .is_some_and(|metadata| metadata & IndentConsts::DECREASE_MASK != 0)
        {
            new_indentation = unshift_indent(self.config, &new_indentation);
        }
        Some(self.space_count(&new_indentation) - self.space_count(old_indentation))
    }

    /// Monaco `getEnterAction` at the end of row `row`.
    fn enter_action_at_end(
        &self,
        lang: &LanguageSupport,
        model: &dyn CursorModel,
        row: u32,
    ) -> Option<CompleteEnterAction> {
        let end = Position::new(row, model.line_max_column(row));
        enter_action(self.config.auto_indent, lang, model, &(end..end))
    }

    /// Monaco `MoveLinesCommand.matchEnterRule`: the onEnter rules of the
    /// nearest non-blank row at or above `one_row_above`, applied to `row`.
    fn match_enter_rule(
        &self,
        lang: &LanguageSupport,
        model: &dyn CursorModel,
        row: u32,
        one_row_above: i64,
    ) -> Option<i64> {
        let mut valid = one_row_above;
        while valid >= 0 {
            let units = line_units(model, valid as u32);
            if strings::last_non_whitespace_index(&units, None).is_some() {
                break;
            }
            valid -= 1;
        }
        if valid < 0 || row >= model.line_count() {
            return None;
        }
        let enter = self.enter_action_at_end(lang, model, valid as u32);
        self.parse_enter_result(lang, model, row, enter)
    }

    /// Monaco `MoveLinesCommand.matchEnterRuleMovingDown`: the onEnter rules
    /// of the row that will be above `row` (`future_above_row`, whose text
    /// becomes `future_above_text`), applied to `row`.
    fn match_enter_rule_moving_down(
        &self,
        lang: &LanguageSupport,
        model: &dyn CursorModel,
        row: u32,
        future_above_row: u32,
        future_above_text: &str,
    ) -> Option<i64> {
        let units: Vec<u16> = future_above_text.encode_utf16().collect();
        if strings::last_non_whitespace_index(&units, None).is_some() {
            let enter = self.enter_action_at_end(lang, model, future_above_row);
            self.parse_enter_result(lang, model, row, enter)
        } else {
            self.match_enter_rule(lang, model, row, row as i64 - 1)
        }
    }

    /// Monaco `MoveLinesCommand.shouldAutoIndent`. Not ported: the
    /// `isCheapToTokenize` and embedded-language checks (the token types
    /// always come from the [`LanguageContext`], for one language).
    fn should_auto_indent(&self) -> Option<&'_ LanguageSupport<'_>> {
        if self.config.auto_indent < AutoIndent::Full {
            return None;
        }
        self.lang.filter(|lang| lang.has_indentation_rules())
    }

    /// Monaco `MoveLinesCommand.getIndentEditsOfMovingBlock`: shifts the
    /// indentation of the rows of `s` by `offset` columns.
    fn indent_edits_of_moving_block(
        &mut self,
        model: &dyn CursorModel,
        builder: &mut EditBuilder,
        s: Selection,
        offset: i64,
    ) {
        for row in s.start().row..=s.end().row {
            let line = model.line_content(row);
            let original_indent = strings::leading_whitespace(&line);
            let new_indent = self.generate_indent(self.space_count(original_indent) + offset);
            if new_indent != original_indent {
                let indent_len = strings::utf16_len(original_indent);
                builder.add_edit(
                    Position::new(row, 0)..Position::new(row, indent_len),
                    &new_indent,
                );
                if row == s.end().row && s.end().column <= indent_len && new_indent.is_empty() {
                    // Part of the original indentation was selected; the
                    // selection's end shrinks with it.
                    self.move_end_line_selection_shrink = true;
                }
            }
        }
    }

    /// How many columns the indentation `good_indent` (from
    /// `rules_good_indent_for_line`) moves `old_indentation`; `None` when
    /// it stays.
    fn indent_shift(&self, good_indent: Option<String>, old_indentation: &str) -> Option<i64> {
        let new = self.space_count(&good_indent?);
        let old = self.space_count(old_indentation);
        (new != old).then_some(new - old)
    }

    fn move_down(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder, s: Selection) {
        let moving = s.end().row + 1;
        let moving_text = model.line_content(moving);
        // Delete the line that moves.
        builder.add_edit(
            Position::new(moving - 1, model.line_max_column(moving - 1))
                ..Position::new(moving, model.line_max_column(moving)),
            "",
        );
        let start_row = s.start().row;
        let insert_at = Position::new(start_row, 0)..Position::new(start_row, 0);
        let Some(lang) = self.should_auto_indent() else {
            // Insert the line that moves before the block.
            builder.add_edit(insert_at, &format!("{moving_text}\n"));
            return;
        };
        let mut inserting_text = moving_text.clone();
        let old_indentation = strings::leading_whitespace(&moving_text);
        if let Some(enter_shift) = self.match_enter_rule(lang, model, moving, start_row as i64 - 1)
        {
            // The onEnter rules of the row above the block still apply.
            let new_spaces = enter_shift + self.space_count(old_indentation);
            inserting_text = format!(
                "{}{}",
                self.generate_indent(new_spaces),
                trim_start(&moving_text)
            );
        } else {
            // No onEnter rule matches; try the indentation rules.
            let map = |line: u32| {
                if line == start_row + 1 {
                    moving + 1
                } else {
                    line
                }
            };
            let vm = VirtualModel::new(model, lang).with_line_map(&map);
            let good = rules_good_indent_for_line(self.config, &vm, start_row + 1);
            if let Some(shift) = self.indent_shift(good, old_indentation) {
                let new_spaces = self.space_count(old_indentation) + shift;
                inserting_text = format!(
                    "{}{}",
                    self.generate_indent(new_spaces),
                    trim_start(&moving_text)
                );
            }
        }
        // The moved line's edit goes first, so it runs after the block's
        // indentation change at the same position.
        builder.add_edit(insert_at, &format!("{inserting_text}\n"));
        match self.match_enter_rule_moving_down(lang, model, start_row, moving, &inserting_text) {
            // The moved line's onEnter rules adjust the block.
            Some(0) => {}
            Some(offset) => self.indent_edits_of_moving_block(model, builder, s, offset),
            None => {
                // No onEnter rule matches; try the indentation rules.
                let (first, last) = (start_row + 1, s.end().row + 1);
                let map = |line: u32| {
                    if line == first {
                        moving + 1
                    } else if line > first && line <= last + 1 {
                        line - 1
                    } else {
                        line
                    }
                };
                let vm = VirtualModel::new(model, lang)
                    .with_line_map(&map)
                    .with_line_content(first, inserting_text.clone());
                let good = rules_good_indent_for_line(self.config, &vm, first + 1);
                let old = model.line_content(start_row);
                if let Some(offset) = self.indent_shift(good, strings::leading_whitespace(&old)) {
                    self.indent_edits_of_moving_block(model, builder, s, offset);
                }
            }
        }
    }

    fn move_up(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder, s: Selection) {
        let moving = s.start().row - 1;
        let moving_text = model.line_content(moving);
        // Delete the line that moves, and insert it after the block.
        builder.add_edit(Position::new(moving, 0)..Position::new(moving + 1, 0), "");
        let end_row = s.end().row;
        let end = Position::new(end_row, model.line_max_column(end_row));
        builder.add_edit(end..end, &format!("\n{moving_text}"));
        let Some(lang) = self.should_auto_indent() else {
            return;
        };
        let start_row = s.start().row;
        match self.match_enter_rule(lang, model, start_row, start_row as i64 - 2) {
            // The onEnter rules of the row above the line that moves adjust
            // the block.
            Some(0) => {}
            Some(offset) => self.indent_edits_of_moving_block(model, builder, s, offset),
            None => {
                // No onEnter rule matches; try the indentation rules.
                let map = |line: u32| {
                    if line == moving + 1 {
                        start_row + 1
                    } else {
                        line
                    }
                };
                let vm = VirtualModel::new(model, lang).with_line_map(&map);
                let good = rules_good_indent_for_line(self.config, &vm, moving + 1);
                let first = model.line_content(start_row);
                if let Some(offset) = self.indent_shift(good, strings::leading_whitespace(&first)) {
                    self.indent_edits_of_moving_block(model, builder, s, offset);
                }
            }
        }
    }
}

impl Command for MoveLinesCommand<'_> {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let last = last_row(model);
        if (self.down && self.selection.end().row == last)
            || (!self.down && self.selection.start().row == 0)
        {
            self.tracked = Some(builder.track_selection(self.selection, None));
            return;
        }
        let mut s = self.selection;
        if s.start().row < s.end().row && s.end().column == 0 {
            self.move_end_position_down = true;
            let row = s.end().row - 1;
            s = set_end_position(s, Position::new(row, model.line_max_column(row)));
        }
        if s.start().row == s.end().row && model.line_max_column(s.start().row) == 0 {
            // The current line is empty.
            let row = s.start().row;
            let other = if self.down { row + 1 } else { row - 1 };
            if model.line_max_column(other) == 0 {
                // The other line is empty too; nothing to edit.
                builder.add_noop(Position::zero());
            } else {
                builder.add_edit(
                    Position::new(row, 0)..Position::new(row, 0),
                    &model.line_content(other),
                );
                builder.add_edit(
                    Position::new(other, 0)..Position::new(other, model.line_max_column(other)),
                    "",
                );
            }
            s = Selection::collapsed(Position::new(other, 0));
        } else if self.down {
            self.move_down(model, builder, s);
        } else {
            self.move_up(model, builder, s);
        }
        self.tracked = Some(builder.track_selection(s, None));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let mut result =
            helper.tracked_selection(self.tracked.expect("tracked in edit_operations"));
        if self.move_end_position_down {
            result = set_end_position(result, Position::new(result.end().row + 1, 0));
        }
        if self.move_end_line_selection_shrink && result.start().row < result.end().row {
            result = set_end_position(result, Position::new(result.end().row, 1));
        }
        result
    }
}

fn move_lines(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &CursorCollection,
    down: bool,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let lang = lang.map(LanguageSupport::new);
    let selections = cursors.selections();
    let commands: Vec<Option<Box<dyn Command + '_>>> = selections
        .iter()
        .map(|&selection| {
            Some(Box::new(MoveLinesCommand {
                selection,
                down,
                config,
                lang: lang.as_ref(),
                tracked: None,
                move_end_position_down: false,
                move_end_line_selection_shrink: false,
            }) as Box<dyn Command + '_>)
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}

/// Moves the selected lines up one line (Monaco
/// `editor.action.moveLinesUpAction`, Alt+Up), without a language (see
/// [`move_lines_up_with_language`]).
pub fn move_lines_up(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    move_lines(config, model, cursors, false, None)
}

/// Moves the selected lines down one line (Monaco
/// `editor.action.moveLinesDownAction`, Alt+Down), without a language (see
/// [`move_lines_down_with_language`]).
pub fn move_lines_down(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    move_lines(config, model, cursors, true, None)
}

/// [`move_lines_up`] that, with [`CursorConfig::auto_indent`] `Full`,
/// reindents the moved block with the language's onEnter and indentation
/// rules (Monaco `MoveLinesCommand`). With `lang == None` this is exactly
/// [`move_lines_up`].
pub fn move_lines_up_with_language(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    move_lines(config, model, cursors, false, lang)
}

/// [`move_lines_down`] that, with [`CursorConfig::auto_indent`] `Full`,
/// reindents the moved block and the line moved over it with the
/// language's onEnter and indentation rules (Monaco `MoveLinesCommand`).
/// With `lang == None` this is exactly [`move_lines_down`].
pub fn move_lines_down_with_language(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    move_lines(config, model, cursors, true, lang)
}

// ---------------------------------------------------------------------------
// Copy lines

struct CopyLinesCommand {
    selection: Selection,
    down: bool,
    noop: bool,
    tracked: Option<TrackedId>,
    start_row_delta: u32,
    end_row_delta: u32,
}

impl Command for CopyLinesCommand {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let mut s = self.selection;
        self.start_row_delta = 0;
        self.end_row_delta = 0;
        if s.start().row < s.end().row && s.end().column == 0 {
            self.end_row_delta = 1;
            let row = s.end().row - 1;
            s = set_end_position(s, Position::new(row, model.line_max_column(row)));
        }
        let source_text = (s.start().row..=s.end().row)
            .map(|row| model.line_content(row))
            .collect::<Vec<_>>()
            .join("\n");
        if source_text.is_empty() && self.down {
            // Duplicating an empty line.
            self.start_row_delta += 1;
            self.end_row_delta += 1;
        }
        let end_row = s.end().row;
        let end_of_selection_line = Position::new(end_row, model.line_max_column(end_row));
        if self.noop {
            builder.add_noop(end_of_selection_line);
        } else if !self.down {
            builder.add_edit(
                end_of_selection_line..end_of_selection_line,
                &format!("\n{source_text}"),
            );
        } else {
            let start = Position::new(s.start().row, 0);
            builder.add_edit(start..start, &format!("{source_text}\n"));
        }
        self.tracked = Some(builder.track_selection(s, None));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let result = helper.tracked_selection(self.tracked.expect("tracked in edit_operations"));
        if self.start_row_delta == 0 && self.end_row_delta == 0 {
            return result;
        }
        let mut start = result.start();
        let mut end = result.end();
        if self.start_row_delta != 0 {
            start = Position::new(start.row + self.start_row_delta, 0);
        }
        if self.end_row_delta != 0 {
            end = Position::new(end.row + self.end_row_delta, 0);
        }
        Selection::from_range(start..end, self.selection.is_reversed())
    }
}

/// Monaco `new CopyLinesCommand(selection, down)`, for other commands that
/// duplicate lines.
pub(crate) fn copy_lines_command(selection: Selection, down: bool) -> Box<dyn Command> {
    Box::new(CopyLinesCommand {
        selection,
        down,
        noop: false,
        tracked: None,
        start_row_delta: 0,
        end_row_delta: 0,
    })
}

fn copy_lines(model: &dyn CursorModel, cursors: &CursorCollection, down: bool) -> EditOperation {
    let selections = cursors.selections();
    let mut sorted: Vec<(usize, Selection, bool)> = selections
        .iter()
        .enumerate()
        .map(|(i, s)| (i, *s, false))
        .collect();
    sorted.sort_by_key(|(_, s, _)| (s.start(), s.end()));
    // Ignore selections that would copy the same line again.
    let mut prev = 0;
    for i in 1..sorted.len() {
        if sorted[prev].1.end().row == sorted[i].1.start().row {
            if sorted[prev].0 < sorted[i].0 {
                sorted[i].2 = true;
            } else {
                sorted[prev].2 = true;
                prev = i;
            }
        }
    }
    let ordered: Vec<Selection> = sorted.iter().map(|(_, s, _)| *s).collect();
    let commands: Commands = sorted
        .into_iter()
        .map(|(_, selection, noop)| {
            Some(Box::new(CopyLinesCommand {
                selection,
                down,
                noop,
                tracked: None,
                start_row_delta: 0,
                end_row_delta: 0,
            }) as Box<dyn Command>)
        })
        .collect();
    execute_commands(
        model,
        &ordered,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}

/// Duplicates the selected lines above them, keeping the selection on the
/// upper copy (Monaco `editor.action.copyLinesUpAction`, Shift+Alt+Up).
pub fn copy_lines_up(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    copy_lines(model, cursors, false)
}

/// Duplicates the selected lines below them, moving the selection to the
/// lower copy (Monaco `editor.action.copyLinesDownAction`, Shift+Alt+Down).
pub fn copy_lines_down(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    copy_lines(model, cursors, true)
}

// ---------------------------------------------------------------------------
// Delete lines

/// Deletes the lines touched by every selection (Monaco
/// `editor.action.deleteLines`, Cmd+Shift+K).
pub fn delete_lines(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    if model.line_count() == 1 && model.line_max_column(0) == 0 {
        // The model is empty.
        return EditOperation::none(selections, EditOperationType::Other);
    }
    struct Op {
        start_row: u32,
        end_row: u32,
        position_column: u32,
    }
    let mut ops: Vec<Op> = selections
        .iter()
        .map(|s| {
            let mut end_row = s.end().row;
            if s.start().row < s.end().row && s.end().column == 0 {
                end_row -= 1;
            }
            Op {
                start_row: s.start().row,
                end_row,
                position_column: s.head.column,
            }
        })
        .collect();
    ops.sort_by_key(|op| (op.start_row, op.end_row));
    // Merge adjacent or overlapping operations.
    let mut merged: Vec<Op> = Vec::new();
    for op in ops {
        match merged.last_mut() {
            Some(prev) if prev.end_row + 1 >= op.start_row => prev.end_row = op.end_row,
            _ => merged.push(op),
        }
    }

    let last = last_row(model);
    let mut lines_deleted = 0;
    let mut edits = Vec::new();
    let mut cursor_state = Vec::new();
    for op in &merged {
        let mut start_row = op.start_row;
        let mut end_row = op.end_row;
        let mut start_column = 0;
        let mut end_column = model.line_max_column(end_row);
        if end_row < last {
            end_row += 1;
            end_column = 0;
        } else if start_row > 0 {
            start_row -= 1;
            start_column = model.line_max_column(start_row);
        }
        edits.push((
            Position::new(start_row, start_column)..Position::new(end_row, end_column),
            String::new(),
        ));
        cursor_state.push(Selection::collapsed(Position::new(
            start_row - lines_deleted,
            op.position_column,
        )));
        lines_deleted += op.end_row - op.start_row + 1;
    }
    edits.retain(|(r, _)| r.start != r.end);
    EditOperation {
        edits,
        selections_after: cursor_state,
        kind: EditOperationType::Other,
        push_stack_element_before: true,
        push_stack_element_after: true,
        auto_closed: Vec::new(),
        follow_up: None,
        auto_whitespace: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Insert line

/// Inserts an empty line above each cursor's line and moves the cursor
/// there, keeping indentation (Monaco `editor.action.insertLineBefore`,
/// Cmd+Shift+Enter).
pub fn insert_line_above(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|s| {
            let row = s.head.row;
            if row == 0 {
                Some(Box::new(ReplaceCommandWithoutChangingPosition {
                    range: Position::zero()..Position::zero(),
                    text: "\n".to_string(),
                }) as Box<dyn Command>)
            } else {
                let at = Position::new(row - 1, model.line_max_column(row - 1));
                enter_command(config, model, false, at..at)
            }
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        false,
    )
}

/// Inserts an empty line below each cursor's line and moves the cursor
/// there, keeping indentation (Monaco `editor.action.insertLineAfter`,
/// Cmd+Enter).
pub fn insert_line_below(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|s| {
            let row = s.head.row;
            let at = Position::new(row, model.line_max_column(row));
            enter_command(config, model, false, at..at)
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        false,
    )
}
