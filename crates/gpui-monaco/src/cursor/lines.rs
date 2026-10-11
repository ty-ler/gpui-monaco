//! Whole-line editing commands.
//!
//! Ports Monaco's `src/vs/editor/contrib/linesOperations/browser/linesOperations.ts`
//! (indent/outdent lines, copy/move/delete lines, insert line above/below),
//! `copyLinesCommand.ts`, `moveLinesCommand.ts` (without its
//! language-dependent auto-indentation) and `EnterOperation.lineInsertBefore`
//! / `lineInsertAfter` from
//! `src/vs/editor/common/cursor/cursorTypeEditOperations.ts` (MIT).

use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType,
    ReplaceCommandWithoutChangingPosition, TrackedId, execute_commands,
};
use super::typing::{enter_command, shift};
use super::{CursorCollection, CursorConfig, CursorModel, Position, Selection, last_row};

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

struct MoveLinesCommand {
    selection: Selection,
    down: bool,
    tracked: Option<TrackedId>,
    move_end_position_down: bool,
}

impl Command for MoveLinesCommand {
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
            let moving = s.end().row + 1;
            let moving_text = model.line_content(moving);
            builder.add_edit(
                Position::new(moving - 1, model.line_max_column(moving - 1))
                    ..Position::new(moving, model.line_max_column(moving)),
                "",
            );
            builder.add_edit(
                Position::new(s.start().row, 0)..Position::new(s.start().row, 0),
                &format!("{moving_text}\n"),
            );
        } else {
            let moving = s.start().row - 1;
            let moving_text = model.line_content(moving);
            builder.add_edit(Position::new(moving, 0)..Position::new(moving + 1, 0), "");
            let end_row = s.end().row;
            let end = Position::new(end_row, model.line_max_column(end_row));
            builder.add_edit(end..end, &format!("\n{moving_text}"));
        }
        self.tracked = Some(builder.track_selection(s, None));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let mut result =
            helper.tracked_selection(self.tracked.expect("tracked in edit_operations"));
        if self.move_end_position_down {
            result = set_end_position(result, Position::new(result.end().row + 1, 0));
        }
        result
    }
}

fn move_lines(model: &dyn CursorModel, cursors: &CursorCollection, down: bool) -> EditOperation {
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|&selection| {
            Some(Box::new(MoveLinesCommand {
                selection,
                down,
                tracked: None,
                move_end_position_down: false,
            }) as Box<dyn Command>)
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
/// `editor.action.moveLinesUpAction`, Alt+Up).
pub fn move_lines_up(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    move_lines(model, cursors, false)
}

/// Moves the selected lines down one line (Monaco
/// `editor.action.moveLinesDownAction`, Alt+Down).
pub fn move_lines_down(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    move_lines(model, cursors, true)
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
