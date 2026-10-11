//! Cursor movement and selection commands.
//!
//! Ports Monaco's `src/vs/editor/common/cursor/cursorMoveOperations.ts`,
//! `src/vs/editor/common/cursor/cursorMoveCommands.ts` and the navigation
//! commands of `src/vs/editor/browser/coreCommands.ts` (MIT). There is no
//! view model here (no word wrap, folding or injected text), so Monaco's
//! "view" and "model" variants of each move coincide.

use super::word::{self, WordClassifier, WordNavigationType};
use super::{
    CursorCollection, CursorConfig, CursorModel, Position, SelectionStartKind, SingleCursorState,
    first_non_whitespace_column, last_row, max_position, strings, validate_position,
};

/// A cursor movement, as bound to keys by Monaco's default keymap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveCommand {
    /// Monaco `cursorLeft`: one grapheme left, wrapping to the end of the
    /// previous line; without `select`, collapses a selection to its start.
    Left,
    /// Monaco `cursorRight`: one grapheme right, wrapping to the start of the
    /// next line; without `select`, collapses a selection to its end.
    Right,
    /// Monaco `cursorUp`: one line up, keeping the visible column.
    Up,
    /// Monaco `cursorDown`: one line down, keeping the visible column.
    Down,
    /// Monaco `cursorWordLeft` (Ctrl+Left / Alt+Left on macOS): word starts,
    /// skipping single separators glued to a word.
    WordLeft,
    /// Monaco `cursorWordRight` (Ctrl+Right / Alt+Right on macOS): word ends.
    WordRight,
    /// Monaco `cursorWordStartLeft`: the previous word start.
    WordStartLeft,
    /// Monaco `cursorWordStartRight`: the next word start.
    WordStartRight,
    /// Monaco `cursorWordEndLeft`: the previous word end.
    WordEndLeft,
    /// Monaco `cursorWordEndRight`: the next word end.
    WordEndRight,
    /// Monaco `cursorWordPartLeft`: the previous camelCase, `snake_case` or
    /// `kebab-case` word part boundary.
    WordPartLeft,
    /// Monaco `cursorWordPartRight`: the next camelCase, `snake_case` or
    /// `kebab-case` word part boundary.
    WordPartRight,
    /// Toggles between the first non-whitespace column and column 0.
    Home,
    /// Monaco `cursorEnd`: the end of the line.
    End,
    /// Monaco `cursorLineStart` (Ctrl+A on macOS): column 0 of the line,
    /// without [`MoveCommand::Home`]'s toggle.
    LineStart,
    /// Monaco `cursorLineEnd` (Ctrl+E on macOS): the end of the line.
    LineEnd,
    /// Monaco `cursorTop`: the start of the text.
    Top,
    /// Monaco `cursorBottom`: the end of the text.
    Bottom,
    /// Monaco `cursorPageUp`: up by the `page_lines` passed to
    /// [`move_cursors`].
    PageUp,
    /// Monaco `cursorPageDown`: down by the `page_lines` passed to
    /// [`move_cursors`].
    PageDown,
    /// Column 0 of the previous blank line, past any blank lines the cursor
    /// is on (Monaco `cursorMove` to `prevBlankLine`); the first line if none.
    PrevBlankLine,
    /// Column 0 of the next blank line, past any blank lines the cursor is
    /// on (Monaco `cursorMove` to `nextBlankLine`); the last line if none.
    NextBlankLine,
}

/// Moves every cursor (Monaco's `CoreNavigationCommands` and word movement
/// commands). `select` extends the selections; `page_lines` is the number of
/// lines PageUp/PageDown move (Monaco's `pageSize`). Callers should push an
/// undo stop before moving, as Monaco does.
pub fn move_cursors(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
    command: MoveCommand,
    select: bool,
    page_lines: u32,
) {
    let model: &dyn CursorModel = model;
    let word_move = |cursors: &mut CursorCollection, left: bool, nav: WordNavigationType| {
        word::move_word(config, &DynModel(model), cursors, left, nav, select);
    };
    let per_cursor = |cursors: &mut CursorCollection,
                      f: &dyn Fn(&SingleCursorState) -> SingleCursorState| {
        let states = cursors.all().iter().map(f).collect();
        cursors.validated_states(model, states);
    };
    match command {
        MoveCommand::Left => per_cursor(cursors, &|c| move_left(config, model, c, select, 1)),
        MoveCommand::Right => per_cursor(cursors, &|c| move_right(config, model, c, select, 1)),
        MoveCommand::Up => per_cursor(cursors, &|c| move_up(config, model, c, select, 1)),
        MoveCommand::Down => per_cursor(cursors, &|c| move_down(config, model, c, select, 1)),
        MoveCommand::PageUp => {
            let n = page_lines.max(1);
            per_cursor(cursors, &|c| move_up(config, model, c, select, n))
        }
        MoveCommand::PageDown => {
            let n = page_lines.max(1);
            per_cursor(cursors, &|c| move_down(config, model, c, select, n))
        }
        MoveCommand::Home => per_cursor(cursors, &|c| move_to_beginning_of_line(model, c, select)),
        MoveCommand::End | MoveCommand::LineEnd => {
            per_cursor(cursors, &|c| move_to_end_of_line(model, c, select, false))
        }
        MoveCommand::LineStart => per_cursor(cursors, &|c| {
            c.move_to(select, Position::new(c.position.row, 0), 0)
        }),
        MoveCommand::Top => per_cursor(cursors, &|c| c.move_to(select, Position::zero(), 0)),
        MoveCommand::Bottom => per_cursor(cursors, &|c| c.move_to(select, max_position(model), 0)),
        MoveCommand::PrevBlankLine => {
            per_cursor(cursors, &|c| move_to_prev_blank_line(model, c, select))
        }
        MoveCommand::NextBlankLine => {
            per_cursor(cursors, &|c| move_to_next_blank_line(model, c, select))
        }
        MoveCommand::WordLeft => word_move(cursors, true, WordNavigationType::WordStartFast),
        MoveCommand::WordRight => word_move(cursors, false, WordNavigationType::WordEnd),
        MoveCommand::WordStartLeft => word_move(cursors, true, WordNavigationType::WordStart),
        MoveCommand::WordStartRight => word_move(cursors, false, WordNavigationType::WordStart),
        MoveCommand::WordEndLeft => word_move(cursors, true, WordNavigationType::WordEnd),
        MoveCommand::WordEndRight => word_move(cursors, false, WordNavigationType::WordEnd),
        MoveCommand::WordPartLeft | MoveCommand::WordPartRight => {
            let classifier = WordClassifier::for_config(config);
            let has_multicursor = cursors.all().len() > 1;
            let left = command == MoveCommand::WordPartLeft;
            word::move_selections_by(model, cursors, select, |p| {
                if left {
                    word::move_word_part_left(&classifier, model, p, has_multicursor)
                } else {
                    word::move_word_part_right(&classifier, model, p)
                }
            });
        }
    }
}

/// Adapts `&dyn CursorModel` to the `impl CursorModel` public functions.
pub(crate) struct DynModel<'a>(pub &'a dyn CursorModel);

impl CursorModel for DynModel<'_> {
    fn line_count(&self) -> u32 {
        self.0.line_count()
    }

    fn line_content(&self, row: u32) -> String {
        self.0.line_content(row)
    }

    fn line_max_column(&self, row: u32) -> u32 {
        self.0.line_max_column(row)
    }
}

// ---------------------------------------------------------------------------
// MoveOperations

/// Monaco `MoveOperations.leftPosition`: one grapheme left, wrapping to the
/// end of the previous line.
pub(crate) fn left_position(model: &dyn CursorModel, p: Position) -> Position {
    if p.column > 0 {
        let line = model.line_content(p.row);
        Position::new(p.row, p.column - strings::prev_char_length(&line, p.column))
    } else if p.row > 0 {
        Position::new(p.row - 1, model.line_max_column(p.row - 1))
    } else {
        p
    }
}

fn left_position_atomic_soft_tabs(model: &dyn CursorModel, p: Position, tab_size: u32) -> Position {
    let line = model.line_content(p.row);
    if p.column <= strings::indent_len(&line) {
        let units = strings::utf16(&line);
        if let Some(column) = strings::atomic_position(
            &units,
            p.column as usize,
            tab_size,
            strings::AtomicDirection::Left,
        ) {
            return Position::new(p.row, column as u32);
        }
    }
    left_position(model, p)
}

fn left(config: &CursorConfig, model: &dyn CursorModel, p: Position) -> Position {
    if config.sticky_tab_stops {
        left_position_atomic_soft_tabs(model, p, config.tab_size)
    } else {
        left_position(model, p)
    }
}

fn clip_column(model: &dyn CursorModel, row: u32, column: i64) -> Position {
    Position::new(
        row,
        column.clamp(0, model.line_max_column(row) as i64) as u32,
    )
}

/// Monaco `MoveOperations.moveLeft`.
pub(crate) fn move_left(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    in_selection_mode: bool,
    no_of_columns: u32,
) -> SingleCursorState {
    let p = if cursor.has_selection() && !in_selection_mode {
        // Collapse to the start of the selection.
        cursor.selection().start()
    } else {
        let pos = clip_column(
            model,
            cursor.position.row,
            cursor.position.column as i64 - (no_of_columns as i64 - 1),
        );
        left(config, model, pos)
    };
    cursor.move_to(in_selection_mode, p, 0)
}

/// Monaco `MoveOperations.rightPosition`: one grapheme right, wrapping to the
/// start of the next line.
pub(crate) fn right_position(model: &dyn CursorModel, row: u32, column: u32) -> Position {
    if column < model.line_max_column(row) {
        let line = model.line_content(row);
        Position::new(row, column + strings::next_char_length(&line, column))
    } else if row < last_row(model) {
        Position::new(row + 1, 0)
    } else {
        Position::new(row, column)
    }
}

fn right_position_atomic_soft_tabs(
    model: &dyn CursorModel,
    row: u32,
    column: u32,
    tab_size: u32,
) -> Position {
    let line = model.line_content(row);
    if column < strings::indent_len(&line) {
        let units = strings::utf16(&line);
        if let Some(c) = strings::atomic_position(
            &units,
            column as usize,
            tab_size,
            strings::AtomicDirection::Right,
        ) {
            return Position::new(row, c as u32);
        }
    }
    right_position(model, row, column)
}

/// Monaco `MoveOperations.right`.
pub(crate) fn right(config: &CursorConfig, model: &dyn CursorModel, p: Position) -> Position {
    if config.sticky_tab_stops {
        right_position_atomic_soft_tabs(model, p.row, p.column, config.tab_size)
    } else {
        right_position(model, p.row, p.column)
    }
}

/// Monaco `MoveOperations.moveRight`.
pub(crate) fn move_right(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    in_selection_mode: bool,
    no_of_columns: u32,
) -> SingleCursorState {
    let p = if cursor.has_selection() && !in_selection_mode {
        // Collapse to the end of the selection.
        cursor.selection().end()
    } else {
        let pos = clip_column(
            model,
            cursor.position.row,
            cursor.position.column as i64 + (no_of_columns as i64 - 1),
        );
        right(config, model, pos)
    };
    cursor.move_to(in_selection_mode, p, 0)
}

/// Monaco `MoveOperations.vertical`: moves to `new_row`, keeping the visible
/// column (plus leftover). Returns the position and the new leftover.
pub(crate) fn vertical(
    config: &CursorConfig,
    model: &dyn CursorModel,
    row: u32,
    column: u32,
    leftover_visible_columns: i32,
    new_row: i64,
    allow_move_on_edge_line: bool,
) -> (Position, i32) {
    let current_visible_column =
        strings::visible_column_from_column(&model.line_content(row), column, config.tab_size)
            as i64
            + leftover_visible_columns as i64;
    let last = last_row(model);
    let was_on_first_position = row == 0 && column == 0;
    let was_on_last_position = row == last && column == model.line_max_column(row);
    let was_at_edge_position = if new_row < row as i64 {
        was_on_first_position
    } else {
        was_on_last_position
    };

    let (row, column) = if new_row < 0 {
        let column = if allow_move_on_edge_line {
            0
        } else {
            model.line_max_column(0).min(column)
        };
        (0, column)
    } else if new_row > last as i64 {
        let column = if allow_move_on_edge_line {
            model.line_max_column(last)
        } else {
            model.line_max_column(last).min(column)
        };
        (last, column)
    } else {
        let row = new_row as u32;
        (
            row,
            config.column_from_visible_column(model, row, current_visible_column),
        )
    };

    let leftover = if was_at_edge_position {
        0
    } else {
        current_visible_column
            - strings::visible_column_from_column(&model.line_content(row), column, config.tab_size)
                as i64
    };
    (Position::new(row, column), leftover as i32)
}

/// Monaco `MoveOperations.moveDown`.
pub(crate) fn move_down(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    in_selection_mode: bool,
    lines_count: u32,
) -> SingleCursorState {
    let from = if cursor.has_selection() && !in_selection_mode {
        cursor.selection().end()
    } else {
        cursor.position
    };
    let (p, leftover) = vertical(
        config,
        model,
        from.row,
        from.column,
        cursor.leftover_visible_columns,
        from.row as i64 + lines_count as i64,
        true,
    );
    cursor.move_to(in_selection_mode, p, leftover)
}

/// Monaco `MoveOperations.moveUp`.
pub(crate) fn move_up(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    in_selection_mode: bool,
    lines_count: u32,
) -> SingleCursorState {
    let from = if cursor.has_selection() && !in_selection_mode {
        cursor.selection().start()
    } else {
        cursor.position
    };
    let (p, leftover) = vertical(
        config,
        model,
        from.row,
        from.column,
        cursor.leftover_visible_columns,
        from.row as i64 - lines_count as i64,
        true,
    );
    cursor.move_to(in_selection_mode, p, leftover)
}

/// Monaco `MoveOperations.translateDown`/`translateUp`: the cursor one line
/// below/above, as a new collapsed-start cursor.
fn translate(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    delta: i64,
) -> SingleCursorState {
    let selection = cursor.selection();
    let (start, start_leftover) = vertical(
        config,
        model,
        selection.anchor.row,
        selection.anchor.column,
        cursor.selection_start_leftover_visible_columns,
        selection.anchor.row as i64 + delta,
        false,
    );
    let (position, leftover) = vertical(
        config,
        model,
        selection.head.row,
        selection.head.column,
        cursor.leftover_visible_columns,
        selection.head.row as i64 + delta,
        false,
    );
    SingleCursorState {
        selection_start: start..start,
        selection_start_kind: SelectionStartKind::Simple,
        selection_start_leftover_visible_columns: start_leftover,
        position,
        leftover_visible_columns: leftover,
    }
}

fn is_blank_line(model: &dyn CursorModel, row: u32) -> bool {
    first_non_whitespace_column(model, row).is_none()
}

/// Monaco `MoveOperations.moveToPrevBlankLine`.
fn move_to_prev_blank_line(
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    select: bool,
) -> SingleCursorState {
    let mut row = cursor.position.row;
    while row > 0 && is_blank_line(model, row) {
        row -= 1;
    }
    while row > 0 && !is_blank_line(model, row) {
        row -= 1;
    }
    cursor.move_to(select, Position::new(row, 0), 0)
}

/// Monaco `MoveOperations.moveToNextBlankLine`.
fn move_to_next_blank_line(
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    select: bool,
) -> SingleCursorState {
    let last = last_row(model);
    let mut row = cursor.position.row;
    while row < last && is_blank_line(model, row) {
        row += 1;
    }
    while row < last && !is_blank_line(model, row) {
        row += 1;
    }
    cursor.move_to(select, Position::new(row, 0), 0)
}

/// Monaco `MoveOperations.moveToBeginningOfLine`.
fn move_to_beginning_of_line(
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    select: bool,
) -> SingleCursorState {
    let row = cursor.position.row;
    let first_non_blank_column = first_non_whitespace_column(model, row).unwrap_or(0);
    let column = if cursor.position.column == first_non_blank_column {
        0
    } else {
        first_non_blank_column
    };
    cursor.move_to(select, Position::new(row, column), 0)
}

/// Monaco `MoveOperations.moveToEndOfLine`. With `sticky`, the cursor keeps
/// to line ends when later moving vertically.
fn move_to_end_of_line(
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    select: bool,
    sticky: bool,
) -> SingleCursorState {
    let row = cursor.position.row;
    let max_column = model.line_max_column(row);
    let leftover = if sticky {
        (1i32 << 30) - max_column as i32
    } else {
        0
    };
    cursor.move_to(select, Position::new(row, max_column), leftover)
}

// ---------------------------------------------------------------------------
// Selection commands

/// Selects the whole text, leaving a single cursor (Monaco `selectAll`).
pub fn select_all(model: &impl CursorModel, cursors: &mut CursorCollection) {
    let model: &dyn CursorModel = model;
    let state = SingleCursorState {
        selection_start: Position::zero()..Position::zero(),
        selection_start_kind: SelectionStartKind::Simple,
        selection_start_leftover_visible_columns: 0,
        position: max_position(model),
        leftover_visible_columns: 0,
    };
    cursors.set_states(vec![state]);
}

/// Extends each selection to cover whole lines, then one more line on each
/// repeat (Monaco `expandLineSelection`, Cmd+L).
pub fn expand_line_selection(model: &impl CursorModel, cursors: &mut CursorCollection) {
    let model: &dyn CursorModel = model;
    let last = last_row(model);
    let states = cursors
        .all()
        .iter()
        .map(|cursor| {
            let selection = cursor.selection();
            let start_row = selection.start().row;
            let mut end_row = selection.end().row;
            let end_column = if end_row == last {
                model.line_max_column(last)
            } else {
                end_row += 1;
                0
            };
            let start = Position::new(start_row, 0);
            SingleCursorState {
                selection_start: start..start,
                selection_start_kind: SelectionStartKind::Simple,
                selection_start_leftover_visible_columns: 0,
                position: Position::new(end_row, end_column),
                leftover_visible_columns: 0,
            }
        })
        .collect();
    cursors.validated_states(model, states);
}

/// Adds a cursor one line above each cursor (Monaco `addCursorUp`).
pub fn add_cursor_above(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
) {
    add_cursors(config, model, cursors, -1);
}

/// Adds a cursor one line below each cursor (Monaco `addCursorDown`).
pub fn add_cursor_below(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
) {
    add_cursors(config, model, cursors, 1);
}

fn add_cursors(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &mut CursorCollection,
    delta: i64,
) {
    let mut states = Vec::with_capacity(cursors.all().len() * 2);
    for cursor in cursors.all() {
        states.push(cursor.clone());
        states.push(translate(config, model, cursor, delta));
    }
    cursors.validated_states(model, states);
}

/// Moves the primary cursor to `p` (a click), dropping secondary cursors;
/// with `select`, extends the selection instead (a shift-click or drag),
/// honouring word and line selections started by [`select_word`] and
/// [`select_line`] (Monaco `CursorMoveCommands.moveTo`).
pub fn move_to(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
    p: Position,
    select: bool,
) {
    let model: &dyn CursorModel = model;
    let primary = cursors.primary().clone();
    if select {
        match primary.selection_start_kind {
            SelectionStartKind::Word => return select_word_impl(config, model, cursors, p, true),
            SelectionStartKind::Line => return select_line_impl(model, cursors, p, true),
            SelectionStartKind::Simple => {}
        }
    }
    let p = validate_position(model, p);
    cursors.validated_states(model, vec![primary.move_to(select, p, 0)]);
}

/// Selects the word at `p` with the primary cursor (double click), or with
/// `select` extends a word selection to `p` (Monaco
/// `CursorMoveCommands.word`).
pub fn select_word(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
    p: Position,
    select: bool,
) {
    select_word_impl(config, model, cursors, p, select)
}

fn select_word_impl(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursors: &mut CursorCollection,
    p: Position,
    select: bool,
) {
    let p = validate_position(model, p);
    let state = word::word_selection(config, model, cursors.primary(), select, p);
    cursors.validated_states(model, vec![state]);
}

/// Selects the line at `p` with the primary cursor (triple click), or with
/// `select` extends a line selection to `p` (Monaco
/// `CursorMoveCommands.line`).
pub fn select_line(
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
    p: Position,
    select: bool,
) {
    select_line_impl(model, cursors, p, select)
}

fn select_line_impl(
    model: &dyn CursorModel,
    cursors: &mut CursorCollection,
    p: Position,
    select: bool,
) {
    let position = validate_position(model, p);
    let last = last_row(model);
    let line_end = |row: u32| {
        if row + 1 > last {
            Position::new(last, model.line_max_column(last))
        } else {
            Position::new(row + 1, 0)
        }
    };
    let primary = cursors.primary().clone();
    let state = if !select {
        let end = line_end(position.row);
        SingleCursorState {
            selection_start: Position::new(position.row, 0)..end,
            selection_start_kind: SelectionStartKind::Line,
            selection_start_leftover_visible_columns: 0,
            position: end,
            leftover_visible_columns: 0,
        }
    } else {
        let entering_row = primary.selection_start.start.row;
        if position.row < entering_row {
            primary.move_to(true, Position::new(position.row, 0), 0)
        } else if position.row > entering_row {
            primary.move_to(true, line_end(position.row), 0)
        } else {
            primary.move_to(true, primary.selection_start.end, 0)
        }
    };
    cursors.validated_states(model, vec![state]);
}
