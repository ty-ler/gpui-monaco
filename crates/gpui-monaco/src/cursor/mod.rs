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
pub(crate) use edit::execute_edits_forcing;
pub(crate) use edit::{Command, CursorStateHelper, EditBuilder, TrackedId, execute_commands};
pub use edit::{EditOperation, EditOperationType};
pub use in_place_replace::in_place_replace;
pub use language::{LanguageContext, StandardTokenType};
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
    add_cursors, line_state, move_left, move_right, move_to_beginning_of_line, move_to_end_of_line,
    move_to_state, vertical, word_state,
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
    outdent, paste, tab, type_text,
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
        rope.slice(start..end).to_string()
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
        let rope = self.rope();
        let max = rope.line_len_utf16(row);
        let (start, end) = (columns.start.min(max), columns.end.min(max));
        let start = rope.point_utf16_to_offset(Position::new(row, start.min(end)));
        let end = rope.point_utf16_to_offset(Position::new(row, end));
        rope.slice(start..end).to_string()
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
