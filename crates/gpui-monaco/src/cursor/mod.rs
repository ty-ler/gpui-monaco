//! Monaco's cursor and editing core, without any gpui dependency.
//!
//! Ports Monaco's `src/vs/editor/common/cursorCommon.ts` and the files under
//! `src/vs/editor/common/cursor/` (MIT); see each submodule for the exact
//! sources. Positions are [`text::PointUtf16`] (0-based row, 0-based UTF-16
//! column); Monaco's 1-based positions are these plus one.
//!
//! Navigation functions mutate a [`CursorCollection`]. Editing functions are
//! pure: they read the model and the cursors and return an [`EditOperation`]
//! describing the text edits and the resulting selections, which the caller
//! applies atomically (and records for undo) before calling
//! [`CursorCollection::set_selections`].
//!
//! Language-aware behaviour (auto-closing pairs, surround selection, onEnter
//! rules, electric characters, auto-indent) is intentionally absent: the
//! editing functions here are the language-agnostic fallbacks Monaco uses
//! when no language configuration applies, and a later language layer can
//! intercept typed characters before falling back to [`type_text`] and
//! [`line_break`].

mod collection;
mod column;
mod edit;
mod lines;
mod movement;
mod strings;
mod typing;
mod word;

#[cfg(test)]
mod tests;

use std::ops::Range;

pub use collection::{CursorCollection, SelectionStartKind, SingleCursorState};
pub use column::{ColumnSelectCommand, ColumnSelectData, column_select, column_select_to};
pub use edit::{EditOperation, EditOperationType};
pub use lines::{
    copy_lines_down, copy_lines_up, delete_lines, indent_lines, insert_line_above,
    insert_line_below, move_lines_down, move_lines_up, outdent_lines,
};
pub use movement::{
    MoveCommand, add_cursor_above, add_cursor_below, expand_line_selection, move_cursors, move_to,
    select_all, select_line, select_word,
};
pub use text::PointUtf16 as Position;
pub use typing::{
    composition_type, copy_text, cut, delete_all_left, delete_inside_word, delete_left,
    delete_right, delete_word, delete_word_left, delete_word_part, delete_word_right, line_break,
    outdent, paste, tab, type_text,
};
pub use word::{WordNavigationType, move_word, word_at};

/// Monaco's `USUAL_WORD_SEPARATORS`.
pub const USUAL_WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

/// How pasting text into multiple cursors distributes it (Monaco
/// `editor.multiCursorPaste`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MultiCursorPaste {
    /// Each cursor gets one line of the pasted text when the line count
    /// matches the cursor count.
    #[default]
    Spread,
    /// Each cursor gets the full text.
    Full,
}

/// The subset of Monaco's `CursorConfiguration` that the cursor core uses.
#[derive(Clone, Debug)]
pub struct CursorConfig {
    pub tab_size: u32,
    pub indent_size: u32,
    pub insert_spaces: bool,
    pub word_separators: String,
    pub sticky_tab_stops: bool,
    pub multi_cursor_paste: MultiCursorPaste,
    pub empty_selection_clipboard: bool,
    /// Monaco `editor.useTabStops`: backspace inside indentation deletes to
    /// the previous indent stop, and Tab/Shift+Tab snap to indent stops.
    pub use_tab_stops: bool,
    /// Monaco `editor.trimWhitespaceOnDelete`: deleting a line break also
    /// deletes the next line's leading whitespace.
    pub trim_whitespace_on_delete: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            tab_size: 4,
            indent_size: 4,
            insert_spaces: true,
            word_separators: USUAL_WORD_SEPARATORS.to_string(),
            sticky_tab_stops: false,
            multi_cursor_paste: MultiCursorPaste::Spread,
            empty_selection_clipboard: true,
            use_tab_stops: true,
            trim_whitespace_on_delete: false,
        }
    }
}

impl CursorConfig {
    pub(crate) fn visible_column_from_column(&self, model: &dyn CursorModel, p: Position) -> u32 {
        strings::visible_column_from_column(&model.line_content(p.row), p.column, self.tab_size)
    }

    /// Monaco `CursorConfiguration.columnFromVisibleColumn`.
    pub(crate) fn column_from_visible_column(
        &self,
        model: &dyn CursorModel,
        row: u32,
        visible_column: i64,
    ) -> u32 {
        let result = strings::column_from_visible_column(
            &model.line_content(row),
            visible_column,
            self.tab_size,
        );
        result.min(model.line_max_column(row))
    }

    pub(crate) fn normalize_indentation(&self, s: &str) -> String {
        strings::normalize_indentation(s, self.indent_size, self.insert_spaces)
    }
}

/// The text the cursor logic reads (Monaco `ICursorSimpleModel`).
pub trait CursorModel {
    /// Number of lines; always at least 1.
    fn line_count(&self) -> u32;
    /// Contents of `row`, without the line break.
    fn line_content(&self, row: u32) -> String;
    /// UTF-16 length of `row`, i.e. the largest valid column on it.
    fn line_max_column(&self, row: u32) -> u32;
}

impl CursorModel for text::Rope {
    fn line_count(&self) -> u32 {
        text::Rope::line_count(self)
    }

    fn line_content(&self, row: u32) -> String {
        self.line(row)
    }

    fn line_max_column(&self, row: u32) -> u32 {
        self.line_len_utf16(row)
    }
}

impl CursorModel for text::Buffer {
    fn line_count(&self) -> u32 {
        self.rope().line_count()
    }

    fn line_content(&self, row: u32) -> String {
        self.rope().line(row)
    }

    fn line_max_column(&self, row: u32) -> u32 {
        self.rope().line_len_utf16(row)
    }
}

/// A selection: `anchor` is where it started (Monaco's selection start) and
/// `head` is the cursor position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Selection {
    pub anchor: Position,
    pub head: Position,
}

impl Selection {
    pub fn new(anchor: Position, head: Position) -> Self {
        Self { anchor, head }
    }

    pub fn collapsed(p: Position) -> Self {
        Self { anchor: p, head: p }
    }

    /// A selection covering `range`, reversed (head at the start) if
    /// `reversed`.
    pub fn from_range(range: Range<Position>, reversed: bool) -> Self {
        if reversed {
            Self::new(range.end, range.start)
        } else {
            Self::new(range.start, range.end)
        }
    }

    pub fn start(&self) -> Position {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> Position {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// True if the head comes before the anchor (Monaco
    /// `SelectionDirection.RTL`).
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    pub fn range(&self) -> Range<Position> {
        self.start()..self.end()
    }
}

// ---------------------------------------------------------------------------
// Model helpers shared by the submodules (Monaco `ITextModel` methods).

pub(crate) fn line_units(model: &dyn CursorModel, row: u32) -> Vec<u16> {
    strings::utf16(&model.line_content(row))
}

pub(crate) fn last_row(model: &dyn CursorModel) -> u32 {
    model.line_count().saturating_sub(1)
}

pub(crate) fn max_position(model: &dyn CursorModel) -> Position {
    let row = last_row(model);
    Position::new(row, model.line_max_column(row))
}

/// Monaco `getLineFirstNonWhitespaceColumn`, 0-based; `None` for blank lines.
pub(crate) fn first_non_whitespace_column(model: &dyn CursorModel, row: u32) -> Option<u32> {
    strings::first_non_whitespace_index(&line_units(model, row)).map(|i| i as u32)
}

/// Monaco `TextModel.validatePosition`: clamps into the text and moves
/// positions that split a surrogate pair to before it.
pub(crate) fn validate_position(model: &dyn CursorModel, p: Position) -> Position {
    let line_count = model.line_count();
    if p.row >= line_count {
        return max_position(model);
    }
    let max = model.line_max_column(p.row);
    if p.column >= max {
        return Position::new(p.row, max);
    }
    if p.column > 0 && strings::splits_surrogate_pair(&model.line_content(p.row), p.column) {
        return Position::new(p.row, p.column - 1);
    }
    p
}

pub(crate) fn validate_range(model: &dyn CursorModel, r: Range<Position>) -> Range<Position> {
    let start = validate_position(model, r.start);
    let end = validate_position(model, r.end);
    if start <= end { start..end } else { end..start }
}

/// The text in `range` (lines joined with `\n`).
pub(crate) fn value_in_range(model: &dyn CursorModel, range: Range<Position>) -> String {
    if range.start.row == range.end.row {
        return strings::utf16_slice(
            &model.line_content(range.start.row),
            range.start.column,
            range.end.column,
        );
    }
    let mut result = strings::utf16_slice(
        &model.line_content(range.start.row),
        range.start.column,
        u32::MAX,
    );
    for row in range.start.row + 1..range.end.row {
        result.push('\n');
        result.push_str(&model.line_content(row));
    }
    result.push('\n');
    result.push_str(&strings::utf16_slice(
        &model.line_content(range.end.row),
        0,
        range.end.column,
    ));
    result
}
