//! Embedder conveniences matching Monaco's `ITextModel` and `ICodeEditor`
//! APIs: position helpers, words, searching, revealing, edits with cursor
//! state, visible ranges and running commands by name.

use std::ops::Range;
use std::sync::Arc;

use gpui::{App, Context, Entity, EntityId, WeakEntity, Window};
use text::{Bias, PointUtf16};

use crate::cursor::{self, Selection};
use crate::decorations::{Decoration, DecorationId, DecorationOptions};
use crate::editor::{CodeEditor, CodeEditorEvent, PendingReveal};
use crate::events::DecorationsChange;
use crate::model::TextModel;
use crate::search::{self, SearchParams};
use crate::view::reveal::{RevealType, ScrollType};

/// Which line breaks text read from a model has (Monaco
/// `EndOfLinePreference`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EndOfLinePreference {
    /// The model's line ending (Monaco `TextDefined`, the default).
    #[default]
    TextDefined,
    /// `\n` (Monaco `LF`), how the model stores its text.
    Lf,
    /// `\r\n` (Monaco `CRLF`).
    Crlf,
}

/// A word and where it is (Monaco `IWordAtPosition`, with a range).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordAtPosition {
    /// The word's text.
    pub word: String,
    /// Where it is (0-based row, UTF-16 columns).
    pub range: Range<PointUtf16>,
}

impl TextModel {
    /// The number of lines (Monaco `getLineCount`).
    pub fn line_count(&self) -> u32 {
        self.buffer().rope().line_count()
    }

    /// The text of `row`, without its line break (Monaco `getLineContent`).
    pub fn line_content(&self, row: u32) -> String {
        self.buffer().rope().line(row)
    }

    /// The UTF-16 length of `row` (Monaco `getLineMaxColumn` minus one).
    pub fn line_len_utf16(&self, row: u32) -> u32 {
        self.buffer().rope().line_len_utf16(row)
    }

    /// The nearest valid position to `position` (Monaco
    /// `validatePosition`): rows past the end go to the end, columns past
    /// a line's end to its end, and never inside a surrogate pair.
    pub fn validate_position(&self, position: PointUtf16) -> PointUtf16 {
        self.buffer().rope().clip_point_utf16(position, Bias::Left)
    }

    /// [`TextModel::validate_position`] for both ends, ordered (Monaco
    /// `validateRange`).
    pub fn validate_range(&self, range: Range<PointUtf16>) -> Range<PointUtf16> {
        let (a, b) = (
            self.validate_position(range.start),
            self.validate_position(range.end),
        );
        a.min(b)..a.max(b)
    }

    /// The UTF-8 byte offset of `position` in the stored text, whose line
    /// breaks are always `\n` (what [`TextModel::edit`], decorations'
    /// [`Decoration::range`] and [`ContentChange::range_offset`](crate::ContentChange)
    /// use). Monaco's `getOffsetAt` counts differently: see
    /// [`TextModel::get_offset_at`].
    pub fn offset_at(&self, position: PointUtf16) -> usize {
        let rope = self.buffer().rope();
        rope.point_utf16_to_offset(rope.clip_point_utf16(position, Bias::Left))
    }

    /// The position of byte `offset` of the stored text (the inverse of
    /// [`TextModel::offset_at`]). Monaco's `getPositionAt` is
    /// [`TextModel::get_position_at`].
    pub fn position_at(&self, offset: usize) -> PointUtf16 {
        let rope = self.buffer().rope();
        rope.offset_to_point_utf16(rope.clip_offset(offset.min(rope.len()), Bias::Left))
    }

    /// The line break `eol` asks for.
    fn eol_str(&self, eol: EndOfLinePreference) -> &'static str {
        match eol {
            EndOfLinePreference::TextDefined => self.line_ending().as_str(),
            EndOfLinePreference::Lf => "\n",
            EndOfLinePreference::Crlf => "\r\n",
        }
    }

    /// The text in `range` (Monaco `getValueInRange(range, eol)`), with
    /// `eol`'s line breaks: [`EndOfLinePreference::TextDefined`], Monaco's
    /// default, gives the model's.
    pub fn value_in_range(&self, range: Range<PointUtf16>, eol: EndOfLinePreference) -> String {
        let range = self.validate_range(range);
        let rope = self.buffer().rope();
        let text = rope.text_for_range(
            rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end),
        );
        match self.eol_str(eol) {
            "\n" => text,
            eol => text.replace('\n', eol),
        }
    }

    /// The whole text (Monaco `getValue(eol, preserveBOM)`), with `eol`'s
    /// line breaks, starting with the BOM the text was created or set with
    /// if `preserve_bom`. [`TextModel::text`] is `value(TextDefined,
    /// false)`.
    pub fn value(&self, eol: EndOfLinePreference, preserve_bom: bool) -> String {
        let text = self.value_in_range(self.full_model_range(), eol);
        if preserve_bom && self.buffer().has_bom() {
            format!("\u{feff}{text}")
        } else {
            text
        }
    }

    /// [`TextModel::value_length`] with `eol`'s line breaks, plus one for
    /// the BOM if `preserve_bom` and there is one (Monaco
    /// `getValueLength(eol, preserveBOM)`).
    pub fn value_length_with(&self, eol: EndOfLinePreference, preserve_bom: bool) -> usize {
        let bom = usize::from(preserve_bom && self.buffer().has_bom());
        self.value_length_in_range(self.full_model_range(), eol) + bom
    }

    /// The UTF-16 length of the text in `range`, with `eol`'s line breaks
    /// (Monaco `getValueLengthInRange`).
    pub fn value_length_in_range(
        &self,
        range: Range<PointUtf16>,
        eol: EndOfLinePreference,
    ) -> usize {
        let range = self.validate_range(range);
        let rope = self.buffer().rope();
        let (start, end) = (
            rope.point_utf16_to_offset(range.start),
            rope.point_utf16_to_offset(range.end),
        );
        let breaks = (range.end.row - range.start.row) as usize;
        rope.offset_to_offset_utf16(end).0 - rope.offset_to_offset_utf16(start).0
            + breaks * (self.eol_str(eol).len() - 1)
    }

    /// The number of characters in `range`, a surrogate pair counting as
    /// one and each line break as `eol`'s length (Monaco
    /// `getCharacterCountInRange`).
    pub fn character_count_in_range(
        &self,
        range: Range<PointUtf16>,
        eol: EndOfLinePreference,
    ) -> usize {
        let range = self.validate_range(range);
        let rope = self.buffer().rope();
        let text = rope.text_for_range(
            rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end),
        );
        let breaks = (range.end.row - range.start.row) as usize;
        text.chars().filter(|&c| c != '\n').count() + breaks * self.eol_str(eol).len()
    }

    /// The word containing `position`, by the language's `wordPattern` or
    /// Monaco's default (Monaco `getWordAtPosition`).
    pub fn word_at_position(&self, position: PointUtf16) -> Option<WordAtPosition> {
        let position = self.validate_position(position);
        let pattern = self.language().and_then(|l| l.config.word_pattern.clone());
        let regex = match pattern.as_ref().and_then(syntax::Pattern::regex) {
            Some(regex) => regex,
            None => cursor::in_place_replace::default_word_regex(),
        };
        let line = self.line_content(position.row);
        let columns = cursor::in_place_replace::word_at(&line, position.column, regex)?;
        let range = PointUtf16::new(position.row, columns.start)
            ..PointUtf16::new(position.row, columns.end);
        Some(WordAtPosition {
            word: self.value_in_range(range.clone(), EndOfLinePreference::Lf),
            range,
        })
    }

    /// The part of the word at `position` before it (Monaco
    /// `getWordUntilPosition`); empty, at `position`, outside words.
    pub fn word_until_position(&self, position: PointUtf16) -> WordAtPosition {
        let position = self.validate_position(position);
        match self.word_at_position(position) {
            Some(word) => {
                let range = word.range.start..position;
                WordAtPosition {
                    word: self.value_in_range(range.clone(), EndOfLinePreference::Lf),
                    range,
                }
            }
            None => WordAtPosition {
                word: String::new(),
                range: position..position,
            },
        }
    }

    /// Searches the text (Monaco `findMatches`): `whole_word` uses Monaco's
    /// usual word separators. Returns at most `limit` ranges; an invalid
    /// regex finds nothing.
    pub fn find_matches(
        &self,
        query: &str,
        is_regex: bool,
        match_case: bool,
        whole_word: bool,
        limit: usize,
    ) -> Vec<Range<PointUtf16>> {
        let params = SearchParams::new(
            query,
            is_regex,
            match_case,
            whole_word.then_some(cursor::USUAL_WORD_SEPARATORS),
        );
        let Some(data) = params.parse() else {
            return Vec::new();
        };
        let rope = self.buffer().rope();
        search::find_matches(rope, &data, 0..rope.len(), limit)
            .into_iter()
            .map(|m| rope.offset_to_point_utf16(m.start)..rope.offset_to_point_utf16(m.end))
            .collect()
    }

    /// Each line's text, without line breaks (Monaco `getLinesContent`).
    pub fn lines_content(&self) -> Vec<String> {
        let rope = self.buffer().rope();
        (0..rope.line_count()).map(|row| rope.line(row)).collect()
    }

    /// The text's length in UTF-16 units, line breaks counted as the
    /// model's line ending (Monaco `getValueLength`).
    pub fn value_length(&self) -> usize {
        let rope = self.buffer().rope();
        let breaks = rope.line_count() as usize - 1;
        rope.len_utf16().0 + breaks * (self.line_ending().as_str().len() - 1)
    }

    /// Monaco's offset of `position` (Monaco `getOffsetAt`): UTF-16 code
    /// units from the start of the text, each line break counted as the
    /// model's line ending (two for CRLF). [`TextModel::modify_position`],
    /// [`TextModel::value_length`] and
    /// [`ContentChange::range_offset_utf16`](crate::ContentChange) count
    /// the same way; [`TextModel::offset_at`] is the byte offset instead.
    pub fn get_offset_at(&self, position: PointUtf16) -> usize {
        let rope = self.buffer().rope();
        let position = rope.clip_point_utf16(position, Bias::Left);
        let offset = rope.point_utf16_to_offset(position);
        crate::events::utf16_offset(rope, offset, self.line_ending())
    }

    /// The position at Monaco offset `offset` (Monaco `getPositionAt`; see
    /// [`TextModel::get_offset_at`]), clamped to the text. An offset inside
    /// a CRLF line break or a surrogate pair gives the position before it.
    pub fn get_position_at(&self, offset: usize) -> PointUtf16 {
        let rope = self.buffer().rope();
        let extra = self.line_ending().as_str().len() - 1;
        let offset = offset.min(self.value_length());
        // At most this row; step back while the row starts after `offset`.
        let mut row = rope
            .offset_to_point(
                rope.offset_utf16_to_offset(text::OffsetUtf16(offset.min(rope.len_utf16().0))),
            )
            .row;
        while row > 0 && self.get_offset_at(PointUtf16::new(row, 0)) > offset {
            row -= 1;
        }
        let start = rope
            .offset_to_offset_utf16(rope.point_to_offset(text::Point::new(row, 0)))
            .0
            + row as usize * extra;
        let column = ((offset - start) as u32).min(rope.line_len_utf16(row));
        rope.clip_point_utf16(PointUtf16::new(row, column), Bias::Left)
    }

    /// The position `offset` UTF-16 units from `position`, clamped to the
    /// text (Monaco `modifyPosition`; line breaks count as the model's line
    /// ending).
    pub fn modify_position(&self, position: PointUtf16, offset: isize) -> PointUtf16 {
        let start = self.get_offset_at(position) as isize;
        let target = (start + offset).clamp(0, self.value_length() as isize);
        self.get_position_at(target as usize)
    }

    /// The whole text's range (Monaco `getFullModelRange`).
    pub fn full_model_range(&self) -> Range<PointUtf16> {
        PointUtf16::new(0, 0)..self.buffer().rope().max_point_utf16()
    }

    fn search_data(
        query: &str,
        is_regex: bool,
        match_case: bool,
        word_separators: Option<&str>,
    ) -> Option<search::SearchData> {
        SearchParams::new(query, is_regex, match_case, word_separators).parse()
    }

    fn find_match(
        &self,
        data: &search::SearchData,
        range: Range<usize>,
        capture_matches: bool,
    ) -> FindMatch {
        let rope = self.buffer().rope();
        let matches = capture_matches
            .then(|| {
                let (haystack, start) = search::match_context(rope, data, &range);
                search::captures(data, &haystack, range.start - start..range.end - start)
            })
            .flatten();
        FindMatch {
            range: rope.offset_to_point_utf16(range.start)..rope.offset_to_point_utf16(range.end),
            matches,
        }
    }

    /// Searches only within `scope` (Monaco `findMatches` with a search
    /// scope): overlapping ranges are merged first, but touching ones stay
    /// apart (Monaco `Range.areIntersecting`), so no match spans two. With
    /// `capture_matches`, each result has the regex's captures, whole match
    /// first. `word_separators` makes it whole-word. At most `limit`
    /// results; an invalid regex finds nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn find_matches_in(
        &self,
        query: &str,
        scope: &[Range<PointUtf16>],
        is_regex: bool,
        match_case: bool,
        word_separators: Option<&str>,
        capture_matches: bool,
        limit: usize,
    ) -> Vec<FindMatch> {
        let Some(data) = Self::search_data(query, is_regex, match_case, word_separators) else {
            return Vec::new();
        };
        let rope = self.buffer().rope();
        let mut ranges: Vec<Range<usize>> = scope
            .iter()
            .map(|range| {
                let range = self.validate_range(range.clone());
                rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end)
            })
            .collect();
        ranges.sort_by_key(|range| (range.start, range.end));
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start < last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        let mut result = Vec::new();
        for range in merged {
            if result.len() >= limit {
                break;
            }
            for m in search::find_matches(rope, &data, range, limit - result.len()) {
                result.push(self.find_match(&data, m, capture_matches));
            }
        }
        result
    }

    /// The first match at or after `search_start`, wrapping around (Monaco
    /// `findNextMatch`).
    pub fn find_next_match(
        &self,
        query: &str,
        search_start: PointUtf16,
        is_regex: bool,
        match_case: bool,
        word_separators: Option<&str>,
        capture_matches: bool,
    ) -> Option<FindMatch> {
        let data = Self::search_data(query, is_regex, match_case, word_separators)?;
        let rope = self.buffer().rope();
        let from = self.offset_at(search_start);
        let found = search::find_next(rope, &data, from)?;
        Some(self.find_match(&data, found, capture_matches))
    }

    /// The last match ending at or before `search_start`, wrapping around
    /// (Monaco `findPreviousMatch`).
    pub fn find_previous_match(
        &self,
        query: &str,
        search_start: PointUtf16,
        is_regex: bool,
        match_case: bool,
        word_separators: Option<&str>,
        capture_matches: bool,
    ) -> Option<FindMatch> {
        let data = Self::search_data(query, is_regex, match_case, word_separators)?;
        let rope = self.buffer().rope();
        let before = self.offset_at(search_start);
        let found = search::find_previous(rope, &data, before)?;
        Some(self.find_match(&data, found, capture_matches))
    }
}

/// A closed bracket pair (Monaco `BracketPairInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketPairInfo {
    /// The opening bracket.
    pub open: Range<PointUtf16>,
    /// The closing bracket.
    pub close: Range<PointUtf16>,
    /// Nesting level, 0 for top-level pairs.
    pub level: u32,
}

/// Monaco's `model.bracketPairs` and `model.guides`. Brackets in strings
/// and comments are ignored. Unlike Monaco's incremental bracket tree, the
/// pairs are found on each call, so these suit occasional queries.
impl TextModel {
    fn guide_context(&self) -> crate::view::guides::GuideContext {
        crate::view::guides::GuideContext {
            tab_size: self.options().tab_size,
            indent_size: self.options().indent_size,
            off_side: self
                .language()
                .and_then(|l| l.config.folding.as_ref())
                .is_some_and(|f| f.off_side),
        }
    }

    /// Every closed bracket pair, sorted by opening bracket.
    fn all_bracket_pairs(&self) -> Vec<BracketPairInfo> {
        let Some(language) = self.language() else {
            return Vec::new();
        };
        let config = &language.config;
        let pairs = config
            .colorized_bracket_pairs
            .clone()
            .unwrap_or_else(|| config.brackets().to_vec());
        let rope = self.buffer().rope();
        let skip = self.string_and_comment_ranges(0..rope.len());
        let (_, pairs) =
            crate::view::bracket_pairs::bracket_pairs(rope, &pairs, &skip, self.options().tab_size);
        pairs
            .into_iter()
            .map(|pair| BracketPairInfo {
                open: rope.offset_to_point_utf16(pair.open.start)
                    ..rope.offset_to_point_utf16(pair.open.end),
                close: rope.offset_to_point_utf16(pair.close.start)
                    ..rope.offset_to_point_utf16(pair.close.end),
                level: pair.level,
            })
            .collect()
    }

    /// The bracket pairs intersecting `range` (Monaco
    /// `bracketPairs.getBracketPairsInRange`).
    pub fn bracket_pairs_in_range(&self, range: Range<PointUtf16>) -> Vec<BracketPairInfo> {
        let range = self.validate_range(range);
        self.all_bracket_pairs()
            .into_iter()
            .filter(|pair| pair.open.start <= range.end && range.start <= pair.close.end)
            .collect()
    }

    /// The bracket touching `position` and its partner, as (open, close)
    /// (Monaco `bracketPairs.matchBracket`).
    pub fn match_bracket(
        &self,
        position: PointUtf16,
    ) -> Option<(Range<PointUtf16>, Range<PointUtf16>)> {
        let position = self.validate_position(position);
        let touches = |r: &Range<PointUtf16>| r.start <= position && position <= r.end;
        // The bracket after the position wins, as in Monaco.
        let pairs = self.all_bracket_pairs();
        pairs
            .iter()
            .find(|pair| pair.open.start == position || pair.close.start == position)
            .or_else(|| {
                pairs
                    .iter()
                    .find(|pair| touches(&pair.open) || touches(&pair.close))
            })
            .map(|pair| (pair.open.clone(), pair.close.clone()))
    }

    /// The innermost pair around `position` (Monaco
    /// `bracketPairs.findEnclosingBrackets`).
    pub fn find_enclosing_brackets(
        &self,
        position: PointUtf16,
    ) -> Option<(Range<PointUtf16>, Range<PointUtf16>)> {
        let position = self.validate_position(position);
        self.all_bracket_pairs()
            .into_iter()
            .filter(|pair| pair.open.end <= position && position <= pair.close.start)
            .max_by_key(|pair| pair.level)
            .map(|pair| (pair.open, pair.close))
    }

    /// `s` with its leading whitespace converted to the model's indentation
    /// (Monaco `normalizeIndentation`): tabs and spaces measured with
    /// `indent_size`, then written as tabs and spaces without
    /// `insert_spaces`, or as spaces with it.
    pub fn normalize_indentation(&self, s: &str) -> String {
        let options = self.options();
        cursor::strings::normalize_indentation(s, options.indent_size.max(1), options.insert_spaces)
    }

    /// The column of `row`'s first character that isn't a space or a tab
    /// (Monaco `getLineFirstNonWhitespaceColumn`, 0-based), or `None` when
    /// the line is only whitespace (Monaco's `0`).
    pub fn line_first_non_whitespace_column(&self, row: u32) -> Option<u32> {
        let line = self.line_content(row);
        let index = line.find(|c| c != ' ' && c != '\t')?;
        Some(line[..index].encode_utf16().count() as u32)
    }

    /// The column after `row`'s last character that isn't a space or a tab
    /// (Monaco `getLineLastNonWhitespaceColumn`, 0-based), or `None` when
    /// the line is only whitespace (Monaco's `0`).
    pub fn line_last_non_whitespace_column(&self, row: u32) -> Option<u32> {
        let line = self.line_content(row);
        let end = line.trim_end_matches([' ', '\t']).len();
        (end > 0).then(|| line[..end].encode_utf16().count() as u32)
    }

    /// Where `row`'s leading spaces and tabs end (Monaco
    /// `getLineIndentColumn`, 0-based): each counts as one column.
    pub fn line_indent_column(&self, row: u32) -> u32 {
        let line = self.line_content(row);
        line.chars().take_while(|&c| c == ' ' || c == '\t').count() as u32
    }

    /// The visible width of `row`'s indentation, tabs expanded to
    /// `tab_size` (Monaco `computeIndentLevel`, what indent guides and
    /// indentation folding use), or `None` for a line that's only
    /// whitespace (Monaco's `-1`).
    pub fn line_indent_level(&self, row: u32) -> Option<u32> {
        crate::view::folding::indent_level(&self.line_content(row), self.options().tab_size.max(1))
    }

    /// The number of indent guides on each of `rows` (Monaco
    /// `guides.getLinesIndentGuides`).
    pub fn lines_indent_guides(&self, rows: Range<u32>) -> Vec<u32> {
        let rope = self.buffer().rope();
        let rows = rows.start.min(rope.line_count())..rows.end.min(rope.line_count());
        crate::view::guides::lines_indent_guides(rope, rows, self.guide_context())
    }

    /// The indentation block to highlight for a cursor on `row`, searched
    /// within `min_row..=max_row` (Monaco `guides.getActiveIndentGuide`).
    pub fn active_indent_guide(
        &self,
        row: u32,
        min_row: u32,
        max_row: u32,
    ) -> crate::view::guides::ActiveIndentGuide {
        let rope = self.buffer().rope();
        let last = rope.line_count().saturating_sub(1);
        crate::view::guides::active_indent_guide(
            rope,
            row.min(last),
            min_row.min(last),
            max_row.min(last),
            self.guide_context(),
        )
    }
}

/// A search result (Monaco `FindMatch`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindMatch {
    /// Where it is.
    pub range: Range<PointUtf16>,
    /// With `capture_matches`, the whole match and then each group
    /// (`None` for a group that didn't participate).
    pub matches: Option<Vec<Option<String>>>,
}

/// Monaco `ICursorStateComputer`: the selections after an edit, from the
/// edits that undo it.
pub type CursorStateComputer = dyn Fn(&[(Range<PointUtf16>, String)]) -> Option<Vec<Selection>>;

/// An edit for [`CodeEditor::execute_edit_operations`] and
/// [`TextModel::apply_edit_operations`] (Monaco
/// `IIdentifiedSingleEditOperation`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SingleEditOperation {
    /// The range to replace, in current coordinates.
    pub range: Range<PointUtf16>,
    /// The text to put there.
    pub text: String,
    /// Cursors and decorations at the range's edges, and inside it, move
    /// past the new text whatever their stickiness (Monaco
    /// `forceMoveMarkers`).
    pub force_move_markers: bool,
}

impl SingleEditOperation {
    /// Replaces `range` with `text`.
    pub fn new(range: Range<PointUtf16>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
            force_move_markers: false,
        }
    }

    /// With `force_move_markers` on.
    pub fn force_move_markers(mut self) -> Self {
        self.force_move_markers = true;
        self
    }
}

/// Where [`CodeEditor::execute_edit_operations`] leaves the cursors
/// (Monaco's `endCursorState`).
pub enum EndCursorState {
    /// These selections, in the coordinates after the edits.
    Selections(Vec<Selection>),
    /// What this returns from the edits that undo the change, or (for
    /// `None`) where the cursors followed the edits to.
    Computer(Box<CursorStateComputer>),
}

impl std::fmt::Debug for EndCursorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Selections(selections) => f.debug_tuple("Selections").field(selections).finish(),
            Self::Computer(_) => f.write_str("Computer(..)"),
        }
    }
}

impl TextModel {
    /// Applies `edits` and returns the edits that undo them, in the new
    /// text's coordinates (Monaco `applyEdits` with `computeUndoEdits`).
    /// Unlike Monaco, the edit is also recorded for undo, which keeps undo
    /// history consistent with the text.
    pub fn apply_edits(
        &mut self,
        edits: Vec<(Range<PointUtf16>, String)>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<(Range<PointUtf16>, String)>, text::EditError> {
        let edits = edits
            .into_iter()
            .map(|(range, text)| SingleEditOperation::new(range, text))
            .collect();
        self.apply_edits_in(edits, None, cx)
    }

    /// [`TextModel::apply_edits`] with Monaco's `forceMoveMarkers` per
    /// edit (Monaco `applyEdits(operations)`): decorations, and the cursors
    /// of the editors showing the model, in or at the edges of a forced
    /// edit move past its new text.
    pub fn apply_edit_operations(
        &mut self,
        edits: Vec<SingleEditOperation>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<(Range<PointUtf16>, String)>, text::EditError> {
        self.apply_edits_in(edits, None, cx)
    }

    fn apply_edits_in(
        &mut self,
        edits: Vec<SingleEditOperation>,
        group: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<(Range<PointUtf16>, String)>, text::EditError> {
        let old = self.buffer().rope().clone();
        let mut force = Vec::new();
        let edits: Vec<(Range<usize>, String)> = edits
            .into_iter()
            .map(|edit| {
                let range = self.validate_range(edit.range);
                let range =
                    old.point_utf16_to_offset(range.start)..old.point_utf16_to_offset(range.end);
                if edit.force_move_markers {
                    force.push(range.clone());
                }
                (range, edit.text)
            })
            .collect();
        if !force.is_empty() {
            self.force_move_markers_on_next_edit(force);
        }
        let patch = self.edit_from(edits, group, None, cx);
        self.force_move_markers_on_next_edit(Vec::new());
        let patch = patch?;
        let new = self.buffer().rope();
        Ok(patch
            .edits()
            .iter()
            .map(|edit| {
                (
                    new.offset_to_point_utf16(edit.new.start)
                        ..new.offset_to_point_utf16(edit.new.end),
                    old.text_for_range(edit.old.clone()),
                )
            })
            .collect())
    }

    /// Applies `edits` as part of the current undo step (Monaco
    /// `pushEditOperations`): they join the open step, typing's included,
    /// until [`TextModel::push_stack_element`]. `before_cursor_state` is restored
    /// by undo. `cursor_state_computer` gets the undo edits and returns the
    /// selections after, which redo restores; they're returned.
    pub fn push_edit_operations(
        &mut self,
        before_cursor_state: Option<Vec<Selection>>,
        edits: Vec<(Range<PointUtf16>, String)>,
        cursor_state_computer: Option<&CursorStateComputer>,
        cx: &mut Context<Self>,
    ) -> Result<Option<Vec<Selection>>, text::EditError> {
        let edits = edits
            .into_iter()
            .map(|(range, text)| SingleEditOperation::new(range, text))
            .collect();
        let undo_edits = self.apply_edits_in(edits, Some(text::JOIN_OPEN_UNDO_STEP), cx)?;
        let after = cursor_state_computer.and_then(|compute| compute(&undo_edits));
        let metadata = |selections: Option<Vec<Selection>>| {
            selections.map(|s| Arc::new(s) as text::TransactionMetadata)
        };
        self.set_undo_metadata(metadata(before_cursor_state), metadata(after.clone()));
        Ok(after)
    }

    /// The version, except that undo and redo return to the version the
    /// text had then (Monaco `getAlternativeVersionId`): equal values mean
    /// equal text.
    pub fn alternative_version_id(&self) -> usize {
        self.buffer().alternative_version()
    }
}

impl CodeEditor {
    /// What provider lookups match against: the model's language and uri.
    pub(crate) fn provider_target(&self, cx: &App) -> crate::providers::ProviderTarget {
        self.model().read(cx).into()
    }
}

/// Monaco's `ICodeEditor` reveal methods. They take effect on the next
/// frame and scroll smoothly (Monaco's default `ScrollType.Smooth`); the
/// `*_with_scroll_type` variants take a [`ScrollType`].
impl CodeEditor {
    pub(crate) fn request_reveal(&mut self, request: PendingReveal, cx: &mut Context<Self>) {
        self.pending_reveal = Some(request);
        self.autoscroll_requested = false;
        cx.notify();
    }

    /// Scrolls so `range` shows, horizontally too (Monaco `revealRange`
    /// and friends, which `reveal` picks). A range over several lines
    /// scrolls horizontally to the start, as in Monaco.
    pub fn reveal_range(
        &mut self,
        range: Range<PointUtf16>,
        reveal: RevealType,
        cx: &mut Context<Self>,
    ) {
        self.reveal_range_with_scroll_type(range, reveal, ScrollType::Smooth, cx);
    }

    /// [`CodeEditor::reveal_range`] with a [`ScrollType`] (Monaco
    /// `revealRange*(range, scrollType)`, `revealPosition*(position,
    /// scrollType)` with an empty range).
    pub fn reveal_range_with_scroll_type(
        &mut self,
        range: Range<PointUtf16>,
        reveal: RevealType,
        scroll_type: ScrollType,
        cx: &mut Context<Self>,
    ) {
        self.request_reveal(
            PendingReveal {
                ranges: vec![range],
                cursors: false,
                reveal,
                horizontal: true,
                minimal: false,
                mouse: false,
                scroll_type,
            },
            cx,
        );
    }

    /// Scrolls vertically so rows `start_row..=end_row` show, leaving the
    /// horizontal scroll alone (Monaco `revealLines*(start, end,
    /// scrollType)` and `revealLine*`).
    pub fn reveal_lines_with_scroll_type(
        &mut self,
        start_row: u32,
        end_row: u32,
        reveal: RevealType,
        scroll_type: ScrollType,
        cx: &mut Context<Self>,
    ) {
        self.request_reveal(
            PendingReveal {
                ranges: vec![PointUtf16::new(start_row, 0)..PointUtf16::new(end_row, 0)],
                cursors: false,
                reveal,
                horizontal: false,
                minimal: false,
                mouse: false,
                scroll_type,
            },
            cx,
        );
    }

    /// Scrolls minimally so `row` shows (Monaco `revealLine`).
    pub fn reveal_line(&mut self, row: u32, cx: &mut Context<Self>) {
        self.reveal_lines_with_scroll_type(row, row, RevealType::Simple, ScrollType::Smooth, cx);
    }

    /// Centers `row` (Monaco `revealLineInCenter`).
    pub fn reveal_line_in_center(&mut self, row: u32, cx: &mut Context<Self>) {
        self.reveal_lines_with_scroll_type(row, row, RevealType::Center, ScrollType::Smooth, cx);
    }

    /// Centers `row` unless it's visible (Monaco
    /// `revealLineInCenterIfOutsideViewport`).
    pub fn reveal_line_in_center_if_outside_viewport(&mut self, row: u32, cx: &mut Context<Self>) {
        self.reveal_lines_with_scroll_type(
            row,
            row,
            RevealType::CenterIfOutsideViewport,
            ScrollType::Smooth,
            cx,
        );
    }

    /// Puts `row` near the top (Monaco `revealLineNearTop`).
    pub fn reveal_line_near_top(&mut self, row: u32, cx: &mut Context<Self>) {
        self.reveal_lines_with_scroll_type(row, row, RevealType::NearTop, ScrollType::Smooth, cx);
    }

    /// Scrolls minimally so rows `start_row..=end_row` show (Monaco
    /// `revealLines`).
    pub fn reveal_lines(&mut self, start_row: u32, end_row: u32, cx: &mut Context<Self>) {
        self.reveal_lines_with_scroll_type(
            start_row,
            end_row,
            RevealType::Simple,
            ScrollType::Smooth,
            cx,
        );
    }

    /// Centers rows `start_row..=end_row` (Monaco `revealLinesInCenter`).
    pub fn reveal_lines_in_center(&mut self, start_row: u32, end_row: u32, cx: &mut Context<Self>) {
        self.reveal_lines_with_scroll_type(
            start_row,
            end_row,
            RevealType::Center,
            ScrollType::Smooth,
            cx,
        );
    }

    /// Centers rows `start_row..=end_row` unless they're visible (Monaco
    /// `revealLinesInCenterIfOutsideViewport`).
    pub fn reveal_lines_in_center_if_outside_viewport(
        &mut self,
        start_row: u32,
        end_row: u32,
        cx: &mut Context<Self>,
    ) {
        self.reveal_lines_with_scroll_type(
            start_row,
            end_row,
            RevealType::CenterIfOutsideViewport,
            ScrollType::Smooth,
            cx,
        );
    }

    /// Puts rows `start_row..=end_row` near the top (Monaco
    /// `revealLinesNearTop`).
    pub fn reveal_lines_near_top(&mut self, start_row: u32, end_row: u32, cx: &mut Context<Self>) {
        self.reveal_lines_with_scroll_type(
            start_row,
            end_row,
            RevealType::NearTop,
            ScrollType::Smooth,
            cx,
        );
    }

    /// Scrolls minimally so `position` shows, horizontally too (Monaco
    /// `revealPosition`).
    pub fn reveal_position(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        self.reveal_range(position..position, RevealType::Simple, cx);
    }

    /// Centers `position` (Monaco `revealPositionInCenter`).
    pub fn reveal_position_in_center(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        self.reveal_range(position..position, RevealType::Center, cx);
    }

    /// Centers `position` unless it's visible (Monaco
    /// `revealPositionInCenterIfOutsideViewport`).
    pub fn reveal_position_in_center_if_outside_viewport(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        self.reveal_range(position..position, RevealType::CenterIfOutsideViewport, cx);
    }

    /// Puts `position` near the top (Monaco `revealPositionNearTop`).
    pub fn reveal_position_near_top(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        self.reveal_range(position..position, RevealType::NearTop, cx);
    }

    /// Centers `range` (Monaco `revealRangeInCenter`).
    pub fn reveal_range_in_center(&mut self, range: Range<PointUtf16>, cx: &mut Context<Self>) {
        self.reveal_range(range, RevealType::Center, cx);
    }

    /// Centers `range` unless it's visible (Monaco
    /// `revealRangeInCenterIfOutsideViewport`).
    pub fn reveal_range_in_center_if_outside_viewport(
        &mut self,
        range: Range<PointUtf16>,
        cx: &mut Context<Self>,
    ) {
        self.reveal_range(range, RevealType::CenterIfOutsideViewport, cx);
    }

    /// Puts `range` at the top (Monaco `revealRangeAtTop`).
    pub fn reveal_range_at_top(&mut self, range: Range<PointUtf16>, cx: &mut Context<Self>) {
        self.reveal_range(range, RevealType::Top, cx);
    }

    /// Puts `range` near the top (Monaco `revealRangeNearTop`).
    pub fn reveal_range_near_top(&mut self, range: Range<PointUtf16>, cx: &mut Context<Self>) {
        self.reveal_range(range, RevealType::NearTop, cx);
    }

    /// Puts `range` near the top unless it's visible (Monaco
    /// `revealRangeNearTopIfOutsideViewport`).
    pub fn reveal_range_near_top_if_outside_viewport(
        &mut self,
        range: Range<PointUtf16>,
        cx: &mut Context<Self>,
    ) {
        self.reveal_range(range, RevealType::NearTopIfOutsideViewport, cx);
    }

    /// Scrolls so every cursor shows (Monaco `revealAllCursors`): a single
    /// cursor's position, or the lines from the first selection to the last
    /// (not at all when they don't fit). `reveal_horizontal` also scrolls
    /// horizontally to the cursors on their lines; `minimal_reveal` keeps
    /// no context lines around them.
    pub fn reveal_all_cursors(
        &mut self,
        reveal_horizontal: bool,
        minimal_reveal: bool,
        cx: &mut Context<Self>,
    ) {
        let selections = self.cursors.selections();
        let cursors = selections.len() > 1;
        let ranges = if cursors {
            selections.iter().map(|s| s.start()..s.end()).collect()
        } else {
            vec![selections[0].head..selections[0].head]
        };
        self.request_reveal(
            PendingReveal {
                ranges,
                cursors,
                reveal: RevealType::Simple,
                horizontal: reveal_horizontal,
                minimal: minimal_reveal,
                mouse: false,
                scroll_type: ScrollType::Smooth,
            },
            cx,
        );
    }

    /// Scrolls so the primary selection shows (Monaco `revealPrimaryCursor`
    /// with `revealHorizontal`): not at all when it doesn't fit.
    pub(crate) fn reveal_primary_cursor(&mut self, cx: &mut Context<Self>) {
        let selection = self.cursors.primary().selection();
        self.request_reveal(
            PendingReveal {
                ranges: vec![selection.start()..selection.end()],
                cursors: true,
                reveal: RevealType::Simple,
                horizontal: true,
                minimal: false,
                mouse: false,
                scroll_type: ScrollType::Smooth,
            },
            cx,
        );
    }

    /// Scrolls so the top-most (`top`) or bottom-most cursor shows (Monaco
    /// `revealTopMostCursor` / `revealBottomMostCursor`), as Add Cursor
    /// Above/Below and column selection do. `mouse` is Monaco's `source ===
    /// 'mouse'`.
    pub(crate) fn reveal_edge_cursor(&mut self, top: bool, mouse: bool, cx: &mut Context<Self>) {
        let positions = self.cursors.all().iter().map(|cursor| cursor.position);
        let position = if top {
            positions.min()
        } else {
            positions.max()
        };
        let Some(position) = position else {
            return;
        };
        self.request_reveal(
            PendingReveal {
                ranges: vec![position..position],
                cursors: false,
                reveal: RevealType::Simple,
                horizontal: true,
                minimal: false,
                mouse,
                scroll_type: ScrollType::Smooth,
            },
            cx,
        );
    }

    /// The model ranges on screen, split around folded regions (Monaco
    /// `getVisibleRanges`), as of the last frame.
    pub fn visible_ranges(&self, cx: &App) -> Vec<Range<PointUtf16>> {
        let Some(layout) = self.layout.as_ref() else {
            return Vec::new();
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let mut ranges: Vec<Range<PointUtf16>> = Vec::new();
        for line in &layout.lines {
            let end = PointUtf16::new(line.row, rope.line_len_utf16(line.row));
            match ranges.last_mut() {
                Some(last) if last.end.row == line.row || last.end.row + 1 == line.row => {
                    last.end = end;
                }
                _ => ranges.push(PointUtf16::new(line.row, 0)..end),
            }
        }
        ranges
    }

    /// [`CodeEditor::visible_ranges`] grown by a viewport's worth of lines
    /// above and below (Monaco `getVisibleRangesPlusViewportAboveBelow`),
    /// e.g. to ask providers for what scrolling will show next.
    pub fn visible_ranges_plus_viewport_above_below(&self, cx: &App) -> Vec<Range<PointUtf16>> {
        let visible = self.visible_ranges(cx);
        let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
            return Vec::new();
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let page = self.layout.as_ref().map_or(0, |l| l.page_rows());
        let start = first.start.row.saturating_sub(page);
        let end = (last.end.row + page).min(rope.line_count() - 1);
        let mut ranges: Vec<Range<PointUtf16>> = Vec::new();
        for row in start..=end {
            if self.view_lines.is_row_hidden(row) {
                continue;
            }
            let line_end = PointUtf16::new(row, rope.line_len_utf16(row));
            match ranges.last_mut() {
                Some(range) if range.end.row + 1 == row => range.end = line_end,
                _ => ranges.push(PointUtf16::new(row, 0)..line_end),
            }
        }
        ranges
    }

    /// Whether the editor has keyboard focus (Monaco `hasTextFocus`).
    pub fn has_text_focus(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Whether the editor or a widget in it, such as the find widget, has
    /// keyboard focus (Monaco `hasWidgetFocus`).
    pub fn has_widget_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }

    /// Applies `edits` (in current coordinates), then sets
    /// `end_selections`, or lets the cursors follow the edits (Monaco
    /// `executeEdits`). See [`CodeEditor::execute_edit_operations`].
    pub fn execute_edits(
        &mut self,
        edits: Vec<(Range<PointUtf16>, String)>,
        end_selections: Option<Vec<Selection>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let edits = edits
            .into_iter()
            .map(|(range, text)| SingleEditOperation::new(range, text))
            .collect();
        self.execute_edit_operations(edits, end_selections.map(EndCursorState::Selections), cx)
    }

    /// Applies `edits` (in current coordinates) and sets the cursors from
    /// `end_cursor_state` (Monaco `executeEdits(source, edits,
    /// endCursorState)`): given selections, selections a
    /// [`CursorStateComputer`] returns from the undo edits, or else (and
    /// when the computer returns `None`) the cursors follow the edits, an
    /// edit with `force_move_markers` moving the cursors at its edges past
    /// its text.
    ///
    /// As in Monaco, the edits join the open undo step (typing's too) until
    /// [`CodeEditor::push_undo_stop`], and a read-only editor returns
    /// `false` without saying anything (its read-only message is only for
    /// the user's own edits).
    pub fn execute_edit_operations(
        &mut self,
        edits: Vec<SingleEditOperation>,
        end_cursor_state: Option<EndCursorState>,
        cx: &mut Context<Self>,
    ) -> bool {
        // Monaco returns `false` without a model, or when read-only.
        if self.options.read_only || self.detached {
            return false;
        }
        let selections = self.cursors.selections();
        let old = self.snapshot.clone();
        let (end_selections, computer) = match end_cursor_state {
            Some(EndCursorState::Selections(selections)) => (Some(selections), None),
            Some(EndCursorState::Computer(computer)) => (None, Some(computer)),
            None => (None, None),
        };
        let force: Vec<Range<usize>> = edits
            .iter()
            .filter(|edit| edit.force_move_markers)
            .map(|edit| {
                let range = old.rope().clip_range_utf16(edit.range.clone());
                old.point_utf16_to_offset(range.start)..old.point_utf16_to_offset(range.end)
            })
            .collect();
        let edits: Vec<(Range<PointUtf16>, String, bool)> = edits
            .into_iter()
            .map(|edit| (edit.range, edit.text, edit.force_move_markers))
            .collect();
        let mut operation =
            cursor::execute_edits_forcing(old.rope(), &selections, edits, end_selections);
        // Monaco's `executeEdits` pushes no undo stops.
        operation.push_stack_element_before = false;
        operation.push_stack_element_after = false;
        let applied_edits = operation.edits.clone();
        if !force.is_empty() {
            self.model()
                .update(cx, |model, _| model.force_move_markers_on_next_edit(force));
        }
        self.next_selection_change.source = crate::events::CursorChangeSource::Api;
        self.edit_group = Some(text::JOIN_OPEN_UNDO_STEP);
        self.begin_update(cx);
        let ranges = self.apply(operation, cx);
        self.edit_group = None;
        self.model().update(cx, |model, _| {
            model.force_move_markers_on_next_edit(Vec::new())
        });
        if let Some(computer) = computer
            && ranges.len() == applied_edits.len()
        {
            let new = self.snapshot.rope();
            let undo_edits: Vec<(Range<PointUtf16>, String)> = applied_edits
                .iter()
                .zip(&ranges)
                .map(|((range, _), new_range)| {
                    (
                        new.offset_to_point_utf16(new_range.start)
                            ..new.offset_to_point_utf16(new_range.end),
                        old.text_for_range(
                            old.point_utf16_to_offset(range.start)
                                ..old.point_utf16_to_offset(range.end),
                        ),
                    )
                })
                .collect();
            if let Some(selections) = computer(&undo_edits).filter(|s| !s.is_empty()) {
                self.cursors.set_selections(&selections);
                self.cursors.validate(self.snapshot.rope());
                let after = self.cursors.selections();
                self.model().update(cx, |model, _| {
                    model.set_undo_metadata(None, Some(crate::model::metadata(after)))
                });
                self.next_selection_change.source = crate::events::CursorChangeSource::Api;
                self.cursors_changed(true, cx);
            }
        }
        self.end_update(cx);
        self.next_selection_change = Default::default();
        true
    }

    /// Shows `banner` above the text, `height` tall, pushing the text down
    /// (Monaco `setBanner`), or removes it with `None`. The unicode
    /// highlighter's "too many characters" banner uses it too; there's one
    /// at a time.
    pub fn set_banner(
        &mut self,
        banner: Option<gpui::AnyView>,
        height: gpui::Pixels,
        cx: &mut Context<Self>,
    ) {
        self.banner = banner.map(|banner| (banner, height));
        cx.notify();
    }

    /// Whether a banner shows (see [`CodeEditor::set_banner`]).
    pub fn has_banner(&self) -> bool {
        self.banner.is_some()
    }

    /// Ends the current undo group (Monaco `pushUndoStop`).
    pub fn push_undo_stop(&mut self, cx: &mut Context<Self>) {
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        self.prev_edit_kind = None;
    }

    /// Reopens the last undo step, so the next edits join it until
    /// [`CodeEditor::push_undo_stop`] (Monaco `popUndoStop`).
    pub fn pop_undo_stop(&mut self, cx: &mut Context<Self>) {
        self.model()
            .update(cx, |model, _| model.pop_stack_element());
    }

    /// Runs the editor action or command with Monaco id `name` (e.g.
    /// `"editor.action.commentLine"`, `"cursorDown"`, `"type"`, an alias
    /// such as `"editor.action.goToDeclaration"`, one from
    /// [`CodeEditor::add_action`] or [`CodeEditor::add_command`], or a
    /// command from [`crate::register_command`]), or else the GPUI action
    /// named `name` (e.g. `"code_editor::CommentLine"`), as if its key were
    /// pressed in this editor (Monaco `trigger`). A built-in action or
    /// command whose precondition fails does nothing, as in Monaco. Fails
    /// for unknown names.
    pub fn trigger(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.trigger_command(name, None, window, cx)
    }

    /// [`CodeEditor::trigger`] with Monaco's `payload`: the arguments of
    /// commands that take some, as JSON in Monaco's shape, such as
    /// `editor.trigger('keyboard', 'type', { text })`
    /// ([`Type`](crate::actions::Type)), `cursorMove`
    /// ([`CursorMove`](crate::actions::CursorMove)), `editor.fold`
    /// ([`Fold`](crate::actions::Fold)), `editor.actions.findWithArgs`
    /// ([`FindWithArgs`](crate::actions::FindWithArgs)) and
    /// `editor.action.codeAction`
    /// ([`TriggerCodeAction`](crate::actions::TriggerCodeAction)). Actions
    /// added with [`CodeEditor::add_action`] run without it; registered
    /// commands get it as their argument. Fails for unknown names and
    /// payloads the action can't be built from.
    pub fn trigger_with_payload(
        &mut self,
        name: &str,
        payload: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.trigger_command(name, Some(payload), window, cx)
    }

    fn trigger_command(
        &mut self,
        name: &str,
        payload: Option<serde_json::Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        // Monaco's `trigger` does nothing without a model.
        if self.detached {
            return Ok(());
        }
        let added = self.added_actions.iter().any(|a| a.id.as_ref() == name)
            || crate::editor_actions::global_actions(cx)
                .iter()
                .any(|a| a.id.as_ref() == name);
        if added {
            self.run_action(name, window, cx);
            return Ok(());
        }
        if let Some(handler) = self.commands.handlers.get(name).cloned() {
            handler(self, window, cx);
            return Ok(());
        }
        let action = match crate::editor_actions::action_name(name) {
            Some((action, when)) => {
                if !self.when(when, cx) {
                    return Ok(());
                }
                cx.build_action(action, payload)?
            }
            None if crate::commands::has_command(cx, name) => {
                let command = crate::providers::Command {
                    id: name.to_string().into(),
                    arguments: payload.into_iter().collect(),
                    ..Default::default()
                };
                return crate::commands::execute_command(cx, &command);
            }
            None => cx.build_action(name, payload)?,
        };
        window.focus(&self.focus_handle, cx);
        window.dispatch_action(action, cx);
        Ok(())
    }
}

/// What a [`DecorationsCollection`] decorates.
#[derive(Clone, Debug)]
enum CollectionTarget {
    /// Always this model (the editor's own collections, e.g. a peek
    /// view's, which outlive a model change).
    Model,
    /// Whichever model the editor shows, as Monaco's collections do: the
    /// editor and its shared current model (readable while the editor is
    /// being updated).
    Editor(WeakEntity<CodeEditor>, SharedModel),
}

/// An editor's current model, shared with its decorations collections.
pub(crate) type SharedModel = std::rc::Rc<std::cell::RefCell<WeakEntity<TextModel>>>;

/// A set of decorations an editor owns, replaced as a whole (Monaco
/// `IEditorDecorationsCollection`, from
/// [`CodeEditor::create_decorations_collection`]).
///
/// Like Monaco's, it acts on the model the editor shows at the time: after
/// [`CodeEditor::set_model`] (which removes the editor's decorations from
/// the old model, as Monaco does), the next [`DecorationsCollection::set`]
/// decorates the new model. Dropping it leaves the decorations in place;
/// call [`DecorationsCollection::clear`] first to remove them.
#[derive(Clone, Debug)]
pub struct DecorationsCollection {
    target: CollectionTarget,
    /// The model `ids` are on.
    model: Entity<TextModel>,
    owner: EntityId,
    ids: Vec<DecorationId>,
    /// For each `on_did_change` listener, the collection's own changes
    /// whose `ModelDecorationsChanged` it hasn't seen yet, which it skips
    /// (Monaco `_isChangingDecorations`).
    own_changes: OwnChanges,
}

type OwnChanges = std::rc::Rc<std::cell::RefCell<Vec<std::rc::Weak<std::cell::Cell<usize>>>>>;

impl DecorationsCollection {
    /// An empty collection on `model` whose decorations only `owner` shows.
    pub(crate) fn for_owner(model: Entity<TextModel>, owner: EntityId) -> Self {
        Self {
            target: CollectionTarget::Model,
            model,
            owner,
            ids: Vec::new(),
            own_changes: Default::default(),
        }
    }

    /// The model the collection acts on now: the editor's, for an editor's
    /// collection.
    fn current_model(&self) -> Entity<TextModel> {
        match &self.target {
            CollectionTarget::Editor(_, model) => model
                .borrow()
                .upgrade()
                .unwrap_or_else(|| self.model.clone()),
            CollectionTarget::Model => self.model.clone(),
        }
    }

    /// Follows the editor to its current model; the ids on the old one are
    /// gone with the editor's other decorations there.
    fn follow_model(&mut self) {
        let model = self.current_model();
        if model != self.model {
            self.model = model;
            self.ids.clear();
        }
    }

    /// Replaces the decorations with `decorations` (positions in the
    /// current text), returning their ids (Monaco `set`).
    pub fn set(
        &mut self,
        decorations: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut App,
    ) -> &[DecorationId] {
        self.follow_model();
        let old = std::mem::take(&mut self.ids);
        self.ids = self.delta(&old, decorations, cx);
        &self.ids
    }

    /// Adds `decorations` to the collection, returning their ids (Monaco
    /// `append`).
    pub fn append(
        &mut self,
        decorations: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut App,
    ) -> Vec<DecorationId> {
        self.follow_model();
        let ids = self.delta(&[], decorations, cx);
        self.ids.extend(&ids);
        ids
    }

    /// Removes the decorations `remove` and adds `add`, keeping the rest
    /// (Monaco `deltaDecorations` on some of the collection's ids). Returns
    /// the new ids.
    pub(crate) fn replace(
        &mut self,
        remove: &[DecorationId],
        add: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut App,
    ) -> Vec<DecorationId> {
        self.follow_model();
        if remove.is_empty() && add.is_empty() {
            return Vec::new();
        }
        let removed: std::collections::HashSet<DecorationId> = remove.iter().copied().collect();
        self.ids.retain(|id| !removed.contains(id));
        let ids = self.delta(remove, add, cx);
        self.ids.extend(&ids);
        ids
    }

    /// Removes every decoration (Monaco `clear`).
    pub fn clear(&mut self, cx: &mut App) {
        self.follow_model();
        if !self.ids.is_empty() {
            let old = std::mem::take(&mut self.ids);
            self.delta(&old, Vec::new(), cx);
        }
    }

    /// The number of decorations (Monaco `length`).
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The decorations' ids, in the order they were added.
    pub fn ids(&self) -> &[DecorationId] {
        &self.ids
    }

    /// Whether `id` is in the collection (Monaco `has`).
    pub fn has(&self, id: DecorationId) -> bool {
        self.ids.contains(&id)
    }

    /// Where decoration `index` is now, after edits (Monaco `getRange`);
    /// `None` once the editor shows another model.
    pub fn range(&self, index: usize, cx: &App) -> Option<Range<PointUtf16>> {
        if self.current_model() != self.model {
            return None;
        }
        self.model.read(cx).decoration_range(*self.ids.get(index)?)
    }

    /// Where every decoration is now (Monaco `getRanges`).
    pub fn ranges(&self, cx: &App) -> Vec<Range<PointUtf16>> {
        (0..self.ids.len())
            .filter_map(|index| self.range(index, cx))
            .collect()
    }

    /// Calls `callback` when the decorations of the editor's model change,
    /// except by this collection (Monaco `onDidChange`). `None` for a
    /// collection that isn't an editor's, or once the editor is gone.
    pub fn on_did_change(
        &self,
        mut callback: impl FnMut(&DecorationsChange, &mut App) + 'static,
        cx: &mut App,
    ) -> Option<gpui::Subscription> {
        let CollectionTarget::Editor(editor, _) = &self.target else {
            return None;
        };
        let editor = editor.upgrade()?;
        let pending = std::rc::Rc::new(std::cell::Cell::new(0));
        self.own_changes
            .borrow_mut()
            .push(std::rc::Rc::downgrade(&pending));
        Some(
            cx.subscribe(&editor, move |_, event: &CodeEditorEvent, cx| {
                if let CodeEditorEvent::ModelDecorationsChanged(change) = event {
                    match pending.get() {
                        0 => callback(change, cx),
                        own => pending.set(own - 1),
                    }
                }
            }),
        )
    }

    fn delta(
        &self,
        old: &[DecorationId],
        new: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut App,
    ) -> Vec<DecorationId> {
        let owner = self.owner;
        // The editor forwards the model's event if it shows the model.
        if let CollectionTarget::Editor(editor, _) = &self.target
            && editor.upgrade().is_some()
            && self.current_model() == self.model
        {
            self.own_changes
                .borrow_mut()
                .retain(|pending| match pending.upgrade() {
                    Some(pending) => {
                        pending.set(pending.get() + 1);
                        true
                    }
                    None => false,
                });
        }
        self.model.update(cx, |model, cx| {
            let new = new
                .into_iter()
                .map(|(range, options)| {
                    let range = model.validate_range(range);
                    (
                        model.offset_at(range.start)..model.offset_at(range.end),
                        options,
                    )
                })
                .collect();
            model.delta_decorations(Some(owner), old, new, cx)
        })
    }
}

impl CodeEditor {
    /// A [`DecorationsCollection`] owned by this editor, on whichever model
    /// it shows, starting with `decorations` (Monaco
    /// `createDecorationsCollection`).
    pub fn create_decorations_collection(
        &self,
        decorations: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut Context<Self>,
    ) -> DecorationsCollection {
        let mut collection = DecorationsCollection {
            target: CollectionTarget::Editor(cx.weak_entity(), self.shared_model.clone()),
            model: self.model().clone(),
            owner: cx.entity_id(),
            ids: Vec::new(),
            own_changes: Default::default(),
        };
        if !decorations.is_empty() {
            collection.set(decorations, cx);
        }
        collection
    }
}

/// A decoration with its range in positions (Monaco `IModelDecoration`).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDecoration {
    /// Its id (Monaco `id`).
    pub id: DecorationId,
    /// The editor it's shown in, `None` for every editor (Monaco
    /// `ownerId`).
    pub owner: Option<EntityId>,
    /// Where it is now (0-based rows, UTF-16 columns; Monaco `range`).
    pub range: Range<PointUtf16>,
    /// How it's drawn (Monaco `options`).
    pub options: Arc<DecorationOptions>,
}

/// Monaco's `ITextModel` decoration getters, in positions (0-based rows,
/// UTF-16 columns), next to the byte-offset
/// [`TextModel::decorations_in_range`]. `owner` is Monaco's `ownerId`:
/// `Some(editor)` leaves out decorations only other editors show, `None`
/// keeps every decoration. `filter_out_validation` leaves out diagnostics'
/// (Monaco `filterOutValidation`).
impl TextModel {
    fn model_decoration(&self, decoration: &Decoration) -> ModelDecoration {
        ModelDecoration {
            id: decoration.id,
            owner: decoration.owner,
            range: self.position_at(decoration.range.start)..self.position_at(decoration.range.end),
            options: decoration.options.clone(),
        }
    }

    /// The decorations matching `owner` and `filter_out_validation` that
    /// `keep` accepts, sorted by position.
    fn query_decorations(
        &self,
        owner: Option<EntityId>,
        filter_out_validation: bool,
        keep: impl Fn(&Decoration) -> bool,
    ) -> Vec<ModelDecoration> {
        let mut result: Vec<ModelDecoration> = self
            .decoration_set()
            .iter()
            .filter(|d| owner.is_none() || d.owner.is_none() || d.owner == owner)
            .filter(|d| !(filter_out_validation && d.options.validation))
            .filter(|d| keep(d))
            .map(|d| self.model_decoration(d))
            .collect();
        result.sort_by_key(|d| (d.range.start, d.range.end, d.id));
        result
    }

    /// Where decoration `id` is now (Monaco `getDecorationRange`).
    pub fn decoration_range(&self, id: DecorationId) -> Option<Range<PointUtf16>> {
        let decoration = self.decoration(id)?;
        Some(self.position_at(decoration.range.start)..self.position_at(decoration.range.end))
    }

    /// Decoration `id`'s options (Monaco `getDecorationOptions`).
    pub fn decoration_options(&self, id: DecorationId) -> Option<Arc<DecorationOptions>> {
        Some(self.decoration(id)?.options.clone())
    }

    /// The decorations on rows `start_row..=end_row` (Monaco
    /// `getLinesDecorations`).
    pub fn lines_decorations(
        &self,
        start_row: u32,
        end_row: u32,
        owner: Option<EntityId>,
        filter_out_validation: bool,
    ) -> Vec<ModelDecoration> {
        let end = PointUtf16::new(
            end_row,
            self.line_len_utf16(end_row.min(self.line_count() - 1)),
        );
        self.decorations_in_point_range(
            PointUtf16::new(start_row, 0)..end,
            owner,
            filter_out_validation,
        )
    }

    /// The decorations on `row` (Monaco `getLineDecorations`).
    pub fn line_decorations(
        &self,
        row: u32,
        owner: Option<EntityId>,
        filter_out_validation: bool,
    ) -> Vec<ModelDecoration> {
        self.lines_decorations(row, row, owner, filter_out_validation)
    }

    /// The decorations intersecting `range`, touching ones included (Monaco
    /// `getDecorationsInRange`).
    pub fn decorations_in_point_range(
        &self,
        range: Range<PointUtf16>,
        owner: Option<EntityId>,
        filter_out_validation: bool,
    ) -> Vec<ModelDecoration> {
        let range = self.validate_range(range);
        let (start, end) = (self.offset_at(range.start), self.offset_at(range.end));
        self.query_decorations(owner, filter_out_validation, |d| {
            d.range.start <= end && start <= d.range.end
        })
    }

    /// Every decoration (Monaco `getAllDecorations`).
    pub fn all_model_decorations(
        &self,
        owner: Option<EntityId>,
        filter_out_validation: bool,
    ) -> Vec<ModelDecoration> {
        self.query_decorations(owner, filter_out_validation, |_| true)
    }

    /// The decorations with an overview ruler mark (Monaco
    /// `getOverviewRulerDecorations`): an `overview_ruler` or a marker
    /// `severity`.
    pub fn overview_ruler_decorations(
        &self,
        owner: Option<EntityId>,
        filter_out_validation: bool,
    ) -> Vec<ModelDecoration> {
        self.query_decorations(owner, filter_out_validation, |d| {
            d.options.overview_ruler.is_some() || d.options.severity.is_some()
        })
    }

    /// The decorations that inject text (Monaco
    /// `getInjectedTextDecorations`): `before` or `after`, on an empty range
    /// only with `show_if_collapsed`.
    pub fn injected_text_decorations(&self, owner: Option<EntityId>) -> Vec<ModelDecoration> {
        self.query_decorations(owner, false, |d| {
            (d.options.before.is_some() || d.options.after.is_some())
                && d.options.shows_injected_text(&d.range)
        })
    }

    /// The decorations drawn in the gutter (Monaco
    /// `getAllMarginDecorations`): a glyph, a line number color, a line
    /// decoration or a margin background.
    pub fn all_margin_decorations(&self, owner: Option<EntityId>) -> Vec<ModelDecoration> {
        self.query_decorations(owner, false, |d| {
            let o = &d.options;
            o.glyph.is_some()
                || o.line_number_color.is_some()
                || o.gutter_bar.is_some()
                || o.lines_decoration_text.is_some()
                || o.first_line_decoration.is_some()
                || o.margin_background.is_some()
        })
    }

    /// [`TextModel::delta_decorations`] in positions (Monaco
    /// `deltaDecorations`).
    pub fn delta_decorations_at(
        &mut self,
        owner: Option<EntityId>,
        old: &[DecorationId],
        new: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut Context<Self>,
    ) -> Vec<DecorationId> {
        let new = new
            .into_iter()
            .map(|(range, options)| {
                let range = self.validate_range(range);
                (
                    self.offset_at(range.start)..self.offset_at(range.end),
                    options,
                )
            })
            .collect();
        self.delta_decorations(owner, old, new, cx)
    }
}

/// Monaco `ICodeEditor`'s decoration methods: the editor's model's
/// decorations as this editor sees them, validation decorations left out
/// as its `render_validation_decorations` says.
impl CodeEditor {
    /// Monaco `filterValidationDecorations`.
    fn filter_out_validation(&self) -> bool {
        use crate::options::RenderValidationDecorations;
        match self.options.render_validation_decorations {
            RenderValidationDecorations::Editable => self.options.read_only,
            RenderValidationDecorations::On => false,
            RenderValidationDecorations::Off => true,
        }
    }

    /// The decorations on `row` (Monaco `getLineDecorations`).
    pub fn line_decorations(&self, row: u32, cx: &Context<Self>) -> Vec<ModelDecoration> {
        let owner = Some(cx.entity_id());
        self.model()
            .read(cx)
            .line_decorations(row, owner, self.filter_out_validation())
    }

    /// The decorations intersecting `range` (Monaco
    /// `getDecorationsInRange`).
    pub fn decorations_in_range(
        &self,
        range: Range<PointUtf16>,
        cx: &Context<Self>,
    ) -> Vec<ModelDecoration> {
        let owner = Some(cx.entity_id());
        self.model()
            .read(cx)
            .decorations_in_point_range(range, owner, self.filter_out_validation())
    }

    /// Replaces this editor's decorations `old` with `new` on its model,
    /// returning the new ids (Monaco `deltaDecorations`). Prefer
    /// [`CodeEditor::create_decorations_collection`].
    pub fn delta_decorations(
        &mut self,
        old: &[DecorationId],
        new: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut Context<Self>,
    ) -> Vec<DecorationId> {
        let owner = cx.entity_id();
        self.model().clone().update(cx, |model, cx| {
            model.delta_decorations_at(Some(owner), old, new, cx)
        })
    }

    /// Removes decorations `ids` (Monaco `removeDecorations`).
    pub fn remove_decorations(&mut self, ids: &[DecorationId], cx: &mut Context<Self>) {
        if !ids.is_empty() {
            self.delta_decorations(ids, Vec::new(), cx);
        }
    }
}

/// The editor's font as measured (Monaco `FontInfo`, from
/// `getOption(EditorOption.fontInfo)`), with the font zoom applied.
#[derive(Clone, Debug, PartialEq)]
pub struct FontInfo {
    /// The font family (Monaco `fontFamily`).
    pub font_family: gpui::SharedString,
    /// The font weight (Monaco `fontWeight`).
    pub font_weight: gpui::FontWeight,
    /// The font size (Monaco `fontSize`).
    pub font_size: gpui::Pixels,
    /// The line height (Monaco `lineHeight`).
    pub line_height: gpui::Pixels,
    /// A space is as wide as an `m` (Monaco `isMonospace` checks more
    /// characters).
    pub is_monospace: bool,
    /// The width of a typical half-width character, an `m` (Monaco
    /// `typicalHalfwidthCharacterWidth`).
    pub typical_halfwidth_character_width: gpui::Pixels,
    /// The width of a space (Monaco `spaceWidth`).
    pub space_width: gpui::Pixels,
}

/// Where a position shows, from the editor's top-left (Monaco
/// `getScrolledVisiblePosition`'s result).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrolledVisiblePosition {
    /// From the editor's top (Monaco `top`).
    pub top: gpui::Pixels,
    /// From the editor's left, the gutter included (Monaco `left`).
    pub left: gpui::Pixels,
    /// The line's height (Monaco `height`).
    pub height: gpui::Pixels,
}

/// Monaco `ICodeEditor` conveniences over the cursors, the model and the
/// options.
impl CodeEditor {
    /// The primary cursor's position (Monaco `getPosition`).
    pub fn position(&self) -> PointUtf16 {
        self.cursors.primary().position
    }

    /// Puts a single cursor at `position`, clamped to the text (Monaco
    /// `setPosition`). It doesn't scroll; follow it with a `reveal_*` call
    /// (e.g. [`CodeEditor::reveal_position_in_center`]) to show it.
    pub fn set_position(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        self.set_selections(&[Selection::collapsed(position)], cx);
    }

    /// The primary selection (Monaco `getSelection`).
    pub fn selection(&self) -> Selection {
        self.cursors.selections()[0]
    }

    /// Replaces the selections with `selection` (Monaco `setSelection`),
    /// without scrolling.
    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.set_selections(&[selection], cx);
    }

    /// The model's text (Monaco `ICodeEditor.getValue`), with `eol`'s line
    /// breaks and the BOM if `preserve_bom`.
    pub fn value(&self, eol: EndOfLinePreference, preserve_bom: bool, cx: &App) -> String {
        self.model().read(cx).value(eol, preserve_bom)
    }

    /// Replaces the model's text (Monaco `ICodeEditor.setValue`, which is
    /// the model's [`TextModel::set_value`]).
    pub fn set_value(&mut self, text: &str, cx: &mut Context<Self>) {
        self.model()
            .clone()
            .update(cx, |model, cx| model.set_value(text, cx));
    }

    /// The word at `position` by the editor's `word_separators` (Monaco
    /// `getConfiguredWordAtPosition`), unlike
    /// [`TextModel::word_at_position`], which uses the language's word
    /// pattern.
    pub fn configured_word_at_position(
        &self,
        position: PointUtf16,
        cx: &App,
    ) -> Option<WordAtPosition> {
        let model = self.model().read(cx);
        let position = model.validate_position(position);
        let range =
            cursor::word_at_position(&self.cursor_config(cx), model.buffer().rope(), position)?;
        Some(WordAtPosition {
            word: model.value_in_range(range.clone(), EndOfLinePreference::Lf),
            range,
        })
    }

    /// The editor's font, as of the last frame (Monaco
    /// `getOption(EditorOption.fontInfo)`).
    pub fn font_info(&self) -> Option<FontInfo> {
        let metrics = &self.layout.as_ref()?.metrics;
        Some(FontInfo {
            font_family: metrics.font.family.clone(),
            font_weight: metrics.font.weight,
            font_size: metrics.font_size,
            line_height: metrics.line_height,
            is_monospace: (metrics.space_width - metrics.em_width).abs() < gpui::px(0.01),
            typical_halfwidth_character_width: metrics.em_width,
            space_width: metrics.space_width,
        })
    }

    /// Changes some options (Monaco `updateOptions`, which merges the
    /// options given into the current ones): `update` edits a copy of
    /// [`CodeEditor::options`], then [`CodeEditor::update_options`] applies
    /// it.
    pub fn update_options_with(
        &mut self,
        update: impl FnOnce(&mut crate::EditorOptions),
        cx: &mut Context<Self>,
    ) {
        let mut options = self.options().clone();
        update(&mut options);
        self.update_options(options, cx);
    }

    /// Where `position` shows, from the editor's top-left, with its line's
    /// height, if it was laid out in the last frame (Monaco
    /// `getScrolledVisiblePosition`). [`CodeEditor::scrolled_visible_position`]
    /// is the same point in window coordinates.
    pub fn scrolled_visible_position_in_editor(
        &self,
        position: PointUtf16,
    ) -> Option<ScrolledVisiblePosition> {
        let layout = self.layout.as_ref()?;
        let line = layout.line_for_point(position)?;
        Some(ScrolledVisiblePosition {
            top: layout.line_top(line.view_line) - layout.bounds.top(),
            left: layout.text_origin_x() + line.x_for_column(position.column)
                - layout.bounds.left(),
            height: layout.line_height_of(line.view_line),
        })
    }
}
