//! Embedder conveniences matching Monaco's `ITextModel` and `ICodeEditor`
//! APIs: position helpers, words, searching, revealing, edits with cursor
//! state, visible ranges and running commands by name.

use std::ops::Range;

use gpui::{App, Context, Window};
use text::{Bias, PointUtf16};

use crate::cursor::{self, EditOperation, Selection};
use crate::editor::CodeEditor;
use crate::model::TextModel;
use crate::search::{self, SearchParams};
use crate::view::reveal::RevealType;

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
}

impl CodeEditor {
    /// Scrolls so `range` shows (Monaco `revealRange` and friends). Takes
    /// effect on the next frame.
    pub fn reveal_range(
        &mut self,
        range: Range<PointUtf16>,
        reveal: RevealType,
        cx: &mut Context<Self>,
    ) {
        self.pending_reveal = Some((range, reveal));
        self.autoscroll_requested = false;
        cx.notify();
    }

    /// Scrolls minimally so `row` shows (Monaco `revealLine`).
    pub fn reveal_line(&mut self, row: u32, cx: &mut Context<Self>) {
        let p = PointUtf16::new(row, 0);
        self.reveal_range(p..p, RevealType::Simple, cx);
    }

    /// Centers `row` (Monaco `revealLineInCenter`).
    pub fn reveal_line_in_center(&mut self, row: u32, cx: &mut Context<Self>) {
        let p = PointUtf16::new(row, 0);
        self.reveal_range(p..p, RevealType::Center, cx);
    }

    /// Centers `row` unless it's visible (Monaco
    /// `revealLineInCenterIfOutsideViewport`).
    pub fn reveal_line_in_center_if_outside_viewport(&mut self, row: u32, cx: &mut Context<Self>) {
        let p = PointUtf16::new(row, 0);
        self.reveal_range(p..p, RevealType::CenterIfOutsideViewport, cx);
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

    /// Centers `range` (Monaco `revealRangeInCenter`).
    pub fn reveal_range_in_center(&mut self, range: Range<PointUtf16>, cx: &mut Context<Self>) {
        self.reveal_range(range, RevealType::Center, cx);
    }

    /// Puts `range` at the top (Monaco `revealRangeAtTop`).
    pub fn reveal_range_at_top(&mut self, range: Range<PointUtf16>, cx: &mut Context<Self>) {
        self.reveal_range(range, RevealType::Top, cx);
    }

    /// Puts `range` near the top (Monaco `revealRangeNearTop`).
    pub fn reveal_range_near_top(&mut self, range: Range<PointUtf16>, cx: &mut Context<Self>) {
        self.reveal_range(range, RevealType::NearTop, cx);
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
            return false;
        }
        let selections = self.cursors.selections();
        let mut operation = EditOperation::from_edits(self.snapshot.rope(), &selections, edits);
        if let Some(end) = end_selections.filter(|s| !s.is_empty()) {
            operation.selections_after = end;
        }
        self.apply(operation, cx);
        true
    }

    /// Ends the current undo group (Monaco `pushUndoStop`).
    pub fn push_undo_stop(&mut self, cx: &mut Context<Self>) {
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        self.prev_edit_kind = None;
    }

    /// Runs the GPUI action named `name` (e.g. `"code_editor::CommentLine"`)
    /// as if its key were pressed in this editor (Monaco `trigger`). Fails
    /// for unknown names.
    pub fn trigger(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let action = cx.build_action(name, None)?;
        window.focus(&self.focus_handle, cx);
        window.dispatch_action(action, cx);
        Ok(())
    }
}
