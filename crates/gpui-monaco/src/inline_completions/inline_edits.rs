//! Inline edits (next edit suggestions): how an inline edit's changes are
//! computed and followed through typing, and which view shows them.
//!
//! Ported from Monaco / VS Code (MIT License, Copyright (c) Microsoft
//! Corporation):
//! - `vs/editor/contrib/inlineCompletions/browser/model/inlineSuggestionItem.ts`
//!   (`InlineEditItem`): [`diffed_edits`] (`getDiffedStringEdit`,
//!   `reshapeInlineEdit`, `reshapeMultiLineInsertion`) and [`UpdatedEdit`]
//!   (`SingleUpdatedNextEdit`),
//! - `.../browser/view/inlineEdits/inlineEditsView.ts` (`_determineView`,
//!   `_willRenderAboveCursor`, `isDeletion`, `growEditsToEntireWord`, ...)
//!   as [`determine_view`] and [`grow_edits_to_entire_word`],
//! - `.../view/inlineEdits/inlineEditWithChanges.ts` as [`InlineEditDiff`],
//! - `vs/editor/common/core/edits/lineEdit.ts`
//!   (`LineReplacement.fromSingleTextEdit`) as [`line_replacement`], and
//!   `textEdit.ts` (`getNewRanges`) as [`new_ranges`].
//!
//! # Coordinates
//!
//! Edits being followed through typing use byte offsets in the document;
//! everything else uses 0-based rows and UTF-16 columns ([`PointUtf16`]).
//! Monaco measures text in UTF-16 code units; prefix and suffix lengths
//! here are bytes, which agree on where they end.

use std::ops::Range;

use text::{PointUtf16, Rope};

use crate::diff::{self, DiffOptions, Position, RangeMapping};
use crate::options::{InlineEditsCodeShifting, InlineEditsRenderSideBySide};

/// Replacements of ranges by text, in the current text.
pub(crate) type Replacements = Vec<(Range<PointUtf16>, String)>;

/// Text changes: byte ranges of the previous text and their new text.
pub(crate) type Changes = Vec<(Range<usize>, String)>;

/// Monaco `InlineEditsWordReplacementView.MAX_LENGTH`.
const WORD_REPLACEMENT_MAX_LENGTH: u32 = 100;

/// How an inline edit shows (Monaco `InlineCompletionViewKind`, without the
/// ghost text, custom and jump-to kinds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InlineEditViewKind {
    /// The new word in a box below the changed one (Monaco
    /// `wordReplacements`).
    WordReplacements,
    /// The new lines in a box below the changed ones (`lineReplacement`).
    LineReplacement,
    /// Text inserted within a line, shown in place (`insertionInline`).
    InsertionInline,
    /// Inserted lines, shown in place, pushing the text below down
    /// (`insertionMultiLine`).
    InsertionMultiLine,
    /// The deleted text marked in place (`deletion`).
    Deletion,
    /// The new lines in a card right of the changed ones (`sideBySide`).
    SideBySide,
    /// Only a marker at the edit until it's jumped to (`collapsed`).
    Collapsed,
}

/// The byte length of the longest common prefix of `a` and `b`.
pub(crate) fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

/// The byte length of the longest common suffix of `a` and `b`.
pub(crate) fn common_suffix_len(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

/// The byte offset of UTF-16 column `column` in `line`.
pub(crate) fn utf16_to_byte(line: &str, column: u32) -> usize {
    let mut units = 0;
    for (ix, ch) in line.char_indices() {
        if units >= column as usize {
            return ix;
        }
        units += ch.len_utf16();
    }
    line.len()
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Where text inserted at `start` ends (Monaco `TextLength.createRange`).
pub(crate) fn text_end(start: PointUtf16, text: &str) -> PointUtf16 {
    match text.rfind('\n') {
        Some(ix) => PointUtf16::new(
            start.row + text.matches('\n').count() as u32,
            utf16_len(&text[ix + 1..]) as u32,
        ),
        None => PointUtf16::new(start.row, start.column + utf16_len(text) as u32),
    }
}

/// One replacement of an inline edit, followed through the user's typing
/// (Monaco `SingleUpdatedNextEdit`). Typing the replacement's text moves
/// its start along; other changes inside it drop it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UpdatedEdit {
    /// The replacement (bytes of the current text), or `None` once a change
    /// conflicted with it.
    pub edit: Option<(Range<usize>, String)>,
    trimmed_new_text: String,
    prefix_len: usize,
    suffix_len: usize,
    /// The last change typed part of it (Monaco `lastChangeUpdatedEdit`).
    pub last_change_updated_edit: bool,
}

impl UpdatedEdit {
    /// `range`, holding `replaced`, replaced by `new_text` (Monaco
    /// `SingleUpdatedNextEdit.create`).
    pub fn new(range: Range<usize>, new_text: String, replaced: &str) -> Self {
        let prefix_len = common_prefix_len(&new_text, replaced);
        let suffix_len = common_suffix_len(&new_text, replaced);
        let end = new_text.len().saturating_sub(suffix_len).max(prefix_len);
        Self {
            trimmed_new_text: new_text[prefix_len..end].to_string(),
            edit: Some((range, new_text)),
            prefix_len,
            suffix_len,
            last_change_updated_edit: false,
        }
    }

    /// Follows `changes` (old ranges, sorted and disjoint, with their new
    /// text) (Monaco `applyTextModelChanges`).
    pub fn apply_changes(&mut self, changes: &[(Range<usize>, String)]) {
        self.last_change_updated_edit = false;
        let Some(edit) = self.edit.take() else {
            return;
        };
        if let Some((edit, changed)) = self.apply(&edit, changes) {
            self.edit = Some(edit);
            self.last_change_updated_edit = changed;
        }
    }

    /// Monaco `_applyChanges`.
    fn apply(
        &mut self,
        edit: &(Range<usize>, String),
        changes: &[(Range<usize>, String)],
    ) -> Option<((Range<usize>, String), bool)> {
        let mut start = edit.0.start as isize;
        let mut end = edit.0.end as isize;
        let mut text = edit.1.clone();
        let mut changed = false;
        let preserve_shape = self.prefix_len > 0 || self.suffix_len > 0;
        for (range, new_text) in changes.iter().rev() {
            let (change_start, change_end) = (range.start as isize, range.end as isize);
            let inserted = new_text.len() as isize;
            // Insertions, only at the start of the edit.
            let is_insertion = !new_text.is_empty() && range.is_empty();
            if is_insertion
                && !preserve_shape
                && change_start == start
                && text.starts_with(new_text.as_str())
            {
                start += inserted;
                text = text[new_text.len()..].to_string();
                end += inserted;
                changed = true;
                continue;
            }
            if is_insertion
                && preserve_shape
                && change_start == start + self.prefix_len as isize
                && self.trimmed_new_text.starts_with(new_text.as_str())
            {
                end += inserted;
                changed = true;
                self.prefix_len += new_text.len();
                self.trimmed_new_text = self.trimmed_new_text[new_text.len()..].to_string();
                continue;
            }
            // Deleting text the edit deletes too.
            let is_deletion = new_text.is_empty() && !range.is_empty();
            if is_deletion
                && change_start >= start + self.prefix_len as isize
                && change_end <= end - self.suffix_len as isize
            {
                end -= change_end - change_start;
                changed = true;
                continue;
            }
            // Doing exactly the edit.
            if *range == edit.0 && *new_text == edit.1 {
                changed = true;
                start = change_end;
                text.clear();
                continue;
            }
            // Changes before or after it move it.
            if change_start > end {
                continue;
            }
            if change_end < start {
                let delta = inserted - (change_end - change_start);
                start += delta;
                end += delta;
                continue;
            }
            // A change inside it drops it.
            return None;
        }
        if start < 0 || end < start {
            return None;
        }
        // Nothing left to change.
        let at = start + self.prefix_len as isize;
        if self.trimmed_new_text.is_empty() && at == end - self.suffix_len as isize {
            let at = at as usize;
            return Some(((at..at, String::new()), true));
        }
        Some(((start as usize..end as usize, text), changed))
    }
}

/// The point `relative` (1-based, within text that starts at `base`) in the
/// document (Monaco's `addRangeToPos`).
fn absolute(base: PointUtf16, relative: Position) -> PointUtf16 {
    let row = relative.line_number as u32 - 1;
    let column = relative.column as u32 - 1;
    if row == 0 {
        PointUtf16::new(base.row, base.column + column)
    } else {
        PointUtf16::new(base.row + row, column)
    }
}

/// The text of `range` (1-based) in `lines`.
fn text_in_lines(lines: &[&str], range: &diff::Range) -> String {
    let line = |n: usize| lines.get(n - 1).copied().unwrap_or("");
    let (start, end) = (range.start(), range.end());
    if start.line_number == end.line_number {
        let text = line(start.line_number);
        let from = utf16_to_byte(text, start.column as u32 - 1);
        let to = utf16_to_byte(text, end.column as u32 - 1).max(from);
        return text[from..to].to_string();
    }
    let first = line(start.line_number);
    let mut out = first[utf16_to_byte(first, start.column as u32 - 1)..].to_string();
    for n in start.line_number + 1..end.line_number {
        out.push('\n');
        out.push_str(line(n));
    }
    let last = line(end.line_number);
    out.push('\n');
    out.push_str(&last[..utf16_to_byte(last, end.column as u32 - 1)]);
    out
}

/// Monaco `getDiffedStringEdit`: `range` (bytes) of `rope` replaced by
/// `text`, as the small replacements a line diff with word-level inner
/// changes finds, each reshaped by `reshapeInlineEdit`.
pub(crate) fn diffed_edits(
    rope: &Rope,
    range: Range<usize>,
    text: &str,
) -> Vec<(Range<usize>, String)> {
    let original = rope.text_for_range(range.clone());
    let original_lines: Vec<&str> = original.split('\n').collect();
    let modified_lines: Vec<&str> = text.split('\n').collect();
    let lines_diff = diff::compute_diff(
        &original_lines,
        &modified_lines,
        &DiffOptions {
            ignore_trim_whitespace: false,
            max_computation_time_ms: 50,
            compute_moves: false,
            extend_to_subwords: true,
        },
    );
    let inner: Vec<RangeMapping> = lines_diff
        .changes
        .iter()
        .flat_map(|change| change.inner_changes.iter().copied())
        .collect();
    let base = rope.offset_to_point_utf16(range.start);
    let count = inner.len();
    inner
        .iter()
        .map(|change| {
            let start = absolute(base, change.original_range.start());
            let end = absolute(base, change.original_range.end());
            let replaced = rope.point_utf16_to_offset(start)..rope.point_utf16_to_offset(end);
            let new_text = text_in_lines(&modified_lines, &change.modified_range);
            let original_text = rope.text_for_range(replaced.clone());
            reshape_inline_edit(rope, (replaced, new_text), &original_text, count)
        })
        .collect()
}

/// Monaco `reshapeInlineEdit`.
fn reshape_inline_edit(
    rope: &Rope,
    edit: (Range<usize>, String),
    original_text: &str,
    total_inner_edits: usize,
) -> (Range<usize>, String) {
    let (mut range, mut text) = edit;
    // The diff doesn't trim line breaks.
    if text.ends_with('\n') && original_text.ends_with('\n') && !range.is_empty() {
        range.end -= 1;
        text.pop();
    }
    // An insertion ending in a line break at the start of a line with text
    // moves to the end of the line before.
    if total_inner_edits == 1 && range.is_empty() && text.contains('\n') {
        let row = rope.offset_to_point(range.start).row;
        if rope.line_len(row) != 0 {
            (range, text) = reshape_multi_line_insertion(rope, range, text);
        }
    }
    // The diff grew a plain insertion or deletion to whole words: shrink it
    // back.
    if total_inner_edits == 1 {
        let prefix = common_prefix_len(original_text, &text);
        let suffix = common_suffix_len(&original_text[prefix..], &text[prefix..]);
        let shrunk = range.start + prefix..range.end.saturating_sub(suffix);
        if shrunk.start <= shrunk.end && prefix + suffix <= text.len() {
            if prefix + suffix == original_text.len() {
                return (shrunk, text[prefix..text.len() - suffix].to_string());
            }
            if prefix + suffix == text.len() {
                return (shrunk, String::new());
            }
        }
    }
    (range, text)
}

/// Monaco `reshapeMultiLineInsertion`: an insertion at a line start that
/// ends with a line break moves before the line break ending the line
/// before.
fn reshape_multi_line_insertion(
    rope: &Rope,
    range: Range<usize>,
    text: String,
) -> (Range<usize>, String) {
    if range.start == 0 {
        return (range, text);
    }
    let start = rope.offset_to_point(range.start);
    if start.column == 0 && start.row > 0 && text.ends_with('\n') && !text.starts_with('\n') {
        let text = format!("\n{}", &text[..text.len() - 1]);
        return (range.start - 1..range.end - 1, text);
    }
    (range, text)
}

/// Monaco `TextEdit.getNewRanges`: where each of `edits` (sorted and
/// disjoint) ends up once they're all applied.
pub(crate) fn new_ranges(edits: &[(Range<PointUtf16>, String)]) -> Vec<Range<PointUtf16>> {
    let mut ranges = Vec::with_capacity(edits.len());
    let mut previous_end_row = None;
    let mut row_offset = 0i64;
    let mut column_offset = 0i64;
    for (range, text) in edits {
        let column = range.start.column as i64
            + if previous_end_row == Some(range.start.row) {
                column_offset
            } else {
                0
            };
        let start = PointUtf16::new(
            (range.start.row as i64 + row_offset).max(0) as u32,
            column.max(0) as u32,
        );
        let end = text_end(start, text);
        row_offset = end.row as i64 - range.end.row as i64;
        column_offset = end.column as i64 - range.end.column as i64;
        previous_end_row = Some(range.end.row);
        ranges.push(start..end);
    }
    ranges
}

/// The text of `line` before UTF-16 column `column`.
fn utf16_prefix(line: &str, column: u32) -> &str {
    &line[..utf16_to_byte(line, column)]
}

/// Monaco `LineReplacement.fromSingleTextEdit`: the rows `range` of `rope`
/// touches (end exclusive) and the lines that replace them once `range` is
/// replaced by `text`, without rows the replacement leaves alone.
pub(crate) fn line_replacement(
    rope: &Rope,
    range: Range<PointUtf16>,
    text: &str,
) -> (Range<u32>, Vec<String>) {
    let mut new_lines: Vec<String> = text.split('\n').map(String::from).collect();
    let mut start_row = range.start.row;
    let first_line = rope.line(range.start.row);
    let surviving_first = utf16_prefix(&first_line, range.start.column).to_string();
    new_lines[0] = format!("{surviving_first}{}", new_lines[0]);
    let mut end_row = range.end.row + 1;
    let end_line = rope.line(range.end.row);
    let surviving_end = end_line[utf16_to_byte(&end_line, range.end.column)..].to_string();
    if let Some(last) = new_lines.last_mut() {
        last.push_str(&surviving_end);
    }
    let start_before_newline = range.start.column == rope.line_len_utf16(range.start.row);
    let end_after_newline = range.end.column == 0;
    if start_before_newline && new_lines[0].len() == surviving_first.len() {
        // Nothing on the first line is deleted.
        start_row += 1;
        new_lines.remove(0);
    }
    if !new_lines.is_empty()
        && start_row < end_row
        && end_after_newline
        && new_lines
            .last()
            .is_some_and(|l| l.len() == surviving_end.len())
    {
        // Nothing on the last line is deleted.
        end_row -= 1;
        new_lines.pop();
    }
    (start_row..end_row, new_lines)
}

/// An inline edit's changes as Monaco's inline edits view sees them
/// (`InlineEditWithChanges`, its line edit and its diff).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InlineEditDiff {
    /// The replacements, sorted and disjoint, in the current text.
    pub edits: Vec<(Range<PointUtf16>, String)>,
    /// The first row the edit touches, which starts both windows.
    window_start: u32,
    /// The rows the edit touches.
    original_lines: Vec<String>,
    /// The same rows with the edit applied.
    modified_lines: Vec<String>,
    /// Each replacement's range in the original text and in the text with
    /// the edit applied (Monaco `RangeMapping.fromEdit`).
    pub inner: Vec<(Range<PointUtf16>, Range<PointUtf16>)>,
    /// The changed rows (Monaco's `DetailedLineRangeMapping`s): original
    /// rows, modified rows and the indices of their `inner` changes.
    pub lines: Vec<(Range<u32>, Range<u32>, Vec<usize>)>,
    /// The original rows the edit replaces (Monaco `originalLineRange`).
    pub original_line_range: Range<u32>,
    /// The rows replacing them in the modified text (`modifiedLineRange`).
    pub modified_line_range: Range<u32>,
    /// The lines replacing `original_line_range` (`lineEdit.newLines`).
    pub new_lines: Vec<String>,
    /// The document's row count.
    row_count: u32,
}

impl InlineEditDiff {
    /// The diff of `edits` (sorted and disjoint) on `rope`; `None` without
    /// edits.
    pub fn new(rope: &Rope, edits: Vec<(Range<PointUtf16>, String)>) -> Option<Self> {
        let first = edits.first()?.0.start;
        let last = edits.last()?.0.end;
        let window_start = first.row;
        let window_end = last.row;
        let offset = |point: PointUtf16| rope.point_utf16_to_offset(point);
        let original_lines: Vec<String> = (window_start..=window_end)
            .map(|row| rope.line(row))
            .collect();
        // The text replacing `first..last`.
        let mut combined = String::new();
        let mut at = offset(first);
        for (range, text) in &edits {
            combined.push_str(&rope.text_for_range(at..offset(range.start).max(at)));
            combined.push_str(text);
            at = offset(range.end);
        }
        let window_end_offset =
            offset(PointUtf16::new(window_end, rope.line_len_utf16(window_end)));
        let modified = format!(
            "{}{combined}{}",
            rope.text_for_range(offset(PointUtf16::new(window_start, 0))..offset(first)),
            rope.text_for_range(at..window_end_offset.max(at)),
        );
        let modified_lines: Vec<String> = modified.split('\n').map(String::from).collect();
        let modified_ranges = new_ranges(&edits);
        let inner: Vec<(Range<PointUtf16>, Range<PointUtf16>)> = edits
            .iter()
            .zip(modified_ranges)
            .map(|((range, _), modified)| (range.clone(), modified))
            .collect();
        // Monaco `lineRangeMappingFromRangeMappings`, on the windows.
        let relative = |point: PointUtf16| {
            Position::new(
                (point.row - window_start) as usize + 1,
                point.column as usize + 1,
            )
        };
        let mappings: Vec<RangeMapping> = inner
            .iter()
            .map(|(original, modified)| {
                RangeMapping::new(
                    diff::Range::from_positions(relative(original.start), relative(original.end)),
                    diff::Range::from_positions(relative(modified.start), relative(modified.end)),
                )
            })
            .collect();
        let utf16 = |lines: &[String]| -> Vec<Vec<u16>> {
            lines
                .iter()
                .map(|line| line.encode_utf16().collect())
                .collect()
        };
        let mut next = 0;
        let lines = diff::line_range_mapping_from_range_mappings(
            &mappings,
            &utf16(&original_lines),
            &utf16(&modified_lines),
        )
        .into_iter()
        .map(|mapping| {
            let rows = |range: diff::LineRange| {
                range.start_line_number as u32 - 1 + window_start
                    ..range.end_line_number_exclusive as u32 - 1 + window_start
            };
            let indices: Vec<usize> = (next..next + mapping.inner_changes.len()).collect();
            next += mapping.inner_changes.len();
            (rows(mapping.original), rows(mapping.modified), indices)
        })
        .collect();
        let (original_line_range, new_lines) = line_replacement(rope, first..last, &combined);
        let modified_line_range =
            original_line_range.start..original_line_range.start + new_lines.len() as u32;
        Some(Self {
            edits,
            window_start,
            original_lines,
            modified_lines,
            inner,
            lines,
            original_line_range,
            modified_line_range,
            new_lines,
            row_count: rope.max_point().row + 1,
        })
    }

    fn text_of(&self, lines: &[String], range: &Range<PointUtf16>) -> String {
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        let relative = |point: PointUtf16| {
            Position::new(
                point.row.saturating_sub(self.window_start) as usize + 1,
                point.column as usize + 1,
            )
        };
        text_in_lines(
            &lines,
            &diff::Range::from_positions(relative(range.start), relative(range.end)),
        )
    }

    /// The original text of `range`.
    pub fn original_text(&self, range: &Range<PointUtf16>) -> String {
        self.text_of(&self.original_lines, range)
    }

    /// The modified text of `range` (rows of the modified text).
    pub fn modified_text(&self, range: &Range<PointUtf16>) -> String {
        self.text_of(&self.modified_lines, range)
    }

    /// The original row `row`, if the edit touches it.
    pub fn original_line(&self, row: u32) -> Option<&str> {
        let ix = row.checked_sub(self.window_start)?;
        self.original_lines.get(ix as usize).map(String::as_str)
    }

    /// Row `row` of the modified text, if the edit touches it.
    pub fn modified_line(&self, row: u32) -> Option<&str> {
        let ix = row.checked_sub(self.window_start)?;
        self.modified_lines.get(ix as usize).map(String::as_str)
    }

    /// The modified rows the edit touches, from the window's start.
    pub fn modified_window(&self) -> (u32, &[String]) {
        (self.window_start, &self.modified_lines)
    }

    /// The original rows shown (Monaco `displayRange`): the replaced rows,
    /// extended to as many rows as replace them.
    pub fn display_range(&self) -> Range<u32> {
        let start = self.original_line_range.start;
        let end = self
            .original_line_range
            .end
            .max(start + self.new_lines.len() as u32)
            .min(self.row_count);
        start.min(end)..end
    }
}

/// The settings [`determine_view`] depends on.
pub(crate) struct ViewSettings {
    /// The primary cursor.
    pub cursor: PointUtf16,
    /// All cursors.
    pub cursors: Vec<PointUtf16>,
    /// `inline_suggest.edits.allow_code_shifting`.
    pub code_shifting: InlineEditsCodeShifting,
    /// `inline_suggest.edits.render_side_by_side`.
    pub render_side_by_side: InlineEditsRenderSideBySide,
}

/// Monaco `TextLength.ofRange(range).columnCount`.
fn column_count(range: &Range<PointUtf16>) -> u32 {
    if range.start.row == range.end.row {
        range.end.column - range.start.column
    } else {
        range.end.column
    }
}

/// Monaco `isSingleLineInsertion`: every change inserts within a line.
fn is_single_line_insertion(diff: &InlineEditDiff) -> bool {
    diff.inner
        .iter()
        .all(|(original, modified)| original.is_empty() && modified.start.row == modified.end.row)
}

/// Monaco `isSingleLineInsertionAfterPosition`: also, none inserts before
/// `position` on its line.
fn is_single_line_insertion_after_position(diff: &InlineEditDiff, position: PointUtf16) -> bool {
    is_single_line_insertion(diff)
        && diff
            .inner
            .iter()
            .all(|(original, _)| position <= original.start || original.start.row < position.row)
}

/// Monaco `isSingleMultiLineInsertion`.
fn is_single_multi_line_insertion(diff: &InlineEditDiff) -> bool {
    match diff.inner.as_slice() {
        [(original, modified)] => original.is_empty() && modified.start.row != modified.end.row,
        _ => false,
    }
}

/// Monaco `isDeletion`: every change deletes text, leaving at most
/// whitespace.
fn is_deletion(diff: &InlineEditDiff) -> bool {
    diff.inner.iter().all(|(original, modified)| {
        let original = diff.original_text(original);
        let modified = diff.modified_text(modified);
        modified.trim().is_empty()
            && !original.is_empty()
            && (utf16_len(&original) > utf16_len(&modified) || !original.trim().is_empty())
    })
}

/// Monaco `_determineView` and `_willRenderAboveCursor`: how to show `diff`.
/// `fits_side_by_side` is Monaco `InlineEditsSideBySideView.fitsInsideViewport`.
pub(crate) fn determine_view(
    diff: &InlineEditDiff,
    settings: &ViewSettings,
    fits_side_by_side: impl FnOnce() -> bool,
) -> InlineEditViewKind {
    let view = determine_view_kind(diff, settings, fits_side_by_side);
    if will_render_above_cursor(diff, settings, view)
        && let InlineEditViewKind::LineReplacement | InlineEditViewKind::WordReplacements = view
    {
        return InlineEditViewKind::SideBySide;
    }
    view
}

fn determine_view_kind(
    diff: &InlineEditDiff,
    settings: &ViewSettings,
    fits_side_by_side: impl FnOnce() -> bool,
) -> InlineEditViewKind {
    use InlineEditViewKind::*;
    let original_line_count = diff.original_line_range.len();
    let modified_line_count = diff.modified_line_range.len();
    let single_inner_edit = diff.inner.len() == 1;
    if single_inner_edit
        && settings.code_shifting != InlineEditsCodeShifting::Never
        && is_single_line_insertion(diff)
    {
        if is_single_line_insertion_after_position(diff, settings.cursor) {
            return InsertionInline;
        }
        // Inserting before the cursor would move it: a line replacement.
        return LineReplacement;
    }
    if is_deletion(diff) {
        return Deletion;
    }
    if is_single_multi_line_insertion(diff)
        && settings.code_shifting == InlineEditsCodeShifting::Always
    {
        return InsertionMultiLine;
    }
    let short = diff.inner.iter().all(|(original, modified)| {
        column_count(original) < WORD_REPLACEMENT_MAX_LENGTH
            && column_count(modified) < WORD_REPLACEMENT_MAX_LENGTH
    });
    if short && single_inner_edit && original_line_count == 1 && modified_line_count == 1 {
        // No indentation changes as word replacements.
        let tabs = diff.inner.iter().any(|(original, modified)| {
            diff.original_text(original).contains('\t')
                || diff.modified_text(modified).contains('\t')
        });
        if !tabs {
            // No insertions, even grown ones.
            let any_insertion = diff.inner.iter().any(|(original, _)| original.is_empty());
            let grown_insertion = || {
                let deletions: Vec<_> = diff
                    .inner
                    .iter()
                    .map(|(original, _)| (original.clone(), String::new()))
                    .collect();
                grow_edits_until_whitespace(diff, deletions)
                    .iter()
                    .any(|(range, _)| {
                        range.is_empty() && column_count(range) < WORD_REPLACEMENT_MAX_LENGTH
                    })
            };
            if !any_insertion || !grown_insertion() {
                return WordReplacements;
            }
        }
    }
    if original_line_count > 0 && modified_line_count > 0 {
        if original_line_count == 1 && modified_line_count == 1 {
            return LineReplacement;
        }
        if settings.render_side_by_side != InlineEditsRenderSideBySide::Never && fits_side_by_side()
        {
            return SideBySide;
        }
        return LineReplacement;
    }
    SideBySide
}

/// Monaco `_willRenderAboveCursor`: the box would cover a cursor.
fn will_render_above_cursor(
    diff: &InlineEditDiff,
    settings: &ViewSettings,
    view: InlineEditViewKind,
) -> bool {
    if settings.code_shifting == InlineEditsCodeShifting::Always {
        return false;
    }
    settings.cursors.iter().any(|cursor| match view {
        InlineEditViewKind::WordReplacements => cursor.row == diff.original_line_range.start + 1,
        InlineEditViewKind::LineReplacement => {
            cursor.row >= diff.original_line_range.end
                && cursor.row < diff.modified_line_range.end + diff.modified_line_range.len() as u32
        }
        _ => false,
    })
}

/// Monaco `growEditsToEntireWord`: replacements grown to whole ASCII words.
pub(crate) fn grow_edits_to_entire_word(
    diff: &InlineEditDiff,
    replacements: Vec<(Range<PointUtf16>, String)>,
) -> Vec<(Range<PointUtf16>, String)> {
    grow_edits(diff, replacements, |c| {
        char::from_u32(c as u32).is_some_and(|c| c.is_ascii_alphabetic())
    })
}

/// Monaco `growEditsUntilWhitespace`.
pub(crate) fn grow_edits_until_whitespace(
    diff: &InlineEditDiff,
    replacements: Vec<(Range<PointUtf16>, String)>,
) -> Vec<(Range<PointUtf16>, String)> {
    grow_edits(diff, replacements, |c| {
        !char::from_u32(c as u32).is_some_and(char::is_whitespace)
    })
}

/// Monaco `_growEdits`: grows each replacement over the original text's
/// neighbors that `included` accepts, joining ones that then touch.
fn grow_edits(
    diff: &InlineEditDiff,
    mut replacements: Vec<(Range<PointUtf16>, String)>,
    included: fn(u16) -> bool,
) -> Vec<(Range<PointUtf16>, String)> {
    replacements.sort_by_key(|(range, _)| (range.start, range.end));
    let line = |row: u32| -> Vec<u16> {
        diff.original_line(row)
            .map(|line| line.encode_utf16().collect())
            .unwrap_or_default()
    };
    let at = |line: &[u16], ix: isize| -> Option<u16> {
        (ix >= 0).then(|| line.get(ix as usize).copied()).flatten()
    };
    let mut result: Vec<(Range<PointUtf16>, String)> = Vec::new();
    for (range, text) in replacements {
        let mut start = range.start.column as isize;
        let mut end = range.end.column as isize - 1;
        let start_line = line(range.start.row);
        let end_line = line(range.end.row);
        let mut prefix: Vec<u16> = Vec::new();
        let mut suffix: Vec<u16> = Vec::new();
        if at(&start_line, start).is_some_and(included) {
            while let Some(c) = at(&start_line, start - 1).filter(|c| included(*c)) {
                prefix.insert(0, c);
                start -= 1;
            }
        }
        if at(&end_line, end).is_some_and(included) || end < start {
            while let Some(c) = at(&end_line, end + 1).filter(|c| included(*c)) {
                suffix.push(c);
                end += 1;
            }
        }
        let mut edit = (
            PointUtf16::new(range.start.row, start.max(0) as u32)
                ..PointUtf16::new(range.end.row, (end + 1).max(0) as u32),
            format!(
                "{}{text}{}",
                String::from_utf16_lossy(&prefix),
                String::from_utf16_lossy(&suffix)
            ),
        );
        // Join ones that intersect or touch (Monaco
        // `TextReplacement.joinReplacements`).
        if let Some(last) = result.last()
            && !(last.0.end < edit.0.start || edit.0.end < last.0.start)
        {
            let last = result.pop().unwrap_or_default();
            let between = if last.0.end <= edit.0.start {
                diff.original_text(&(last.0.end..edit.0.start))
            } else {
                String::new()
            };
            edit = (
                last.0.start..edit.0.end.max(last.0.end),
                format!("{}{between}{}", last.1, edit.1),
            );
        }
        result.push(edit);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(row: u32, column: u32) -> PointUtf16 {
        PointUtf16::new(row, column)
    }

    /// The diffed edits of replacing `range` of `text` by `new`, as
    /// (replaced text, new text) pairs.
    fn diffed(text: &str, range: Range<usize>, new: &str) -> Vec<(String, String)> {
        let rope = Rope::from(text);
        diffed_edits(&rope, range, new)
            .into_iter()
            .map(|(range, new)| (text[range].to_string(), new))
            .collect()
    }

    #[test]
    fn diffing_finds_the_changed_words() {
        let text = "let colour = 1;\nlet b = colour;\n";
        assert_eq!(
            diffed(text, 0..text.len(), "let color = 1;\nlet b = color;\n"),
            vec![
                ("colour".into(), "color".into()),
                ("colour".into(), "color".into())
            ]
        );
        // A pure insertion shrinks back from the grown word.
        assert_eq!(
            diffed("foo(a)", 0..6, "foo(a, b)"),
            vec![(String::new(), ", b".into())]
        );
        // A pure deletion too.
        assert_eq!(
            diffed("foo(a, b)", 0..9, "foo(a)"),
            vec![(", b".into(), String::new())]
        );
    }

    #[test]
    fn inserted_lines_move_before_the_line_break() {
        // Monaco `reshapeMultiLineInsertion`: "b\n" inserted at the start of
        // "c" becomes "\nb" inserted at the end of "a".
        let rope = Rope::from("a\nc");
        let edits = diffed_edits(&rope, 0..3, "a\nb\nc");
        assert_eq!(edits, vec![(1..1, "\nb".to_string())]);
    }

    /// Monaco's `SingleUpdatedNextEdit` behavior.
    #[test]
    fn typing_the_edit_moves_it_along() {
        // Replace "a" (0..1) with "abc": prefix "a" is shared.
        let mut edit = UpdatedEdit::new(0..1, "abc".into(), "a");
        // Typing "b" after the shared prefix.
        edit.apply_changes(&[(1..1, "b".into())]);
        assert!(edit.last_change_updated_edit);
        assert_eq!(edit.edit, Some((0..2, "abc".into())));
        // Typing the rest leaves nothing to do.
        edit.apply_changes(&[(2..2, "c".into())]);
        assert_eq!(edit.edit.as_ref().map(|e| e.0.is_empty()), Some(true));
        // An insertion without a shared prefix moves the start.
        let mut edit = UpdatedEdit::new(5..5, "xyz".into(), "");
        edit.apply_changes(&[(5..5, "x".into())]);
        assert_eq!(edit.edit, Some((6..6, "yz".into())));
        // Changes elsewhere shift it; changes inside it drop it.
        edit.apply_changes(&[(0..0, "--".into())]);
        assert_eq!(edit.edit, Some((8..8, "yz".into())));
        assert!(!edit.last_change_updated_edit);
        let mut edit = UpdatedEdit::new(5..9, "new".into(), "old!");
        edit.apply_changes(&[(6..7, "q".into())]);
        assert_eq!(edit.edit, None);
    }

    #[test]
    fn line_replacements() {
        let rope = Rope::from("ab\ncd\nef");
        // Within a line.
        assert_eq!(
            line_replacement(&rope, p(1, 1)..p(1, 2), "X"),
            (1..2, vec!["cX".to_string()])
        );
        // Lines inserted after a line's end leave that line alone.
        assert_eq!(
            line_replacement(&rope, p(0, 2)..p(0, 2), "\nnew"),
            (1..1, vec!["new".to_string()])
        );
    }

    #[test]
    fn new_ranges_follow_earlier_edits() {
        let edits = vec![
            (p(0, 0)..p(0, 1), "abc".to_string()),
            (p(0, 3)..p(0, 4), "\nz".to_string()),
        ];
        assert_eq!(new_ranges(&edits), vec![p(0, 0)..p(0, 3), p(0, 5)..p(1, 1)]);
    }

    fn view(
        text: &str,
        edits: Vec<(Range<PointUtf16>, String)>,
        cursor: PointUtf16,
    ) -> InlineEditViewKind {
        let rope = Rope::from(text);
        let diff = InlineEditDiff::new(&rope, edits).unwrap();
        determine_view(
            &diff,
            &ViewSettings {
                cursor,
                cursors: vec![cursor],
                code_shifting: InlineEditsCodeShifting::Always,
                render_side_by_side: InlineEditsRenderSideBySide::Auto,
            },
            || true,
        )
    }

    #[test]
    fn view_kinds() {
        use InlineEditViewKind::*;
        let text = "let colour = 1;\nfoo(a)\nx\ny\n";
        // A changed word.
        assert_eq!(
            view(text, vec![(p(0, 4)..p(0, 10), "color".into())], p(3, 0)),
            WordReplacements
        );
        // Text inserted after the cursor.
        assert_eq!(
            view(text, vec![(p(1, 5)..p(1, 5), ", b".into())], p(0, 0)),
            InsertionInline
        );
        // ...or before it.
        assert_eq!(
            view(text, vec![(p(1, 5)..p(1, 5), ", b".into())], p(1, 6)),
            LineReplacement
        );
        // Deleted text.
        assert_eq!(
            view(text, vec![(p(0, 3)..p(0, 10), String::new())], p(3, 0)),
            Deletion
        );
        // Inserted lines.
        assert_eq!(
            view(
                text,
                vec![(p(2, 1)..p(2, 1), "\nnew\nlines".into())],
                p(0, 0)
            ),
            InsertionMultiLine
        );
        // Several changed lines.
        assert_eq!(
            view(
                text,
                vec![(p(2, 0)..p(3, 1), "first\nsecond".into())],
                p(0, 0)
            ),
            SideBySide
        );
        // Several changes on one line.
        let edits = vec![
            (p(0, 0)..p(0, 3), "const".into()),
            (p(0, 4)..p(0, 10), "value".into()),
        ];
        assert_eq!(view(text, edits, p(3, 0)), LineReplacement);
        // With the cursor right below a word replacement, and without code
        // shifting, it moves beside the text (Monaco
        // `_willRenderAboveCursor`).
        let rope = Rope::from(text);
        let diff = InlineEditDiff::new(&rope, vec![(p(0, 4)..p(0, 10), "color".into())]).unwrap();
        let settings = ViewSettings {
            cursor: p(1, 0),
            cursors: vec![p(1, 0)],
            code_shifting: InlineEditsCodeShifting::Never,
            render_side_by_side: InlineEditsRenderSideBySide::Auto,
        };
        assert_eq!(determine_view(&diff, &settings, || true), SideBySide);
    }

    #[test]
    fn growing_edits() {
        let rope = Rope::from("let colour = 1;");
        let diff = InlineEditDiff::new(&rope, vec![(p(0, 8)..p(0, 9), String::new())]).unwrap();
        assert_eq!(
            grow_edits_to_entire_word(&diff, vec![(p(0, 8)..p(0, 9), String::new())]),
            vec![(p(0, 4)..p(0, 10), "color".to_string())]
        );
    }
}
