//! [`ViewLines`]: maps model rows to view lines after hiding folded rows and
//! soft-wrapping (Monaco's `ViewModelLinesFromProjectedModel`).
//!
//! Each model row projects to zero view lines (hidden by a fold) or one view
//! line per wrapped segment. Continuation segments are indented by the
//! wrap's `wrapped_text_indent_length` visible columns; like Monaco, view
//! line content and columns include that indentation as spaces.

use std::ops::Range;
use std::sync::Arc;

use text::{Patch, PointUtf16, Rope, Version};

use super::line_breaks::{
    LineBreakData, MeasureChar, WrapClassifier, WrappingIndent, compute_measured_line_breaks,
};
use super::prefix_sum::PrefixSum;

/// How [`ViewLines`] soft-wraps model rows.
#[derive(Clone, Debug, PartialEq)]
pub struct WrapSettings {
    /// Wrap at this many visible columns.
    pub column: u32,
    /// Width of a tab character in visible columns.
    pub tab_size: u32,
    /// Indentation of continuation lines (Monaco `editor.wrappingIndent`).
    pub indent: WrappingIndent,
    /// Don't break between CJK characters (Monaco `editor.wordBreak:
    /// keepAll`).
    pub keep_all: bool,
    /// Which characters lines may break before and after (Monaco
    /// `wordWrapBreakBeforeCharacters`/`wordWrapBreakAfterCharacters`).
    pub classifier: Arc<WrapClassifier>,
    /// Also break after literal `\n`s in lines with a quote (Monaco
    /// `wrapOnEscapedLineFeeds`).
    pub wrap_on_escaped_line_feeds: bool,
    /// Measured character widths (Monaco `wrappingStrategy: advanced`);
    /// `None` counts monospace columns.
    pub measure: Option<MeasuredWidths>,
}

/// Shared character widths for [`WrapSettings::measure`]. Equal when they
/// are the same widths.
#[derive(Clone)]
pub struct MeasuredWidths(pub Arc<dyn MeasureChar>);

impl PartialEq for MeasuredWidths {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for MeasuredWidths {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MeasuredWidths")
    }
}

/// Hidden row ranges sorted, with empty ones dropped and overlapping or
/// adjacent ones merged (the form [`ViewLines::hidden`] returns).
pub fn normalize_hidden(mut hidden: Vec<Range<u32>>) -> Vec<Range<u32>> {
    hidden.retain(|r| !r.is_empty());
    hidden.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<u32>> = Vec::new();
    for range in hidden {
        match merged.last_mut() {
            Some(last) if last.end >= range.start => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// One view line: a segment of a model row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewLine {
    /// The model row.
    pub row: u32,
    /// Index of the wrapped segment within the row; 0 for the first.
    pub segment: u32,
}

/// A view line's place in its model row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentInfo {
    /// UTF-16 columns of the model row shown on this view line.
    pub columns: Range<u32>,
    /// Indentation (spaces) prepended to continuation lines.
    pub indent: u32,
    /// Visible column of `columns.start` in the model row (for tab stops).
    pub start_visible_column: u32,
    /// True if this is the row's last segment.
    pub is_last: bool,
}

/// The mapping from model rows to view lines for one snapshot of the text,
/// given the hidden (folded) rows and the wrap settings.
#[derive(Clone)]
pub struct ViewLines {
    rope: Rope,
    version: Version,
    wrap: Option<WrapSettings>,
    breaks: Vec<Option<Arc<LineBreakData>>>,
    /// Sorted, disjoint hidden model row ranges.
    hidden: Vec<Range<u32>>,
    counts: PrefixSum,
}

impl ViewLines {
    /// View lines for `rope` at `version`, with no wrapping and nothing
    /// hidden.
    pub fn new(rope: Rope, version: Version) -> Self {
        let mut this = Self {
            breaks: vec![None; rope.line_count() as usize],
            rope,
            version,
            wrap: None,
            hidden: Vec::new(),
            counts: PrefixSum::default(),
        };
        this.rebuild_counts();
        this
    }

    /// The text version this mapping is for.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The text this mapping is for.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// The wrap settings; `None` when wrapping is off.
    pub fn wrap(&self) -> Option<&WrapSettings> {
        self.wrap.as_ref()
    }

    /// The hidden model row ranges, sorted and disjoint (see
    /// [`normalize_hidden`]).
    pub fn hidden(&self) -> &[Range<u32>] {
        &self.hidden
    }

    fn compute_row(&self, row: u32) -> Option<Arc<LineBreakData>> {
        let wrap = self.wrap.as_ref()?;
        compute_measured_line_breaks(
            &wrap.classifier,
            &self.rope.line(row),
            wrap.tab_size,
            Some(wrap.column),
            wrap.indent,
            wrap.keep_all,
            wrap.wrap_on_escaped_line_feeds,
            wrap.measure.as_ref().map(|m| &*m.0),
        )
        .map(Arc::new)
    }

    fn rebuild_counts(&mut self) {
        let mut counts: Vec<u32> = self
            .breaks
            .iter()
            .map(|b| b.as_ref().map_or(1, |b| b.output_line_count() as u32))
            .collect();
        for range in &self.hidden {
            for row in range.clone() {
                if let Some(count) = counts.get_mut(row as usize) {
                    *count = 0;
                }
            }
        }
        self.counts = PrefixSum::new(counts);
    }

    /// Changes the wrap settings, rewrapping every row if they differ.
    pub fn set_wrap(&mut self, wrap: Option<WrapSettings>) {
        if self.wrap == wrap {
            return;
        }
        self.wrap = wrap;
        self.breaks = match &self.wrap {
            None => vec![None; self.rope.line_count() as usize],
            Some(wrap) => {
                let mut breaks = Vec::with_capacity(self.rope.line_count() as usize);
                self.rope.for_each_line(0..self.rope.len(), |_, line| {
                    breaks.push(
                        compute_measured_line_breaks(
                            &wrap.classifier,
                            line,
                            wrap.tab_size,
                            Some(wrap.column),
                            wrap.indent,
                            wrap.keep_all,
                            wrap.wrap_on_escaped_line_feeds,
                            wrap.measure.as_ref().map(|m| &*m.0),
                        )
                        .map(Arc::new),
                    );
                    true
                });
                breaks
            }
        };
        self.rebuild_counts();
    }

    /// Changes which characters wrapping may break around (the wrap
    /// settings' [`classifier`](WrapSettings::classifier)), rewrapping every
    /// row.
    pub fn set_classifier(&mut self, classifier: WrapClassifier) {
        let wrap = self.wrap.clone().map(|wrap| WrapSettings {
            classifier: Arc::new(classifier),
            ..wrap
        });
        self.set_wrap(wrap);
    }

    /// Sets the hidden (folded) model rows. Ranges are normalized.
    pub fn set_hidden(&mut self, hidden: Vec<Range<u32>>) {
        let merged = normalize_hidden(hidden);
        if merged != self.hidden {
            self.hidden = merged;
            self.rebuild_counts();
        }
    }

    /// Catches up with `rope` at `version`, given the edits since our version.
    pub fn apply_edits(&mut self, rope: Rope, version: Version, patch: &Patch) {
        let old = std::mem::replace(&mut self.rope, rope);
        self.version = version;
        // Row ranges (old rows, new rows) touched by each edit, merged.
        let mut spans: Vec<(Range<u32>, Range<u32>)> = Vec::new();
        for edit in patch.edits() {
            let old_rows =
                old.offset_to_point(edit.old.start).row..old.offset_to_point(edit.old.end).row + 1;
            let new_rows = self.rope.offset_to_point(edit.new.start).row
                ..self.rope.offset_to_point(edit.new.end).row + 1;
            match spans.last_mut() {
                Some((last_old, last_new)) if old_rows.start < last_old.end => {
                    last_old.end = last_old.end.max(old_rows.end);
                    last_new.end = last_new.end.max(new_rows.end);
                }
                _ => spans.push((old_rows, new_rows)),
            }
        }
        // Apply back to front so earlier indices stay valid.
        for (old_rows, new_rows) in spans.iter().rev() {
            let replacement: Vec<Option<Arc<LineBreakData>>> =
                new_rows.clone().map(|row| self.compute_row(row)).collect();
            let start = old_rows.start as usize;
            let end = (old_rows.end as usize).min(self.breaks.len());
            self.breaks.splice(start..end, replacement);
        }
        // Shift hidden ranges by the row delta of edits above them; ranges
        // touched by an edit are dropped (the caller re-derives folds).
        let mut hidden = Vec::new();
        for range in &self.hidden {
            let mut delta: i64 = 0;
            let mut keep = true;
            for (old_rows, new_rows) in &spans {
                let old_len = old_rows.len() as i64;
                let new_len = new_rows.len() as i64;
                if old_rows.end <= range.start {
                    delta += new_len - old_len;
                } else if old_rows.start < range.end {
                    keep = false;
                }
            }
            if keep {
                hidden.push((range.start as i64 + delta) as u32..(range.end as i64 + delta) as u32);
            }
        }
        let unchanged_rows = hidden.len() == self.hidden.len()
            && spans.iter().all(|(old, new)| old.len() == new.len());
        self.hidden = hidden;
        debug_assert_eq!(self.breaks.len(), self.rope.line_count() as usize);
        if unchanged_rows {
            // Typing within lines: update just the edited rows' counts.
            for (_, rows) in &spans {
                for row in rows.clone() {
                    let count = if self.hidden.iter().any(|hidden| hidden.contains(&row)) {
                        0
                    } else {
                        self.breaks[row as usize]
                            .as_ref()
                            .map_or(1, |b| b.output_line_count() as u32)
                    };
                    self.counts.set(row as usize, count);
                }
            }
        } else {
            self.rebuild_counts();
        }
    }

    /// Number of view lines.
    pub fn line_count(&self) -> u32 {
        self.counts.total() as u32
    }

    /// True if model `row` is hidden by a fold. Rows past the end are not
    /// hidden.
    pub fn is_row_hidden(&self, row: u32) -> bool {
        self.counts.len() > row as usize && self.counts.value(row as usize) == 0
    }

    /// The view line at `index` (clamped).
    pub fn view_line(&self, index: u32) -> ViewLine {
        let (row, segment) = self.counts.find(index as u64);
        ViewLine {
            row: row as u32,
            segment,
        }
    }

    /// Index of the first view line of `row`, or of the next visible row if
    /// `row` is hidden.
    pub fn first_view_line_of_row(&self, row: u32) -> u32 {
        self.counts.prefix(row as usize) as u32
    }

    /// Which part of its model row `line` shows. A segment past the row's
    /// last is clamped to the last.
    pub fn segment_info(&self, line: ViewLine) -> SegmentInfo {
        match self.breaks.get(line.row as usize).and_then(|b| b.as_ref()) {
            None => SegmentInfo {
                columns: 0..self.rope.line_len_utf16(line.row),
                indent: 0,
                start_visible_column: 0,
                is_last: true,
            },
            Some(data) => {
                let segment = (line.segment as usize).min(data.output_line_count() - 1);
                let columns = data.input_range(segment);
                SegmentInfo {
                    columns,
                    indent: if segment > 0 {
                        data.wrapped_text_indent_length
                    } else {
                        0
                    },
                    start_visible_column: if segment > 0 {
                        data.break_offsets_visible_column[segment - 1]
                    } else {
                        0
                    },
                    is_last: segment + 1 == data.output_line_count(),
                }
            }
        }
    }

    /// The view line and view column showing model point `point`. Points at
    /// a wrap boundary show at the start of the next segment unless
    /// `prefer_previous`.
    pub fn to_view(&self, point: PointUtf16, prefer_previous: bool) -> (u32, u32) {
        let row = point.row.min(self.rope.max_point_utf16().row);
        let mut row = row;
        // Points in hidden rows show at the end of the fold's start row.
        if self.is_row_hidden(row) {
            while row > 0 && self.is_row_hidden(row) {
                row -= 1;
            }
            let first = self.first_view_line_of_row(row);
            let count = self.counts.value(row as usize).max(1);
            let last = ViewLine {
                row,
                segment: count - 1,
            };
            let info = self.segment_info(last);
            return (first + count - 1, info.indent + info.columns.len() as u32);
        }
        let first = self.first_view_line_of_row(row);
        match self.breaks.get(row as usize).and_then(|b| b.as_ref()) {
            None => (first, point.column),
            Some(data) => {
                let segment = data.output_line_for(point.column, prefer_previous);
                let columns = data.input_range(segment);
                let indent = if segment > 0 {
                    data.wrapped_text_indent_length
                } else {
                    0
                };
                (
                    first + segment as u32,
                    indent + point.column.saturating_sub(columns.start),
                )
            }
        }
    }

    /// The model point shown at view `line`, view `column`. Columns inside the
    /// continuation indent map to the segment start.
    pub fn to_model(&self, line: u32, column: u32) -> PointUtf16 {
        let view_line = self.view_line(line);
        let info = self.segment_info(view_line);
        let column = column.saturating_sub(info.indent);
        let column = (info.columns.start + column).min(info.columns.end);
        PointUtf16::new(view_line.row, column)
    }

    /// The largest view column on view line `line` (Monaco
    /// `getLineMaxColumn` on the view model): its continuation indentation
    /// plus the columns it shows. The end of a wrapped segment is the same
    /// model position as the start of the next.
    pub fn line_max_column(&self, line: u32) -> u32 {
        let info = self.segment_info(self.view_line(line));
        info.indent + info.columns.len() as u32
    }

    /// Monaco `ModelLineProjectionData.normalizeOutputPosition` for view
    /// position `point` (view line, view column): with `right` affinity the
    /// end of a wrapped segment becomes the start of the next (after its
    /// indentation); otherwise the start of a continuation segment becomes
    /// the end of the previous one. Both name the same model position.
    pub fn normalize_view_position(&self, point: PointUtf16, right: bool) -> PointUtf16 {
        let line = self.view_line(point.row);
        let info = self.segment_info(line);
        if right {
            if !info.is_last && point.column >= info.indent + info.columns.len() as u32 {
                let next = self.segment_info(ViewLine {
                    row: line.row,
                    segment: line.segment + 1,
                });
                return PointUtf16::new(point.row + 1, next.indent);
            }
        } else if line.segment > 0 && point.column <= info.indent {
            return PointUtf16::new(point.row - 1, self.line_max_column(point.row - 1));
        }
        point
    }

    /// View line content, with continuation indentation as spaces.
    pub fn line_content(&self, line: u32) -> String {
        let view_line = self.view_line(line);
        let info = self.segment_info(view_line);
        let text = self.rope.line(view_line.row);
        let start = utf16_to_byte(&text, info.columns.start);
        let end = utf16_to_byte(&text, info.columns.end);
        let mut content = " ".repeat(info.indent as usize);
        content.push_str(&text[start..end]);
        content
    }
}

pub(crate) fn utf16_to_byte(text: &str, column: u32) -> usize {
    let mut units = 0u32;
    for (ix, ch) in text.char_indices() {
        if units >= column {
            return ix;
        }
        units += ch.len_utf16() as u32;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use text::Buffer;

    fn wrap(column: u32) -> Option<WrapSettings> {
        Some(WrapSettings {
            column,
            tab_size: 4,
            indent: WrappingIndent::Same,
            keep_all: false,
            classifier: Arc::default(),
            wrap_on_escaped_line_feeds: false,
            measure: None,
        })
    }

    #[test]
    fn folds_and_wraps() {
        let rope = Rope::from("aaaa bbbb cccc\nshort\n  dddd eeee ffff\nend");
        let mut lines = ViewLines::new(rope, 0);
        assert_eq!(lines.line_count(), 4);
        lines.set_wrap(wrap(10));
        // "aaaa bbbb |cccc", "short", "  dddd |  eeee |  ffff", "end"
        assert_eq!(lines.line_count(), 7);
        assert_eq!(lines.line_content(1), "cccc");
        assert_eq!(lines.line_content(4), "  eeee ");
        assert_eq!(lines.view_line(4), ViewLine { row: 2, segment: 1 });
        assert_eq!(lines.to_view(PointUtf16::new(2, 9), false), (4, 4));
        assert_eq!(lines.to_model(4, 4), PointUtf16::new(2, 9));
        assert_eq!(lines.to_model(4, 0), PointUtf16::new(2, 7));
        // A point at a wrap boundary.
        assert_eq!(lines.to_view(PointUtf16::new(0, 10), false), (1, 0));
        assert_eq!(lines.to_view(PointUtf16::new(0, 10), true), (0, 10));

        lines.set_hidden(std::iter::once(1..3).collect());
        assert_eq!(lines.line_count(), 3);
        assert_eq!(lines.view_line(2), ViewLine { row: 3, segment: 0 });
        assert!(lines.is_row_hidden(2));
        assert_eq!(lines.to_view(PointUtf16::new(2, 3), false), (1, 4));
    }

    #[test]
    fn incremental_edits_match_full_rebuild() {
        let mut buffer = Buffer::new("one two three four\nfive\nsix seven eight nine ten\n\tx");
        let mut lines = ViewLines::new(buffer.rope().clone(), buffer.version());
        lines.set_wrap(wrap(8));
        lines.set_hidden(std::iter::once(3..4).collect());
        // In-line edits take the incremental count update; the rest rebuild.
        let edits: [(Range<usize>, &str); 7] = [
            (3..3, " and more words"),
            (1..1, "abcdefghijklmnopqrstuvwxyz"),
            (10..20, "\n\nnew lines here\n"),
            (0..0, "x"),
            (5..9, ""),
            (2..6, "y"),
            (0..0, "z"),
        ];
        for (range, text) in edits {
            buffer.edit([(range, text)]).unwrap();
            let patch = buffer.edits_since(lines.version());
            lines.apply_edits(buffer.rope().clone(), buffer.version(), &patch);
            let mut fresh = ViewLines::new(buffer.rope().clone(), buffer.version());
            fresh.set_wrap(wrap(8));
            fresh.set_hidden(lines.hidden().to_vec());
            assert_eq!(lines.line_count(), fresh.line_count());
            for line in 0..lines.line_count() {
                assert_eq!(lines.line_content(line), fresh.line_content(line));
                assert_eq!(lines.view_line(line), fresh.view_line(line));
            }
            for row in 0..buffer.rope().line_count() {
                assert_eq!(lines.is_row_hidden(row), fresh.is_row_hidden(row));
            }
        }
    }
}
