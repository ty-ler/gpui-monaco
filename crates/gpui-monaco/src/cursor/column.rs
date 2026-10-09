//! Column (box) selection.
//!
//! Ports Monaco's `src/vs/editor/common/cursor/cursorColumnSelection.ts`, the
//! column select commands of `src/vs/editor/browser/coreCommands.ts` and
//! `CursorsController.getCursorColumnSelectData` from
//! `src/vs/editor/common/cursor/cursor.ts` (MIT).

use super::{
    CursorCollection, CursorConfig, CursorModel, Position, SelectionStartKind, SingleCursorState,
    last_row, validate_position,
};

/// The rectangle of an ongoing column selection, in rows and visible
/// columns (Monaco `IColumnSelectData`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnSelectData {
    pub from_row: u32,
    pub from_visible_column: u32,
    pub to_row: u32,
    pub to_visible_column: u32,
}

/// Keyboard column selection (Monaco `cursorColumnSelect*`, Shift+Alt+Arrow
/// on macOS with Cmd).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnSelectCommand {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
}

/// Monaco `ColumnSelection.columnSelect`: one cursor per row in
/// `from_row..=to_row`, spanning the visible columns.
fn column_select_states(
    config: &CursorConfig,
    model: &dyn CursorModel,
    data: ColumnSelectData,
) -> Vec<SingleCursorState> {
    let ColumnSelectData {
        from_row,
        from_visible_column,
        to_row,
        to_visible_column,
    } = data;
    let line_count = from_row.abs_diff(to_row) + 1;
    let reversed = from_row > to_row;
    let is_rtl = from_visible_column > to_visible_column;
    let is_ltr = from_visible_column < to_visible_column;
    let row_at = |i: u32| if reversed { from_row - i } else { from_row + i };

    let mut result = Vec::new();
    for i in 0..line_count {
        let row = row_at(i);
        let start_column =
            config.column_from_visible_column(model, row, from_visible_column as i64);
        let end_column = config.column_from_visible_column(model, row, to_visible_column as i64);
        let visible_start_column =
            config.visible_column_from_column(model, Position::new(row, start_column));
        let visible_end_column =
            config.visible_column_from_column(model, Position::new(row, end_column));
        if is_ltr
            && (visible_start_column > to_visible_column
                || visible_end_column < from_visible_column)
        {
            continue;
        }
        if is_rtl
            && (visible_end_column > from_visible_column
                || visible_start_column < to_visible_column)
        {
            continue;
        }
        let start = Position::new(row, start_column);
        result.push(SingleCursorState {
            selection_start: start..start,
            selection_start_kind: SelectionStartKind::Simple,
            selection_start_leftover_visible_columns: 0,
            position: Position::new(row, end_column),
            leftover_visible_columns: 0,
        });
    }
    if result.is_empty() {
        // We are after all the lines: add a cursor at the end of each line.
        for i in 0..line_count {
            let row = row_at(i);
            result.push(SingleCursorState::from_position(Position::new(
                row,
                model.line_max_column(row),
            )));
        }
    }
    result
}

/// The current column selection rectangle, or one derived from the primary
/// cursor (Monaco `getCursorColumnSelectData`).
fn current_data(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &CursorCollection,
) -> ColumnSelectData {
    if let Some(data) = cursors.column_select_data() {
        return data;
    }
    let primary = cursors.primary();
    let start = primary.selection_start.start;
    ColumnSelectData {
        from_row: start.row,
        from_visible_column: config.visible_column_from_column(model, start),
        to_row: primary.position.row,
        to_visible_column: config.visible_column_from_column(model, primary.position),
    }
}

fn apply(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &mut CursorCollection,
    data: ColumnSelectData,
) {
    let states = column_select_states(config, model, data);
    cursors.validated_states(model, states);
    cursors.set_column_select_data(Some(data));
}

/// Extends the column selection with the keyboard. `page_lines` is the
/// page size for PageUp/PageDown.
pub fn column_select(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
    command: ColumnSelectCommand,
    page_lines: u32,
) {
    let model: &dyn CursorModel = model;
    let mut data = current_data(config, model, cursors);
    match command {
        ColumnSelectCommand::Left => {
            data.to_visible_column = data.to_visible_column.saturating_sub(1);
        }
        ColumnSelectCommand::Right => {
            let min_row = data.from_row.min(data.to_row);
            let max_row = data.from_row.max(data.to_row);
            let max_visible_column = (min_row..=max_row)
                .map(|row| {
                    config.visible_column_from_column(
                        model,
                        Position::new(row, model.line_max_column(row)),
                    )
                })
                .max()
                .unwrap_or(0);
            if data.to_visible_column < max_visible_column {
                data.to_visible_column += 1;
            }
        }
        ColumnSelectCommand::Up | ColumnSelectCommand::PageUp => {
            let lines = if command == ColumnSelectCommand::PageUp {
                page_lines.max(1)
            } else {
                1
            };
            data.to_row = data.to_row.saturating_sub(lines);
        }
        ColumnSelectCommand::Down | ColumnSelectCommand::PageDown => {
            let lines = if command == ColumnSelectCommand::PageDown {
                page_lines.max(1)
            } else {
                1
            };
            data.to_row = (data.to_row + lines).min(last_row(model));
        }
    }
    apply(config, model, cursors, data);
}

/// Column selection with the mouse (Monaco `columnSelect`, Shift+Alt+drag).
/// `to` is the position under the mouse and `mouse_visible_column` the
/// 0-based visible column under it (which may be past the line end). With
/// `extend`, continues the current column selection; otherwise starts a new
/// one at `to`.
pub fn column_select_to(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
    to: Position,
    mouse_visible_column: u32,
    extend: bool,
) {
    let model: &dyn CursorModel = model;
    let to = validate_position(model, to);
    let prev = current_data(config, model, cursors);
    let data = ColumnSelectData {
        from_row: if extend { prev.from_row } else { to.row },
        from_visible_column: if extend {
            prev.from_visible_column
        } else {
            mouse_visible_column
        },
        to_row: to.row,
        to_visible_column: mouse_visible_column,
    };
    apply(config, model, cursors, data);
}
