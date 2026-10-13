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
//! Language-aware behaviour (auto-closing pairs and their overtyping and
//! deletion, surround selection, electric characters, onEnter rules and
//! indentation rules) lives in the `*_with_language` functions
//! ([`type_with_language`], [`line_break_with_language`],
//! [`delete_left_with_language`], [`tab_with_language`]), which take an
//! optional [`LanguageContext`]; without one they are exactly the
//! language-agnostic [`type_text`], [`line_break`], [`delete_left`] and
//! [`tab`]. Which closing characters the editor auto-inserted is remembered
//! across calls in [`CursorCollection::auto_closed_ranges`] (see
//! [`AutoClosedRanges`] for how to keep it up to date). An IME composition
//! runs between [`CursorCollection::start_composition`] and
//! [`composition_end_with_language`], which applies the interceptors to what
//! it typed.

mod auto_closed;
mod auto_indent_paste;
mod brackets;
mod collection;
mod column;
mod column_selection_paste;
mod comment;
mod composition;
mod edit;
pub(crate) mod in_place_replace;
mod language;
mod language_typing;
mod lines;
mod movement;
mod reindent;
pub mod smart_select;
pub(crate) mod strings;
mod transforms;
mod typing;
mod word;
mod word_segmenter;

#[cfg(test)]
mod tests;

use std::ops::Range;

pub use auto_closed::{AutoClosedPair, AutoClosedRanges};
pub use auto_indent_paste::auto_indent_on_paste;
pub(crate) use brackets::match_or_enclosing;
pub use brackets::{enclosing_brackets, match_bracket, matching_bracket, next_bracket};
pub use collection::{CursorCollection, SelectionStartKind, SingleCursorState};
pub use column::{ColumnSelectCommand, ColumnSelectData, column_select, column_select_to};
pub use comment::{CommentOptions, CommentsAt, LineCommentType, block_comment, line_comment};
pub use composition::{CompositionOutcome, CompositionState, composition_end_with_language};
pub(crate) use edit::{
    Command, CursorStateHelper, EditBuilder, Stickiness, TrackedId, execute_commands,
    transform_ranges_with,
};
pub use edit::{EditOperation, EditOperationType};
pub(crate) use edit::{execute_edits_forcing, selections_from_markers};
pub use in_place_replace::in_place_replace;
pub use language::{LanguageContext, LanguageScope, StandardTokenType};
pub use language_typing::{
    delete_left_with_language, delete_word_part_with_language, delete_word_with_language,
    line_break_insert, line_break_with_language, outdent_with_language, snippet_auto_closed_pairs,
    tab_with_language, type_with_language,
};
pub use lines::{
    copy_lines_down, copy_lines_up, delete_lines, indent_lines, indent_lines_with_language,
    insert_line_above, insert_line_above_with_language, insert_line_below,
    insert_line_below_with_language, move_lines_down, move_lines_down_with_language, move_lines_up,
    move_lines_up_with_language, outdent_lines, outdent_lines_with_language,
};
pub use movement::{
    MoveCommand, add_cursor_above, add_cursor_below, expand_line_selection, move_cursors, move_to,
    select_all, select_line, select_word,
};
pub(crate) use movement::{
    ViewportTarget, add_cursors, line_state, move_left, move_right, move_to_beginning_of_line,
    move_to_end_of_line, move_to_state, vertical, viewport_row, word_state,
};
pub use reindent::{reindent_lines, reindent_selected_lines};
pub use text::PointUtf16 as Position;
pub use transforms::{
    CaseTransform, delete_all_right, delete_duplicate_lines, duplicate_selection, join_lines,
    move_caret, reverse_lines, sort_lines, transform_case, transform_text_case, transpose,
    transpose_letters, trim_trailing_whitespace, trim_trailing_whitespace_with_language,
};
pub use typing::{
    composition_type, copy_text, cut, delete_all_left, delete_inside_word, delete_left,
    delete_right, delete_word, delete_word_left, delete_word_part, delete_word_right, line_break,
    outdent, paste, tab, type_text, type_without_interceptors,
};
pub(crate) use word::word_at_position;
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

/// When typing an opening character auto-inserts its closing pair (Monaco
/// `EditorAutoClosingStrategy`, used by `editor.autoClosingBrackets`,
/// `editor.autoClosingQuotes` and `editor.autoClosingComments`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoClosingStrategy {
    /// Always auto-close.
    Always,
    /// Auto-close when the next character is in the language's
    /// `autoCloseBefore` set (or Monaco's default set).
    #[default]
    LanguageDefined,
    /// Auto-close only before whitespace.
    BeforeWhitespace,
    /// Never auto-close.
    Never,
}

/// When typing over or deleting an auto-inserted closing character applies
/// (Monaco `EditorAutoClosingEditStrategy`, used by
/// `editor.autoClosingOvertype` and `editor.autoClosingDelete`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoClosingEditStrategy {
    /// For any matching closing character.
    Always,
    /// Only for closing characters the editor inserted itself (see
    /// [`AutoClosedRanges`]).
    #[default]
    Auto,
    /// Never.
    Never,
}

/// Which typed characters wrap a selection (Monaco
/// `EditorAutoSurroundStrategy`, `editor.autoSurround`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoSurroundStrategy {
    /// Quotes and brackets from the language's surrounding pairs.
    #[default]
    LanguageDefined,
    /// Only quotes.
    Quotes,
    /// Only brackets.
    Brackets,
    /// Never.
    Never,
}

/// How much the editor adjusts indentation when typing and pressing Enter
/// (Monaco `EditorAutoIndentStrategy`, `editor.autoIndent`). The variants
/// are ordered: each level includes the ones before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum AutoIndent {
    /// No automatic indentation; Enter inserts a bare line break.
    None,
    /// Keep the current line's indentation.
    Keep,
    /// Also honor the language's brackets (`{|}` + Enter indents).
    Brackets,
    /// Also apply the language's onEnter rules.
    Advanced,
    /// Also apply the language's indentation rules.
    #[default]
    Full,
}

/// The subset of Monaco's `CursorConfiguration` that the cursor core uses.
#[derive(Clone, Debug)]
pub struct CursorConfig {
    /// Width of a tab character in visible columns (Monaco `tabSize`).
    pub tab_size: u32,
    /// Visible columns per indentation level (Monaco `indentSize`).
    pub indent_size: u32,
    /// Whether indenting inserts spaces rather than tabs (Monaco
    /// `insertSpaces`).
    pub insert_spaces: bool,
    /// Characters that separate words (Monaco `editor.wordSeparators`).
    pub word_separators: String,
    /// BCP 47 tags of the languages whose words word navigation, deletion
    /// and selection find with a word segmenter (Monaco
    /// `editor.wordSegmenterLocales`). The first valid tag is used; none (the
    /// default) or the `word-segmenter` cargo feature off means words are
    /// found by `word_separators` alone.
    pub word_segmenter_locales: Vec<String>,
    /// Monaco `editor.stickyTabStops`: Left/Right move over a tab stop's
    /// worth of leading spaces at once.
    pub sticky_tab_stops: bool,
    /// Monaco `editor.multiCursorPaste`.
    pub multi_cursor_paste: MultiCursorPaste,
    /// Monaco `editor.emptySelectionClipboard`: copy and cut with an empty
    /// selection act on the whole line.
    pub empty_selection_clipboard: bool,
    /// Monaco `editor.useTabStops`: backspace inside indentation deletes to
    /// the previous indent stop, and Tab/Shift+Tab snap to indent stops.
    pub use_tab_stops: bool,
    /// Monaco `editor.trimWhitespaceOnDelete`: deleting a line break also
    /// deletes the next line's leading whitespace.
    pub trim_whitespace_on_delete: bool,
    /// Monaco `editor.autoClosingBrackets`.
    pub auto_closing_brackets: AutoClosingStrategy,
    /// Monaco `editor.autoClosingQuotes`.
    pub auto_closing_quotes: AutoClosingStrategy,
    /// Monaco `editor.autoClosingComments`.
    pub auto_closing_comments: AutoClosingStrategy,
    /// Monaco `editor.autoClosingOvertype`.
    pub auto_closing_overtype: AutoClosingEditStrategy,
    /// Monaco `editor.autoClosingDelete`.
    pub auto_closing_delete: AutoClosingEditStrategy,
    /// Monaco `editor.autoSurround`.
    pub auto_surround: AutoSurroundStrategy,
    /// Monaco `editor.autoIndent`. Only the `*_with_language` functions
    /// honor it, and only when given a [`LanguageContext`].
    pub auto_indent: AutoIndent,
    /// Typing replaces the characters after the cursor (Monaco's `overtype`
    /// input mode, toggled with Insert).
    pub overtype: bool,
    /// In overtype mode, pasting overtypes too (Monaco `overtypeOnPaste`).
    pub overtype_on_paste: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            tab_size: 4,
            indent_size: 4,
            insert_spaces: true,
            word_separators: USUAL_WORD_SEPARATORS.to_string(),
            word_segmenter_locales: Vec::new(),
            sticky_tab_stops: false,
            multi_cursor_paste: MultiCursorPaste::Spread,
            empty_selection_clipboard: true,
            use_tab_stops: true,
            trim_whitespace_on_delete: false,
            auto_closing_brackets: AutoClosingStrategy::LanguageDefined,
            auto_closing_quotes: AutoClosingStrategy::LanguageDefined,
            auto_closing_comments: AutoClosingStrategy::LanguageDefined,
            auto_closing_overtype: AutoClosingEditStrategy::Auto,
            auto_closing_delete: AutoClosingEditStrategy::Auto,
            auto_surround: AutoSurroundStrategy::LanguageDefined,
            auto_indent: AutoIndent::Full,
            overtype: false,
            overtype_on_paste: true,
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
    /// Whether `column` falls between the halves of a surrogate pair.
    fn splits_surrogate_pair(&self, row: u32, column: u32) -> bool {
        strings::splits_surrogate_pair(&self.line_content(row), column)
    }
    /// Monaco `normalizePosition`: `position` moved out of injected text
    /// (and off a wrap point's other side) as `affinity` says. Models
    /// without injected text or wrapping leave it as is.
    fn normalize_position(
        &self,
        position: Position,
        affinity: crate::view::injected_text::PositionAffinity,
    ) -> Position {
        let _ = affinity;
        position
    }
    /// The UTF-16 `columns` of `row` (clamped to it). Long lines are
    /// sliced without copying the whole line.
    fn line_slice(&self, row: u32, columns: Range<u32>) -> String {
        let units = strings::utf16(&self.line_content(row));
        let end = (columns.end as usize).min(units.len());
        let start = (columns.start as usize).min(end);
        String::from_utf16_lossy(&units[start..end])
    }
    /// The UTF-16 units of `row` in the columns `window` picks given the
    /// row's length, clamped to it, with those columns: [`line_slice`]
    /// without looking the row up twice.
    ///
    /// [`line_slice`]: CursorModel::line_slice
    fn line_units_in(
        &self,
        row: u32,
        window: &dyn Fn(u32) -> Range<u32>,
    ) -> (Vec<u16>, Range<u32>) {
        sliced_line_units_in(self, row, window)
    }
    /// The rope holding the text, if it's one: scans that visit many rows
    /// (bracket matching) then read them one after another without
    /// seeking it per row.
    fn rope(&self) -> Option<&text::Rope> {
        None
    }
}

/// The default [`CursorModel::line_units_in`]: the row's length, then the
/// slice.
fn sliced_line_units_in<M: CursorModel + ?Sized>(
    model: &M,
    row: u32,
    window: &dyn Fn(u32) -> Range<u32>,
) -> (Vec<u16>, Range<u32>) {
    let max = model.line_max_column(row);
    let columns = clamp_columns(window(max), max);
    let units = model
        .line_slice(row, columns.clone())
        .encode_utf16()
        .collect();
    (units, columns)
}

/// `columns` clamped to a row `max` units long.
fn clamp_columns(columns: Range<u32>, max: u32) -> Range<u32> {
    columns.start.min(max)..columns.end.min(max)
}

/// Rows up to this many bytes are read whole by [`rope_line_units_in`].
const SHORT_ROW_BYTES: usize = 4096;

/// [`CursorModel::line_units_in`] for a rope: a short row is read in one
/// pass from its start, else `None`. Two rope seeks instead of the five of
/// [`CursorModel::line_max_column`] and [`CursorModel::line_slice`].
fn rope_line_units_in(
    rope: &text::Rope,
    row: u32,
    window: &dyn Fn(u32) -> Range<u32>,
) -> Option<(Vec<u16>, Range<u32>)> {
    if row > rope.max_point().row {
        return None;
    }
    let start = rope.point_to_offset(text::Point::new(row, 0));
    let (units, _) = read_row_forward("", rope.chunks_in_range(start..rope.len()));
    window_units(units?, window)
}

/// The units of a row in the columns `window` picks, given all of them,
/// else `None` when a picked column falls between a surrogate pair's
/// halves (the rope clamps those, so the row is sliced instead).
fn window_units(
    mut units: Vec<u16>,
    window: &dyn Fn(u32) -> Range<u32>,
) -> Option<(Vec<u16>, Range<u32>)> {
    let columns = clamp_columns(window(units.len() as u32), units.len() as u32);
    // A column between a surrogate pair's halves clamps in the rope.
    let splits = |column: u32| {
        units
            .get(column as usize)
            .is_some_and(|&unit| (0xDC00..0xE000).contains(&unit))
    };
    if splits(columns.start) || splits(columns.end) {
        return None;
    }
    units.truncate(columns.end as usize);
    units.drain(..columns.start.min(columns.end) as usize);
    Some((units, columns))
}

/// A row read by [`read_row_forward`] or [`read_row_backward`]: its units
/// (`None` past [`SHORT_ROW_BYTES`]), and the unread text next to it with
/// the chunks beyond, unless it's the text's first or last row (or too long
/// to have been read to its other end).
type ReadRow<'a> = (Option<Vec<u16>>, Option<(&'a str, text::Chunks<'a>)>);

/// The row starting at `rest`, followed by `chunks`, and where the next
/// row starts.
fn read_row_forward<'a>(rest: &'a str, mut chunks: text::Chunks<'a>) -> ReadRow<'a> {
    let mut units: Vec<u16> = Vec::new();
    let mut bytes = 0;
    let mut piece = Some(rest);
    while let Some(text) = piece.take().or_else(|| chunks.next()) {
        let newline = text.find('\n');
        let line = &text[..newline.unwrap_or(text.len())];
        bytes += line.len();
        if bytes > SHORT_ROW_BYTES {
            return (None, None);
        }
        units.extend(line.encode_utf16());
        if let Some(newline) = newline {
            return (Some(units), Some((&text[newline + 1..], chunks)));
        }
    }
    (Some(units), None)
}

/// The row ending at the end of `rest`, preceded by the reversed `chunks`,
/// and where the row before it ends.
fn read_row_backward<'a>(rest: &'a str, mut chunks: text::Chunks<'a>) -> ReadRow<'a> {
    let mut pieces: Vec<&str> = Vec::new();
    let mut bytes = 0;
    let mut piece = Some(rest);
    let mut before = None;
    while let Some(text) = piece.take().or_else(|| chunks.next()) {
        let newline = text.rfind('\n');
        let line = &text[newline.map_or(0, |newline| newline + 1)..];
        bytes += line.len();
        if bytes > SHORT_ROW_BYTES {
            return (None, None);
        }
        pieces.push(line);
        if let Some(newline) = newline {
            before = Some(&text[..newline]);
            break;
        }
    }
    let mut units = Vec::with_capacity(bytes);
    for piece in pieces.iter().rev() {
        units.extend(piece.encode_utf16());
    }
    (Some(units), before.map(|before| (before, chunks)))
}

/// Where a [`RowReader`] goes on reading.
enum NextRow<'a> {
    None,
    /// `row` starts at `rest`, followed by `chunks`.
    Forward {
        row: u32,
        rest: &'a str,
        chunks: text::Chunks<'a>,
    },
    /// `row` ends at byte `end`; nothing read there yet.
    BackwardAt {
        row: u32,
        end: usize,
    },
    /// `row` ends at the end of `rest`, preceded by the reversed `chunks`.
    Backward {
        row: u32,
        rest: &'a str,
        chunks: text::Chunks<'a>,
    },
}

/// [`CursorModel::line_units_in`] for scans that visit row after row in
/// one direction (bracket matching): on a rope, a short row is read on from
/// where the row before it in that direction was read, instead of seeking
/// the rope for it. Any row can be asked for; one out of sequence is
/// looked up as usual.
pub(crate) struct RowReader<'a> {
    model: &'a dyn CursorModel,
    backward: bool,
    next: NextRow<'a>,
}

impl<'a> RowReader<'a> {
    /// A reader for rows read downwards, or upwards if `backward`.
    pub(crate) fn new(model: &'a dyn CursorModel, backward: bool) -> Self {
        Self {
            model,
            backward,
            next: NextRow::None,
        }
    }

    /// [`CursorModel::line_units_in`].
    pub(crate) fn line_units_in(
        &mut self,
        row: u32,
        window: &dyn Fn(u32) -> Range<u32>,
    ) -> (Vec<u16>, Range<u32>) {
        let model = self.model;
        let Some(rope) = model.rope() else {
            return model.line_units_in(row, window);
        };
        if let Some(units) = self.read(rope, row)
            && let Some(found) = window_units(units, window)
        {
            return found;
        }
        sliced_line_units_in(model, row, window)
    }

    /// All of `row`'s units, or `None` for a long row or one past the end,
    /// leaving `next` at the row after it in the reader's direction.
    fn read(&mut self, rope: &'a text::Rope, row: u32) -> Option<Vec<u16>> {
        let backward = self.backward;
        let (units, after) = match std::mem::replace(&mut self.next, NextRow::None) {
            NextRow::Forward {
                row: next,
                rest,
                chunks,
            } if !backward && next == row => read_row_forward(rest, chunks),
            NextRow::BackwardAt { row: next, end } if backward && next == row => {
                read_row_backward("", rope.reversed_chunks_in_range(0..end))
            }
            NextRow::Backward {
                row: next,
                rest,
                chunks,
            } if backward && next == row => read_row_backward(rest, chunks),
            _ => {
                if row > rope.max_point().row {
                    return None;
                }
                let start = rope.point_to_offset(text::Point::new(row, 0));
                let (units, after) = read_row_forward("", rope.chunks_in_range(start..rope.len()));
                if backward {
                    // The row above ends before this one's line break, even
                    // when this one is too long to read.
                    if row > 0 {
                        self.next = NextRow::BackwardAt {
                            row: row - 1,
                            end: start - 1,
                        };
                    }
                    return units;
                }
                (units, after)
            }
        };
        self.next = match after {
            Some((rest, chunks)) if backward => NextRow::Backward {
                row: row - 1,
                rest,
                chunks,
            },
            Some((rest, chunks)) => NextRow::Forward {
                row: row + 1,
                rest,
                chunks,
            },
            None => NextRow::None,
        };
        units
    }
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
    fn line_slice(&self, row: u32, columns: Range<u32>) -> String {
        let rope = self;
        let max = rope.line_len_utf16(row);
        let (start, end) = (columns.start.min(max), columns.end.min(max));
        let start = rope.point_utf16_to_offset(Position::new(row, start.min(end)));
        let end = rope.point_utf16_to_offset(Position::new(row, end));
        rope.text_for_range(start..end)
    }
    fn line_units_in(
        &self,
        row: u32,
        window: &dyn Fn(u32) -> Range<u32>,
    ) -> (Vec<u16>, Range<u32>) {
        rope_line_units_in(self, row, window)
            .unwrap_or_else(|| sliced_line_units_in(self, row, window))
    }
    fn rope(&self) -> Option<&text::Rope> {
        Some(self)
    }
    fn splits_surrogate_pair(&self, row: u32, column: u32) -> bool {
        // A column inside a pair doesn't survive clipping.
        let point = Position::new(row, column);
        self.clip_point_utf16(point, text::Bias::Left) != point
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
    fn line_slice(&self, row: u32, columns: Range<u32>) -> String {
        self.rope().line_slice(row, columns)
    }
    fn line_units_in(
        &self,
        row: u32,
        window: &dyn Fn(u32) -> Range<u32>,
    ) -> (Vec<u16>, Range<u32>) {
        self.rope().line_units_in(row, window)
    }
    fn rope(&self) -> Option<&text::Rope> {
        Some(text::Buffer::rope(self))
    }
    fn splits_surrogate_pair(&self, row: u32, column: u32) -> bool {
        // A column inside a pair doesn't survive clipping.
        let point = Position::new(row, column);
        self.rope().clip_point_utf16(point, text::Bias::Left) != point
    }
}

/// A selection: `anchor` is where it started (Monaco's selection start) and
/// `head` is the cursor position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Selection {
    /// Where the selection started; stays fixed while extending.
    pub anchor: Position,
    /// The cursor position.
    pub head: Position,
}

impl Selection {
    /// A selection from `anchor` to `head`.
    pub fn new(anchor: Position, head: Position) -> Self {
        Self { anchor, head }
    }

    /// An empty selection (a plain cursor) at `p`.
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

    /// The earlier of `anchor` and `head`.
    pub fn start(&self) -> Position {
        self.anchor.min(self.head)
    }

    /// The later of `anchor` and `head`.
    pub fn end(&self) -> Position {
        self.anchor.max(self.head)
    }

    /// True if `anchor` and `head` coincide.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// True if the head comes before the anchor (Monaco
    /// `SelectionDirection.RTL`).
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    /// The selected range, `start()..end()`, regardless of direction.
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
    if p.column > 0 && model.splits_surrogate_pair(p.row, p.column) {
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

#[cfg(test)]
mod line_units_tests {
    use std::ops::Range;

    use super::{CursorModel, Position, RowReader};

    /// The rope's one-pass `line_units_in` matches looking the row's length
    /// up and then slicing it, for every window: short and long rows,
    /// surrogate pairs (split or not), rows past the end.
    #[test]
    fn rope_line_units_match_length_then_slice() {
        let long = "a(".repeat(3000);
        let text = format!("fn a() {{}}\n\n😀x😀(é)\n{long}\n{long}😀\nend");
        let rope = text::Rope::from(text.as_str());
        // Slicing as `line_slice` did before it read the text directly.
        let slice = |row: u32, columns: Range<u32>| {
            let max = rope.line_len_utf16(row);
            let (start, end) = (columns.start.min(max), columns.end.min(max));
            let start = rope.point_utf16_to_offset(Position::new(row, start.min(end)));
            let end = rope.point_utf16_to_offset(Position::new(row, end));
            rope.slice(start..end).to_string()
        };
        let windows: Vec<Box<dyn Fn(u32) -> Range<u32>>> = vec![
            Box::new(|max| 0..max),
            Box::new(|_| 0..3),
            Box::new(|_| 1..4),
            Box::new(|_| 2..5),
            Box::new(|_| 0..20_000),
            Box::new(|max| max.saturating_sub(3)..max),
            Box::new(|max| max.saturating_sub(5000)..max),
            Box::new(|_| 4000..9000),
        ];
        for row in 0..rope.line_count() + 2 {
            for window in &windows {
                let max = rope.line_len_utf16(row);
                let columns = window(max).start.min(max)..window(max).end.min(max);
                let expected: Vec<u16> = slice(row, columns.clone()).encode_utf16().collect();
                assert_eq!(
                    rope.line_units_in(row, window),
                    (expected, columns.clone()),
                    "row {row}"
                );
                assert_eq!(rope.line_slice(row, columns.clone()), slice(row, columns));
            }
        }
    }

    /// Reading rows one after another (either way, from any row) gives
    /// exactly what looking each row up does: short, empty and long rows,
    /// surrogate pairs and other non-ASCII text across chunk boundaries,
    /// CRLF line breaks, a trailing line break, rows past the end, for
    /// every window, on a rope, a buffer and a model without a rope.
    #[test]
    fn row_reader_matches_row_by_row() {
        let long = "a(".repeat(3000);
        let wide = "é😀{".repeat(400);
        let crlf = "x\r\n(\r\n\r\n]";
        let texts = [
            String::new(),
            "\n".to_string(),
            "(".to_string(),
            format!("fn a() {{}}\n\n😀x😀(é)\n{long}\n{long}😀\nend"),
            format!("{wide}\n{long}\n\n{wide}{long}\n{wide}\n😀\n"),
            format!("{crlf}\n{wide}\r\n{long}\r\n{crlf}"),
            (0..300)
                .map(|i| format!("{}😀{}", "é".repeat(i % 7), "(".repeat(i % 5)))
                .collect::<Vec<_>>()
                .join("\n"),
        ];
        let windows: Vec<Box<dyn Fn(u32) -> Range<u32>>> = vec![
            Box::new(|max| 0..max),
            Box::new(|_| 0..3),
            Box::new(|_| 1..4),
            Box::new(|_| 0..20_000),
            Box::new(|max| max.saturating_sub(3)..max),
            Box::new(|max| max.saturating_sub(5000)..max),
            Box::new(|_| 4000..9000),
        ];
        for text in &texts {
            let rope = text::Rope::from(text.as_str());
            let buffer = text::Buffer::new(text);
            let plain = crate::cursor::movement::DynModel(&rope);
            let models: [&dyn CursorModel; 3] = [&rope, &buffer, &plain];
            for model in models {
                let count = model.line_count();
                for window in &windows {
                    // Each row looked up on its own.
                    let expected = |row: u32| model.line_units_in(row, window);
                    // From every row to the end, either way.
                    for first in 0..count {
                        let mut down = RowReader::new(model, false);
                        for row in first..count + 2 {
                            assert_eq!(down.line_units_in(row, window), expected(row));
                        }
                        let mut up = RowReader::new(model, true);
                        for row in (0..=first).rev() {
                            assert_eq!(up.line_units_in(row, window), expected(row));
                        }
                    }
                    // Out of sequence, and against the reader's direction.
                    for backward in [false, true] {
                        let mut reader = RowReader::new(model, backward);
                        for row in [2, 0, 1, 2, 5, 4, 3, count + 1, count - 1, 1, 0, 1] {
                            assert_eq!(reader.line_units_in(row, window), expected(row));
                        }
                    }
                }
            }
        }
    }
}
