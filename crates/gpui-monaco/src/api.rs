//! Embedder conveniences matching Monaco's `ITextModel` and `ICodeEditor`
//! APIs: position helpers, words, searching, revealing, edits with cursor
//! state, visible ranges and running commands by name.

use std::ops::Range;

use gpui::{App, Context, Entity, EntityId, Window};
use text::{Bias, PointUtf16};

use crate::cursor::{self, EditOperation, Selection};
use crate::decorations::{DecorationId, DecorationOptions};
use crate::editor::{CodeEditor, PendingReveal};
use crate::model::TextModel;
use crate::search::{self, SearchParams};
use crate::view::reveal::{RevealType, ScrollType};

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

    /// The byte offset of `position` (Monaco `getOffsetAt`, which counts
    /// UTF-16 units instead).
    pub fn offset_at(&self, position: PointUtf16) -> usize {
        let rope = self.buffer().rope();
        rope.point_utf16_to_offset(rope.clip_point_utf16(position, Bias::Left))
    }

    /// The position of byte `offset` (Monaco `getPositionAt`).
    pub fn position_at(&self, offset: usize) -> PointUtf16 {
        let rope = self.buffer().rope();
        rope.offset_to_point_utf16(rope.clip_offset(offset.min(rope.len()), Bias::Left))
    }

    /// The text in `range` (Monaco `getValueInRange`), with `\n` line breaks.
    pub fn value_in_range(&self, range: Range<PointUtf16>) -> String {
        let range = self.validate_range(range);
        let rope = self.buffer().rope();
        rope.text_for_range(
            rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end),
        )
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
            word: self.value_in_range(range.clone()),
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
                    word: self.value_in_range(range.clone()),
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

    /// Monaco's offset of `position`: UTF-16 units, line breaks counted as
    /// the model's line ending.
    fn utf16_offset_at(&self, position: PointUtf16) -> usize {
        let rope = self.buffer().rope();
        let position = rope.clip_point_utf16(position, Bias::Left);
        let offset = rope.point_utf16_to_offset(position);
        rope.offset_to_offset_utf16(offset).0
            + position.row as usize * (self.line_ending().as_str().len() - 1)
    }

    /// The position at Monaco offset `offset` (see `utf16_offset_at`).
    fn position_at_utf16_offset(&self, offset: usize) -> PointUtf16 {
        let rope = self.buffer().rope();
        let extra = self.line_ending().as_str().len() - 1;
        let offset = offset.min(self.value_length());
        // At most this row; step back while the row starts after `offset`.
        let mut row = rope
            .offset_to_point(
                rope.offset_utf16_to_offset(text::OffsetUtf16(offset.min(rope.len_utf16().0))),
            )
            .row;
        while row > 0 && self.utf16_offset_at(PointUtf16::new(row, 0)) > offset {
            row -= 1;
        }
        let start = rope
            .offset_to_offset_utf16(rope.point_to_offset(text::Point::new(row, 0)))
            .0
            + row as usize * extra;
        let column = ((offset - start) as u32).min(rope.line_len_utf16(row));
        PointUtf16::new(row, column)
    }

    /// The position `offset` UTF-16 units from `position`, clamped to the
    /// text (Monaco `modifyPosition`; line breaks count as the model's line
    /// ending).
    pub fn modify_position(&self, position: PointUtf16, offset: isize) -> PointUtf16 {
        let start = self.utf16_offset_at(position) as isize;
        let target = (start + offset).clamp(0, self.value_length() as isize);
        self.position_at_utf16_offset(target as usize)
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
    /// scope): overlapping or touching ranges are merged first. With
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
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
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

/// The undo group consecutive [`TextModel::push_edit_operations`] calls
/// share until [`TextModel::push_stack_element`], as Monaco appends them to
/// the open undo element.
const PUSH_EDIT_OPERATIONS_GROUP: u64 = u64::MAX - 1;

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
        self.apply_edits_in(edits, None, cx)
    }

    fn apply_edits_in(
        &mut self,
        edits: Vec<(Range<PointUtf16>, String)>,
        group: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<(Range<PointUtf16>, String)>, text::EditError> {
        let old = self.buffer().rope().clone();
        let edits: Vec<(Range<usize>, String)> = edits
            .into_iter()
            .map(|(range, text)| {
                let range = self.validate_range(range);
                (
                    old.point_utf16_to_offset(range.start)..old.point_utf16_to_offset(range.end),
                    text,
                )
            })
            .collect();
        let patch = self.edit_from(edits, group, None, cx)?;
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
    /// `pushEditOperations`): consecutive calls join one step until
    /// [`TextModel::push_stack_element`]. `before_cursor_state` is restored
    /// by undo. `cursor_state_computer` gets the undo edits and returns the
    /// selections after, which redo restores; they're returned.
    pub fn push_edit_operations(
        &mut self,
        before_cursor_state: Option<Vec<Selection>>,
        edits: Vec<(Range<PointUtf16>, String)>,
        cursor_state_computer: Option<&CursorStateComputer>,
        cx: &mut Context<Self>,
    ) -> Result<Option<Vec<Selection>>, text::EditError> {
        let undo_edits = self.apply_edits_in(edits, Some(PUSH_EDIT_OPERATIONS_GROUP), cx)?;
        let after = cursor_state_computer.and_then(|compute| compute(&undo_edits));
        let metadata = |selections: Option<Vec<Selection>>| {
            selections.map(|s| std::sync::Arc::new(s) as text::TransactionMetadata)
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
    fn request_reveal(&mut self, request: PendingReveal, cx: &mut Context<Self>) {
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

    /// Whether the editor has keyboard focus (Monaco `hasTextFocus`).
    pub fn has_text_focus(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Whether the editor or a widget in it, such as the find widget, has
    /// keyboard focus (Monaco `hasWidgetFocus`).
    pub fn has_widget_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.contains_focused(window, cx)
    }

    /// Applies `edits` (in current coordinates) as one undo step, then sets
    /// `end_selections`, or lets the cursors follow the edits (Monaco
    /// `executeEdits`). Returns `false` in a read-only editor.
    pub fn execute_edits(
        &mut self,
        edits: Vec<(Range<PointUtf16>, String)>,
        end_selections: Option<Vec<Selection>>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.options.read_only {
            self.read_only_edit_attempt(cx);
            return false;
        }
        let selections = self.cursors.selections();
        let mut operation = EditOperation::from_edits(self.snapshot.rope(), &selections, edits);
        if let Some(end) = end_selections.filter(|s| !s.is_empty()) {
            operation.selections_after = end;
        }
        self.next_selection_change.source = crate::events::CursorChangeSource::Api;
        self.apply(operation, cx);
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

    /// Runs the editor action or command with Monaco id `name` (e.g.
    /// `"editor.action.commentLine"`, `"cursorDown"`, or one from
    /// [`CodeEditor::add_action`]), or else the GPUI action named `name`
    /// (e.g. `"code_editor::CommentLine"`), as if its key were pressed in
    /// this editor (Monaco `trigger`). Fails for unknown names.
    pub fn trigger(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        if self.run_action(name, window, cx) {
            return Ok(());
        }
        // Monaco's unlabelled commands (`cursorDown`, `undo`, …), then GPUI
        // action names.
        let name = crate::editor_actions::command_action_name(name).unwrap_or(name);
        let action = cx.build_action(name, None)?;
        window.focus(&self.focus_handle, cx);
        window.dispatch_action(action, cx);
        Ok(())
    }

    /// [`CodeEditor::trigger`] with Monaco's `payload`: the arguments of
    /// commands that take some, such as `editor.actions.findWithArgs`
    /// ([`FindWithArgs`](crate::actions::FindWithArgs)) and
    /// `editor.action.codeAction`
    /// ([`TriggerCodeAction`](crate::actions::TriggerCodeAction)), as JSON.
    /// Actions added with [`CodeEditor::add_action`] run without it. Fails
    /// for unknown names and payloads the action can't be built from.
    pub fn trigger_with_payload(
        &mut self,
        name: &str,
        payload: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        if self.added_actions.iter().any(|a| a.id.as_ref() == name) {
            self.run_action(name, window, cx);
            return Ok(());
        }
        let name = match crate::editor_actions::action_name(name) {
            Some((name, precondition)) => {
                if precondition.is_some_and(|precondition| !precondition(self, cx)) {
                    return Ok(());
                }
                name
            }
            None => name,
        };
        let action = cx.build_action(name, Some(payload))?;
        window.focus(&self.focus_handle, cx);
        window.dispatch_action(action, cx);
        Ok(())
    }
}

/// A set of decorations an editor owns, replaced as a whole (Monaco
/// `IEditorDecorationsCollection`, from
/// [`CodeEditor::create_decorations_collection`]).
///
/// It stays bound to the model the editor had when it was created. Dropping
/// it leaves the decorations in place; call [`DecorationsCollection::clear`]
/// first to remove them.
#[derive(Clone, Debug)]
pub struct DecorationsCollection {
    model: Entity<TextModel>,
    owner: EntityId,
    ids: Vec<DecorationId>,
}

impl DecorationsCollection {
    /// An empty collection on `model` whose decorations only `owner` shows.
    pub(crate) fn for_owner(model: Entity<TextModel>, owner: EntityId) -> Self {
        Self {
            model,
            owner,
            ids: Vec::new(),
        }
    }

    /// Replaces the decorations with `decorations` (positions in the
    /// current text), returning their ids (Monaco `set`).
    pub fn set(
        &mut self,
        decorations: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut App,
    ) -> &[DecorationId] {
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
        let ids = self.delta(&[], decorations, cx);
        self.ids.extend(&ids);
        ids
    }

    /// Removes every decoration (Monaco `clear`).
    pub fn clear(&mut self, cx: &mut App) {
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

    /// Where decoration `index` is now, after edits (Monaco `getRange`).
    pub fn range(&self, index: usize, cx: &App) -> Option<Range<PointUtf16>> {
        let model = self.model.read(cx);
        let decoration = model.decoration(*self.ids.get(index)?)?;
        let rope = model.buffer().rope();
        Some(
            rope.offset_to_point_utf16(decoration.range.start)
                ..rope.offset_to_point_utf16(decoration.range.end),
        )
    }

    /// Where every decoration is now (Monaco `getRanges`).
    pub fn ranges(&self, cx: &App) -> Vec<Range<PointUtf16>> {
        (0..self.ids.len())
            .filter_map(|index| self.range(index, cx))
            .collect()
    }

    fn delta(
        &self,
        old: &[DecorationId],
        new: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut App,
    ) -> Vec<DecorationId> {
        let owner = self.owner;
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
    /// A [`DecorationsCollection`] on the current model, owned by this
    /// editor, starting with `decorations` (Monaco
    /// `createDecorationsCollection`).
    pub fn create_decorations_collection(
        &self,
        decorations: Vec<(Range<PointUtf16>, DecorationOptions)>,
        cx: &mut Context<Self>,
    ) -> DecorationsCollection {
        let mut collection = DecorationsCollection {
            model: self.model().clone(),
            owner: cx.entity_id(),
            ids: Vec::new(),
        };
        if !decorations.is_empty() {
            collection.set(decorations, cx);
        }
        collection
    }
}
