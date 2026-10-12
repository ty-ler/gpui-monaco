//! Typing, deleting, indenting and clipboard operations.
//!
//! Ports the language-agnostic parts of Monaco's
//! `src/vs/editor/common/cursor/cursorTypeOperations.ts`,
//! `src/vs/editor/common/cursor/cursorTypeEditOperations.ts`,
//! `src/vs/editor/common/cursor/cursorDeleteOperations.ts`,
//! `src/vs/editor/common/commands/shiftCommand.ts`, the word deletion
//! commands of `src/vs/editor/contrib/wordOperations/browser/wordOperations.ts`
//! and `src/vs/editor/contrib/wordPartOperations/browser/wordPartOperations.ts`,
//! `DeleteAllLeftAction` from
//! `src/vs/editor/contrib/linesOperations/browser/linesOperations.ts` and
//! `ViewModel.getPlainTextToCopy` from
//! `src/vs/editor/common/viewModel/viewModelImpl.ts` (MIT).
//!
//! The language-aware parts (auto-closing pairs and their overtyping and
//! deletion, surround-selection, electric characters, onEnter rules and
//! indentation rules) are in `language_typing.rs`, which falls back to the
//! functions here. [`ShiftCommand`] takes the language when there is one, for
//! its onEnter-based alignment of continuation lines.

use std::ops::Range;

use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType, ReplaceCommand,
    ReplaceCommandThatPreservesSelection, ReplaceOvertypeCommand, TrackedId, auto_whitespace,
    execute_commands, should_push_stack_element_between, typing_operation,
};
use super::language::{LanguageSupport, enter_action};
use super::movement;
use super::word::{self, DeleteWordContext, WordClassifier, WordNavigationType};
use super::{
    AutoIndent, CursorCollection, CursorConfig, CursorModel, MultiCursorPaste, Position, Selection,
    line_units, strings, value_in_range,
};

type Commands<'a> = Vec<Option<Box<dyn Command + 'a>>>;

/// Types `text` at every cursor, replacing selections (Monaco
/// `TypeOperations.typeWithoutInterceptors`, which for a single character is
/// what `typeWithInterceptors` falls back to without language features).
/// Typing a lone line break behaves like [`line_break`].
///
/// `prev` is the `kind` of the previous edit operation (`None` at start);
/// it decides whether an undo stop goes before this one, so consecutive
/// typing groups into one undo step and a run of spaces after a word starts
/// a new one (Monaco `getTypingOperation`/`shouldPushStackElementBetween`).
/// Monaco types keyboard input one character at a time; callers that want
/// identical undo grouping for multi-character keyboard input should do the
/// same.
pub fn type_text(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    text: &str,
    prev: Option<EditOperationType>,
) -> EditOperation {
    if matches!(text, "\n" | "\r\n" | "\r") {
        return line_break(config, model, cursors);
    }
    type_simple(model, cursors, text, prev, config.overtype)
}

/// Monaco `SimpleCharacterTypeOperation`: replaces every selection by
/// `text`, typing over the characters after it when `overtype`.
pub(crate) fn type_simple(
    model: &dyn CursorModel,
    cursors: &CursorCollection,
    text: &str,
    prev: Option<EditOperationType>,
    overtype: bool,
) -> EditOperation {
    let prev = prev.unwrap_or(EditOperationType::Other);
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|s| {
            if overtype {
                ReplaceOvertypeCommand::boxed(s.range(), text)
            } else {
                ReplaceCommand::boxed(s.range(), text)
            }
        })
        .collect();
    let kind = typing_operation(text, prev);
    execute_commands(
        model,
        &selections,
        commands,
        kind,
        should_push_stack_element_between(prev, kind),
        false,
    )
}

/// An IME composition update (Monaco `TypeOperations.compositionType`):
/// replaces `replace_prev_char_cnt` UTF-16 units before and
/// `replace_next_char_cnt` after each (collapsed) cursor with `text`, then
/// moves the cursor by `position_delta` from the end of the inserted text.
/// Cursors with a selection are left alone.
#[allow(clippy::too_many_arguments)]
pub fn composition_type(
    model: &impl CursorModel,
    cursors: &CursorCollection,
    text: &str,
    replace_prev_char_cnt: u32,
    replace_next_char_cnt: u32,
    position_delta: i32,
    prev: Option<EditOperationType>,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let prev = prev.unwrap_or(EditOperationType::Other);
    let selections = cursors.selections();
    if text.is_empty() && replace_prev_char_cnt == 0 && replace_next_char_cnt == 0 {
        // A no-op edit that may still want to move the cursor.
        let moved = selections
            .iter()
            .map(|s| {
                let column = (s.head.column as i64 + position_delta as i64).max(0) as u32;
                Selection::collapsed(Position::new(s.head.row, column))
            })
            .collect();
        return EditOperation::none(moved, prev);
    }
    let commands: Commands = selections
        .iter()
        .map(|s| {
            if !s.is_empty() {
                // A cursor operation happened before a canceled composition.
                return None;
            }
            let pos = s.head;
            let start = pos.column.saturating_sub(replace_prev_char_cnt);
            let end = (pos.column + replace_next_char_cnt).min(model.line_max_column(pos.row));
            Some(Box::new(super::edit::ReplaceCommandWithOffsetCursorState {
                range: Position::new(pos.row, start)..Position::new(pos.row, end),
                text: text.to_string(),
                line_delta: 0,
                column_delta: position_delta,
            }) as Box<dyn Command>)
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::TypingOther,
        should_push_stack_element_between(prev, EditOperationType::TypingOther),
        false,
    )
}

/// Monaco `EnterOperation._enter` with `autoIndent: 'keep'`: a line break
/// followed by the current line's leading whitespace (up to the cursor),
/// normalized to the indentation settings.
pub(crate) fn enter_text(
    config: &CursorConfig,
    model: &dyn CursorModel,
    range: &Range<Position>,
) -> String {
    let line = model.line_content(range.start.row);
    let indentation = strings::utf16_prefix(strings::leading_whitespace(&line), range.start.column);
    format!("\n{}", config.normalize_indentation(indentation))
}

/// Inserts a line break at every cursor, keeping the current line's
/// indentation (Monaco's Enter without language rules).
pub fn line_break(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|s| {
            auto_whitespace(ReplaceCommand::boxed(
                s.range(),
                enter_text(config, model, &s.range()),
            ))
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::TypingOther,
        true,
        false,
    )
}

/// Monaco `EnterOperation._enter` with `keepPosition`, used by
/// `lineInsertBefore`/`lineInsertAfter`/`lineBreakInsert`.
pub(crate) fn enter_command(
    config: &CursorConfig,
    model: &dyn CursorModel,
    keep_position: bool,
    range: Range<Position>,
) -> Option<Box<dyn Command>> {
    let text = enter_text(config, model, &range);
    auto_whitespace(if keep_position {
        Some(Box::new(
            super::edit::ReplaceCommandWithoutChangingPosition { range, text },
        ))
    } else {
        ReplaceCommand::boxed(range, text)
    })
}

// ---------------------------------------------------------------------------
// Tab / outdent

/// Monaco `TabOperation._replaceJumpToNextIndent`.
fn replace_jump_to_next_indent(
    config: &CursorConfig,
    model: &dyn CursorModel,
    selection: Selection,
) -> Option<Box<dyn Command>> {
    let text = if config.insert_spaces {
        let visible_column = config.visible_column_from_column(model, selection.start());
        let spaces = config.indent_size - visible_column % config.indent_size;
        " ".repeat(spaces as usize)
    } else {
        "\t".to_string()
    };
    ReplaceCommand::boxed(selection.range(), text)
}

/// The Tab key (Monaco `TabOperation.getCommands` without language rules):
/// on a whitespace-only line, indents the line to one indent; otherwise
/// inserts whitespace to the next indent stop, or indents the selected
/// lines when the selection spans lines (or a whole line).
pub fn tab(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    // Without language rules there is no "good indent" for a line.
    tab_with_good_indent(config, model, cursors, None, &|_| None)
}

/// Monaco `TabOperation.getCommands`; `good_indent` is
/// `TabOperation._goodIndentForLine` for a row, and `lang` aligns the lines
/// a multi-line selection indents (see [`ShiftCommand`]).
pub(crate) fn tab_with_good_indent(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &CursorCollection,
    lang: Option<&LanguageSupport>,
    good_indent: &dyn Fn(u32) -> Option<String>,
) -> EditOperation {
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|&selection| {
            let start = selection.start();
            let end = selection.end();
            if selection.is_empty() {
                let line = model.line_content(start.row);
                if line.chars().all(char::is_whitespace) {
                    let good_indent = good_indent(start.row).unwrap_or_else(|| "\t".to_string());
                    let possible_type_text = config.normalize_indentation(&good_indent);
                    if !line.starts_with(&possible_type_text) {
                        let len = model.line_max_column(start.row);
                        return auto_whitespace(ReplaceCommand::boxed(
                            Position::new(start.row, 0)..Position::new(start.row, len),
                            possible_type_text,
                        ));
                    }
                }
                return auto_whitespace(replace_jump_to_next_indent(config, model, selection));
            }
            if start.row == end.row {
                let max_column = model.line_max_column(start.row);
                if start.column != 0 || end.column != max_column {
                    // A single line selection that is not the entire line.
                    return replace_jump_to_next_indent(config, model, selection);
                }
            }
            Some(Box::new(ShiftCommand::new(selection, config, false, lang)) as Box<dyn Command>)
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

/// Shift+Tab: outdents the lines of every selection (Monaco
/// `TypeOperations.outdent`).
pub fn outdent(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    shift(config, model, cursors, true, None)
}

/// Indents (`unshift == false`) or outdents the lines of every selection,
/// keeping continuation lines aligned by `lang`'s onEnter rules (see
/// [`ShiftCommand`]).
pub(crate) fn shift(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &CursorCollection,
    unshift: bool,
    lang: Option<&LanguageSupport>,
) -> EditOperation {
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|&s| Some(Box::new(ShiftCommand::new(s, config, unshift, lang)) as Box<dyn Command>))
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

/// Monaco `ShiftCommand`. With `use_tab_stops`, a line whose content starts
/// off an indent stop is checked against the onEnter rules of the line
/// above: the leading spaces of the rule's `appendText` (less its
/// `removeText`) are alignment, not indentation, so ` * ` continuation lines
/// of a doc comment keep their space when indented or outdented. That needs
/// the language (`lang`); without one, every line is shifted as it is.
pub(crate) struct ShiftCommand<'a> {
    selection: Selection,
    unshift: bool,
    tab_size: u32,
    indent_size: u32,
    insert_spaces: bool,
    use_tab_stops: bool,
    auto_indent: AutoIndent,
    lang: Option<&'a LanguageSupport<'a>>,
    tracked: Option<TrackedId>,
    use_last_edit_range_for_cursor_end_position: bool,
    selection_start_column_stays_put: bool,
}

impl<'a> ShiftCommand<'a> {
    pub(crate) fn new(
        selection: Selection,
        config: &CursorConfig,
        unshift: bool,
        lang: Option<&'a LanguageSupport<'a>>,
    ) -> Self {
        Self {
            selection,
            unshift,
            tab_size: config.tab_size,
            indent_size: config.indent_size,
            insert_spaces: config.insert_spaces,
            use_tab_stops: config.use_tab_stops,
            auto_indent: config.auto_indent,
            lang,
            tracked: None,
            use_last_edit_range_for_cursor_end_position: false,
            selection_start_column_stays_put: false,
        }
    }

    /// The spaces at the end of row `row`'s indentation that are alignment
    /// for the onEnter rule of the row above (Monaco's `extraSpaces`), given
    /// the row above's own (`previous_line_extra_spaces`).
    fn alignment_spaces(
        &self,
        model: &dyn CursorModel,
        row: u32,
        previous_line_extra_spaces: u32,
    ) -> Option<u32> {
        // Monaco also skips this while the line above isn't cheap to
        // tokenize; token types are always at hand here.
        let lang = self.lang?;
        let end = Position::new(row - 1, model.line_max_column(row - 1));
        let enter = enter_action(self.auto_indent, lang, model, &(end..end))?;
        let mut extra_spaces = previous_line_extra_spaces;
        for c in enter.append_text.chars() {
            if extra_spaces >= self.indent_size || c != ' ' {
                break;
            }
            extra_spaces += 1;
        }
        Some(extra_spaces.saturating_sub(enter.remove_text))
    }

    /// Monaco `ShiftCommand.shiftIndent`/`unshiftIndent`: the indentation
    /// one stop right/left of the content starting at `column`.
    pub(crate) fn shift_indent(
        line: &str,
        column: u32,
        tab_size: u32,
        indent_size: u32,
        insert_spaces: bool,
        unshift: bool,
    ) -> String {
        let content_start_visible_column =
            strings::visible_column_from_column(line, column, tab_size);
        if insert_spaces {
            let desired = if unshift {
                strings::prev_indent_tab_stop(content_start_visible_column, indent_size)
            } else {
                strings::next_indent_tab_stop(content_start_visible_column, indent_size)
            };
            " ".repeat((desired / indent_size * indent_size) as usize)
        } else {
            let desired = if unshift {
                strings::prev_render_tab_stop(content_start_visible_column, tab_size)
            } else {
                strings::next_render_tab_stop(content_start_visible_column, tab_size)
            };
            "\t".repeat((desired / tab_size) as usize)
        }
    }
}

impl Command for ShiftCommand<'_> {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let start_line = self.selection.start().row;
        let mut end_line = self.selection.end().row;
        if self.selection.end().column == 0 && start_line != end_line {
            end_line -= 1;
        }
        let should_indent_empty_lines = start_line == end_line;

        if self.use_tab_stops {
            // If indenting or outdenting on a whitespace only line.
            if self.selection.is_empty()
                && model
                    .line_content(start_line)
                    .chars()
                    .all(char::is_whitespace)
            {
                self.use_last_edit_range_for_cursor_end_position = true;
            }
            // Keep track of the previous line's misalignment.
            let mut extra_spaces = 0;
            for row in start_line..=end_line {
                let previous_line_extra_spaces = extra_spaces;
                extra_spaces = 0;
                let line = model.line_content(row);
                let units = strings::utf16(&line);
                let first_non_ws = strings::first_non_whitespace_index(&units);
                if self.unshift && (units.is_empty() || first_non_ws == Some(0)) {
                    // Empty line or no leading whitespace: nothing to do.
                    continue;
                }
                if !should_indent_empty_lines && !self.unshift && units.is_empty() {
                    // Do not indent empty lines.
                    continue;
                }
                let mut indentation_end_index = first_non_ws.unwrap_or(units.len()) as u32;
                let content_start_visible_column = strings::visible_column_from_column(
                    &line,
                    indentation_end_index,
                    self.tab_size,
                );
                if row > 0
                    && !content_start_visible_column.is_multiple_of(self.indent_size)
                    && let Some(extra) =
                        self.alignment_spaces(model, row, previous_line_extra_spaces)
                {
                    // The line is misaligned, as expected after a rule
                    // whose `appendText` starts with spaces (` * ` in doc
                    // comments): act as if those spaces weren't indentation.
                    extra_spaces = extra;
                    for _ in 0..extra_spaces {
                        if indentation_end_index == 0
                            || units[indentation_end_index as usize - 1] != b' ' as u16
                        {
                            break;
                        }
                        indentation_end_index -= 1;
                    }
                }
                if self.unshift && indentation_end_index == 0 {
                    continue;
                }
                let desired = Self::shift_indent(
                    &line,
                    indentation_end_index,
                    self.tab_size,
                    self.indent_size,
                    self.insert_spaces,
                    self.unshift,
                );
                builder.add_edit(
                    Position::new(row, 0)..Position::new(row, indentation_end_index),
                    &desired,
                );
                if row == start_line && !self.selection.is_empty() {
                    // Force the start column to stay put because we're
                    // inserting after it.
                    self.selection_start_column_stays_put =
                        self.selection.start().column <= indentation_end_index;
                }
            }
        } else {
            if !self.unshift && self.selection.is_empty() && model.line_max_column(start_line) == 0
            {
                self.use_last_edit_range_for_cursor_end_position = true;
            }
            let one_indent = if self.insert_spaces {
                " ".repeat(self.indent_size as usize)
            } else {
                "\t".to_string()
            };
            for row in start_line..=end_line {
                let units = line_units(model, row);
                let first_non_ws = strings::first_non_whitespace_index(&units);
                if self.unshift && (units.is_empty() || first_non_ws == Some(0)) {
                    continue;
                }
                if !should_indent_empty_lines && !self.unshift && units.is_empty() {
                    continue;
                }
                let mut indentation_end_index = first_non_ws.unwrap_or(units.len());
                if self.unshift && indentation_end_index == 0 {
                    continue;
                }
                if self.unshift {
                    indentation_end_index = indentation_end_index.min(self.indent_size as usize);
                    if let Some(i) = units[..indentation_end_index]
                        .iter()
                        .position(|&c| c == b'\t' as u16)
                    {
                        indentation_end_index = i + 1;
                    }
                    builder.add_edit(
                        Position::new(row, 0)..Position::new(row, indentation_end_index as u32),
                        "",
                    );
                } else {
                    builder.add_edit(Position::new(row, 0)..Position::new(row, 0), &one_indent);
                    if row == start_line && !self.selection.is_empty() {
                        self.selection_start_column_stays_put = self.selection.start().column == 0;
                    }
                }
            }
        }
        self.tracked = Some(builder.track_selection(self.selection, None));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        if self.use_last_edit_range_for_cursor_end_position {
            return Selection::collapsed(helper.inverse_edit_ranges()[0].end);
        }
        let result = helper.tracked_selection(self.tracked.expect("tracked in edit_operations"));
        if self.selection_start_column_stays_put {
            // The selection start should not move.
            let initial_start_column = self.selection.start().column;
            if result.start().column <= initial_start_column {
                return result;
            }
            let start = Position::new(result.start().row, initial_start_column);
            return if result.is_reversed() {
                Selection::new(result.end(), start)
            } else {
                Selection::new(start, result.end())
            };
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Deleting

/// Monaco `DeleteOperations.getDeleteLeftRange`.
fn delete_left_range(
    config: &CursorConfig,
    model: &dyn CursorModel,
    selection: Selection,
) -> Range<Position> {
    if !selection.is_empty() {
        return selection.range();
    }
    let position = selection.head;
    // Unindent when using tab stops and the cursor is within indentation.
    if config.use_tab_stops && position.column > 0 {
        let units = line_units(model, position.row);
        let last_indentation_column =
            strings::first_non_whitespace_index(&units).unwrap_or(units.len()) as u32;
        if position.column <= last_indentation_column {
            let from_visible_column = config.visible_column_from_column(model, position);
            let to_visible_column =
                strings::prev_indent_tab_stop(from_visible_column, config.indent_size);
            let to_column =
                config.column_from_visible_column(model, position.row, to_visible_column as i64);
            return Position::new(position.row, to_column)..position;
        }
    }
    position_after_delete_left(model, position)..position
}

fn position_after_delete_left(model: &dyn CursorModel, position: Position) -> Position {
    if position.column > 0 {
        let line = model.line_content(position.row);
        Position::new(
            position.row,
            strings::left_delete_offset(position.column, &line),
        )
    } else if position.row > 0 {
        Position::new(position.row - 1, model.line_max_column(position.row - 1))
    } else {
        position
    }
}

/// Backspace (Monaco `DeleteOperations.deleteLeft` without auto-closing
/// pair deletion): deletes selections, or the grapheme/emoji before each
/// cursor, or back to the previous indent stop inside indentation (when
/// `use_tab_stops`).
pub fn delete_left(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    prev: Option<EditOperationType>,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let mut push_before = prev != Some(EditOperationType::DeletingLeft);
    let commands: Commands = selections
        .iter()
        .map(|&s| {
            let range = delete_left_range(config, model, s);
            if range.start == range.end {
                // At the beginning of the file.
                return None;
            }
            if range.start.row != range.end.row {
                push_before = true;
            }
            ReplaceCommand::boxed(range, "")
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::DeletingLeft,
        push_before,
        false,
    )
}

/// Monaco `DeleteOperations.getDeleteRightRange`.
fn delete_right_range(
    config: &CursorConfig,
    model: &dyn CursorModel,
    selection: Selection,
) -> Range<Position> {
    if !selection.is_empty() {
        return selection.range();
    }
    let position = selection.head;
    let right_of_position = movement::right(config, model, position);
    if config.trim_whitespace_on_delete && right_of_position.row != position.row {
        // Smart line join: also delete the next line's leading whitespace.
        let current_line_has_content =
            super::first_non_whitespace_column(model, position.row).is_some();
        if current_line_has_content
            && let Some(first_non_ws) =
                super::first_non_whitespace_column(model, right_of_position.row)
        {
            return position..Position::new(right_of_position.row, first_non_ws);
        }
    }
    position.min(right_of_position)..position.max(right_of_position)
}

/// Delete (Monaco `DeleteOperations.deleteRight`): deletes selections or
/// the grapheme after each cursor.
pub fn delete_right(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    prev: Option<EditOperationType>,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let mut push_before = prev != Some(EditOperationType::DeletingRight);
    let commands: Commands = selections
        .iter()
        .map(|&s| {
            let range = delete_right_range(config, model, s);
            if range.start == range.end {
                // Probably at the end of the file.
                return None;
            }
            if range.start.row != range.end.row {
                push_before = true;
            }
            ReplaceCommand::boxed(range, "")
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::DeletingRight,
        push_before,
        false,
    )
}

fn delete_ranges(
    model: &dyn CursorModel,
    selections: &[Selection],
    ranges: Vec<Range<Position>>,
) -> EditOperation {
    let commands: Commands = ranges
        .into_iter()
        .map(|r| ReplaceCommand::boxed(r, ""))
        .collect();
    execute_commands(
        model,
        selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}

/// Deletes words to the left or right of every cursor (Monaco's
/// `DeleteWordCommand` family: `deleteWordLeft` is `left` + `WordStart` +
/// whitespace heuristics, `deleteWordStartLeft`/`deleteWordEndLeft` turn the
/// heuristics off, and likewise to the right).
pub fn delete_word(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    left: bool,
    nav: WordNavigationType,
    whitespace_heuristics: bool,
) -> EditOperation {
    delete_word_with(
        config,
        model,
        cursors,
        left,
        nav,
        whitespace_heuristics,
        &|_| false,
    )
}

/// [`delete_word`], deleting both characters of an auto-closing pair
/// around a cursor for which `auto_closing_pair_delete` holds.
pub(crate) fn delete_word_with(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &CursorCollection,
    left: bool,
    nav: WordNavigationType,
    whitespace_heuristics: bool,
    auto_closing_pair_delete: &dyn Fn(Selection) -> bool,
) -> EditOperation {
    let classifier = WordClassifier::for_config(config);
    let selections = cursors.selections();
    let ranges = selections
        .iter()
        .map(|&selection| {
            let ctx = DeleteWordContext {
                classifier: &classifier,
                model,
                selection,
                whitespace_heuristics,
                auto_closing_pair_delete: left && auto_closing_pair_delete(selection),
            };
            if left {
                word::delete_word_left_range(&ctx, nav)
                    .unwrap_or(Position::zero()..Position::zero())
            } else {
                word::delete_word_right_range(&ctx, nav).unwrap_or_else(|| {
                    let end = super::max_position(model);
                    end..end
                })
            }
        })
        .collect();
    delete_ranges(model, &selections, ranges)
}

/// Monaco `deleteWordLeft` (Ctrl+Backspace / Alt+Backspace on macOS).
pub fn delete_word_left(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    delete_word(
        config,
        model,
        cursors,
        true,
        WordNavigationType::WordStart,
        true,
    )
}

/// Monaco `deleteWordRight` (Ctrl+Delete / Alt+Delete on macOS).
pub fn delete_word_right(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    delete_word(
        config,
        model,
        cursors,
        false,
        WordNavigationType::WordEnd,
        true,
    )
}

/// Monaco `deleteWordPartLeft`/`deleteWordPartRight` (camelCase and
/// snake_case aware).
pub fn delete_word_part(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    left: bool,
) -> EditOperation {
    delete_word_part_with(config, model, cursors, left, &|_| false)
}

/// [`delete_word_part`], deleting both characters of an auto-closing pair
/// around a cursor for which `auto_closing_pair_delete` holds.
pub(crate) fn delete_word_part_with(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &CursorCollection,
    left: bool,
    auto_closing_pair_delete: &dyn Fn(Selection) -> bool,
) -> EditOperation {
    let classifier = WordClassifier::for_config(config);
    let selections = cursors.selections();
    let ranges = selections
        .iter()
        .map(|&selection| {
            let ctx = DeleteWordContext {
                classifier: &classifier,
                model,
                selection,
                whitespace_heuristics: true,
                auto_closing_pair_delete: left && auto_closing_pair_delete(selection),
            };
            if left {
                word::delete_word_part_left_range(&ctx)
                    .unwrap_or(Position::zero()..Position::zero())
            } else {
                word::delete_word_part_right_range(&ctx).unwrap_or_else(|| {
                    let end = super::max_position(model);
                    end..end
                })
            }
        })
        .collect();
    delete_ranges(model, &selections, ranges)
}

/// Monaco `deleteInsideWord`: deletes the word (and adjacent whitespace,
/// unless `only_word`) under every cursor.
pub fn delete_inside_word(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    only_word: bool,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let classifier = WordClassifier::for_config(config);
    let selections = cursors.selections();
    let ranges = selections
        .iter()
        .map(|&s| word::delete_inside_word_range(&classifier, model, s, only_word))
        .collect();
    delete_ranges(model, &selections, ranges)
}

/// Monaco `deleteAllLeft` (Cmd+Backspace): deletes from each cursor to the
/// start of its line (or the previous line break at column 0).
pub fn delete_all_left(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let primary_cursor = cursors.primary().selection().range();
    let mut selections = cursors.selections();
    selections.sort_by_key(|s| (s.start(), s.end()));
    let mut ranges: Vec<Range<Position>> = selections
        .iter()
        .map(|s| {
            let start = s.start();
            if s.is_empty() {
                if start.column == 0 {
                    let from_row = start.row.saturating_sub(1);
                    let from_column = if start.row == 0 {
                        0
                    } else {
                        model.line_max_column(from_row)
                    };
                    Position::new(from_row, from_column)..Position::new(start.row, 0)
                } else {
                    Position::new(start.row, 0)..start
                }
            } else {
                Position::new(start.row, 0)..s.end()
            }
        })
        .collect();

    // Merge overlapping (or touching) ranges.
    let intersects =
        |a: &Range<Position>, b: &Range<Position>| a.start.max(b.start) <= a.end.min(b.end);
    let mut effective: Vec<Range<Position>> = Vec::new();
    for i in 0..ranges.len() {
        if i + 1 < ranges.len() {
            if intersects(&ranges[i], &ranges[i + 1]) {
                ranges[i + 1] =
                    ranges[i].start.min(ranges[i + 1].start)..ranges[i].end.max(ranges[i + 1].end);
            } else {
                effective.push(ranges[i].clone());
            }
        } else {
            effective.push(ranges[i].clone());
        }
    }

    // Monaco `DeleteAllLeftAction._getEndCursorState`.
    let mut end_primary = None;
    let mut end_cursor_state = Vec::new();
    let mut deleted_lines = 0;
    for range in &effective {
        let end_cursor = if range.end.column == 0 && deleted_lines > 0 {
            Selection::collapsed(Position::new(
                range.start.row - deleted_lines,
                range.start.column,
            ))
        } else {
            Selection::collapsed(range.start)
        };
        deleted_lines += range.end.row - range.start.row;
        if intersects(range, &primary_cursor) {
            end_primary = Some(end_cursor);
        } else {
            end_cursor_state.push(end_cursor);
        }
    }
    if let Some(primary) = end_primary {
        end_cursor_state.insert(0, primary);
    }

    EditOperation {
        edits: effective
            .into_iter()
            .filter(|r| r.start != r.end)
            .map(|r| (r, String::new()))
            .collect(),
        selections_after: end_cursor_state,
        kind: EditOperationType::Other,
        push_stack_element_before: true,
        push_stack_element_after: true,
        auto_closed: Vec::new(),
        follow_up: None,
        auto_whitespace: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Clipboard

/// Cut (Monaco `DeleteOperations.cut`): deletes the selections; empty
/// selections cut their whole line when `empty_selection_clipboard`. Get the
/// text to put on the clipboard with [`copy_text`] first.
pub fn cut(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let mut selections = cursors.selections();
    // Monaco sorts the selections it is given (and so reports the resulting
    // selections in this order).
    selections.sort_by_key(|s| (s.start(), s.end()));
    let line_count = model.line_count();
    let mut last_cut_range: Option<Range<Position>> = None;
    let commands: Commands = selections
        .iter()
        .map(|selection| {
            if !selection.is_empty() {
                return ReplaceCommand::boxed(selection.range(), "");
            }
            if !config.empty_selection_clipboard {
                // Cannot cut an empty selection.
                return None;
            }
            // A full line cut.
            let position = selection.head;
            let row = position.row;
            let range = if row + 1 < line_count {
                // Cutting a line in the middle of the model.
                Position::new(row, 0)..Position::new(row + 1, 0)
            } else if row > 0 && last_cut_range.as_ref().map(|r| r.end.row) != Some(row) {
                // Cutting the last line, there is more than one line, and a
                // previous cut does not touch this one.
                Position::new(row - 1, model.line_max_column(row - 1))
                    ..Position::new(row, model.line_max_column(row))
            } else {
                // Cutting the single line that the model contains.
                Position::new(row, 0)..Position::new(row, model.line_max_column(row))
            };
            last_cut_range = Some(range.clone());
            if range.start == range.end {
                None
            } else {
                ReplaceCommand::boxed(range, "")
            }
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

/// The text to put on the clipboard for copy/cut (Monaco
/// `ViewModel.getPlainTextToCopy` and `getDataToCopy`). Returns the text,
/// whether it came from a single empty selection (a whole-line copy, to be
/// passed back to [`paste`] as `pasted_from_empty_selection`), and the
/// per-cursor pieces when several pieces were copied (empty otherwise; pass
/// them back to [`paste`] as `multicursor_text`).
pub fn copy_text(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> (String, bool, Vec<String>) {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let is_from_empty_selection =
        config.empty_selection_clipboard && selections.len() == 1 && selections[0].is_empty();
    let mut ranges: Vec<Range<Position>> = selections.iter().map(|s| s.range()).collect();
    ranges.sort_by_key(|r| (r.start, r.end));
    let has_empty_range = ranges.iter().any(|r| r.start == r.end);
    let has_non_empty_range = ranges.iter().any(|r| r.start != r.end);
    if !has_non_empty_range && !config.empty_selection_clipboard {
        return (String::new(), is_from_empty_selection, Vec::new());
    }
    let mut result: Vec<String> = Vec::new();
    if has_empty_range && config.empty_selection_clipboard {
        // Some (maybe all) selections are empty: copy their whole lines.
        let mut prev_row = None;
        for range in &ranges {
            let row = range.start.row;
            if range.start == range.end {
                if Some(row) != prev_row {
                    result.push(format!("{}\n", model.line_content(row)));
                }
            } else {
                result.push(value_in_range(model, range.clone()));
            }
            prev_row = Some(row);
        }
    } else {
        for range in &ranges {
            if range.start != range.end {
                result.push(value_in_range(model, range.clone()));
            }
        }
    }
    if result.len() == 1 {
        (
            result.pop().unwrap_or_default(),
            is_from_empty_selection,
            Vec::new(),
        )
    } else {
        (result.join("\n"), is_from_empty_selection, result)
    }
}

/// Monaco `PasteOperation._distributePasteToCursors`.
fn distribute_paste_to_cursors(
    config: &CursorConfig,
    selections: &[Selection],
    text: &str,
    paste_on_new_line: bool,
    multicursor_text: &[String],
) -> Option<Vec<String>> {
    if selections.len() == 1 {
        return None;
    }
    if multicursor_text.len() == selections.len() {
        return Some(multicursor_text.to_vec());
    }
    if paste_on_new_line {
        return None;
    }
    if config.multi_cursor_paste == MultiCursorPaste::Spread {
        // Try to spread the pasted text when the line count matches the
        // cursor count.
        let mut text = text;
        if let Some(t) = text.strip_suffix('\n') {
            text = t;
        }
        if let Some(t) = text.strip_suffix('\r') {
            text = t;
        }
        let lines: Vec<String> = text
            .split("\r\n")
            .flat_map(|l| l.split(['\r', '\n']))
            .map(str::to_string)
            .collect();
        if lines.len() == selections.len() {
            return Some(lines);
        }
    }
    None
}

/// Paste (Monaco `PasteOperation.getEdits`). `pasted_from_empty_selection`
/// and `multicursor_text` come from [`copy_text`] when the clipboard still
/// holds what this editor copied: a whole line copied from an empty
/// selection is inserted above the cursor's line, and per-cursor pieces are
/// distributed over the cursors when the counts match. With
/// [`MultiCursorPaste::Spread`], text whose line count equals the cursor
/// count is spread one line per cursor.
pub fn paste(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    text: &str,
    pasted_from_empty_selection: bool,
    multicursor_text: Option<&[String]>,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let mut paste_on_new_line = config.empty_selection_clipboard && pasted_from_empty_selection;
    let mut selections = cursors.selections();
    let text = text::normalize_line_endings(text).into_owned();
    if let Some(distributed) = distribute_paste_to_cursors(
        config,
        &selections,
        &text,
        paste_on_new_line,
        multicursor_text.unwrap_or(&[]),
    ) {
        selections.sort_by_key(|s| (s.start(), s.end()));
        let commands: Commands = selections
            .iter()
            .zip(distributed)
            .map(|(s, t)| {
                if config.overtype && config.overtype_on_paste {
                    ReplaceOvertypeCommand::boxed(s.range(), t)
                } else {
                    ReplaceCommand::boxed(s.range(), t)
                }
            })
            .collect();
        return execute_commands(
            model,
            &selections,
            commands,
            EditOperationType::Other,
            true,
            true,
        );
    }
    let commands: Commands = selections
        .iter()
        .map(|&selection| {
            let position = selection.head;
            if paste_on_new_line && !selection.is_empty() {
                paste_on_new_line = false;
            }
            if paste_on_new_line && text.find('\n') != Some(text.len().wrapping_sub(1)) {
                paste_on_new_line = false;
            }
            if paste_on_new_line {
                // Paste the entire line at the beginning of the line.
                let at = Position::new(position.row, 0);
                Some(Box::new(ReplaceCommandThatPreservesSelection {
                    range: at..at,
                    text: text.clone(),
                    selection,
                    force_move_markers: true,
                    tracked: None,
                }) as Box<dyn Command>)
            } else if config.overtype && config.overtype_on_paste {
                ReplaceOvertypeCommand::boxed(selection.range(), text.clone())
            } else {
                ReplaceCommand::boxed(selection.range(), text.clone())
            }
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
