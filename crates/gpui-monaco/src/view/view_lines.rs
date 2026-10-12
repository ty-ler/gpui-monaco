//! [`ViewLines`]: maps model rows to view lines after hiding folded rows,
//! injecting text and soft-wrapping (Monaco's
//! `ViewModelLinesFromProjectedModel`).
//!
//! Each model row projects to zero view lines (hidden by a fold) or one view
//! line per wrapped segment, through its `ModelLineProjectionData` (Monaco's
//! `ModelLineProjection`) when it has injected text or wraps. As in Monaco,
//! view columns count the injected text (inlay hints, `before`/`after`
//! decorations, ghost text) and the continuation indentation of wrapped
//! segments, `wrapped_text_indent_length` spaces.

use std::ops::Range;
use std::sync::Arc;

use text::{Patch, PointUtf16, Rope, Version};

use super::injected_text::{LineInjectedText, ModelLineProjectionData, PositionAffinity};
use super::line_breaks::{MeasureChar, WrapClassifier, WrappingIndent, create_line_breaks};
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
    /// UTF-16 columns of the model row shown on this view line. A segment
    /// that starts or ends inside injected text starts or ends at the
    /// column the text is injected at.
    pub columns: Range<u32>,
    /// Indentation (spaces) prepended to continuation lines.
    pub indent: u32,
    /// Visible column of `columns.start` in the model row, injected text
    /// included (for tab stops).
    pub start_visible_column: u32,
    /// True if this is the row's last segment.
    pub is_last: bool,
    /// The UTF-16 offsets of the row with its injected text (Monaco's
    /// offsets in input with injections) shown on this view line: `columns`
    /// when the row has no injected text. The view line's columns are
    /// `indent` plus these, less `offsets.start`.
    pub offsets: Range<u32>,
}

/// The mapping from model rows to view lines for one snapshot of the text,
/// given the hidden (folded) rows, the injected text and the wrap settings.
#[derive(Clone)]
pub struct ViewLines {
    rope: Rope,
    version: Version,
    wrap: Option<WrapSettings>,
    /// Each row's projection (Monaco `ModelLineProjectionData`); `None`
    /// when it shows as is, on one view line.
    projections: Vec<Option<Arc<ModelLineProjectionData>>>,
    /// The rows with injected text, sorted, and their text by column
    /// (Monaco `LineInjectedText`). Few rows have any, so edits shift
    /// them cheaply.
    injected: Vec<(u32, Arc<[LineInjectedText]>)>,
    /// Sorted, disjoint hidden model row ranges.
    hidden: Vec<Range<u32>>,
    counts: PrefixSum,
    /// Unique to this mapping: a new one with every change.
    generation: u64,
}

/// The next [`ViewLines::generation`].
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The projection of `line` with `injected` text, wrapped as `wrap` says
/// (Monaco's line breaks computer).
fn project(
    wrap: &WrapSettings,
    line: &str,
    injected: &[LineInjectedText],
) -> Option<Arc<ModelLineProjectionData>> {
    create_line_breaks(
        &wrap.classifier,
        line,
        injected,
        wrap.tab_size,
        Some(wrap.column),
        wrap.indent,
        wrap.keep_all,
        wrap.wrap_on_escaped_line_feeds,
        wrap.measure.as_ref().map(|m| &*m.0),
    )
    .map(Arc::new)
}

impl ViewLines {
    /// View lines for `rope` at `version`, with no wrapping, injected text
    /// or anything hidden.
    pub fn new(rope: Rope, version: Version) -> Self {
        let rows = rope.line_count() as usize;
        let mut this = Self {
            projections: vec![None; rows],
            injected: Vec::new(),
            rope,
            version,
            wrap: None,
            hidden: Vec::new(),
            counts: PrefixSum::default(),
            generation: next_generation(),
        };
        this.rebuild_counts();
        this
    }

    /// The text version this mapping is for.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Identifies this mapping: it changes with every edit, wrap, fold and
    /// injected text change, and no other mapping has it, so views can
    /// cache what they derive.
    pub fn generation(&self) -> u64 {
        self.generation
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

    /// The text injected into model `row`, by column; empty without any.
    pub fn injected_text(&self, row: u32) -> &[LineInjectedText] {
        match self.injected.binary_search_by_key(&row, |(row, _)| *row) {
            Ok(ix) => &self.injected[ix].1,
            Err(_) => &[],
        }
    }

    /// The model rows with injected text, in order.
    pub fn rows_with_injected_text(&self) -> impl Iterator<Item = u32> + '_ {
        self.injected.iter().map(|(row, _)| *row)
    }

    /// Model `row`'s projection; `None` when it shows as is.
    pub fn projection(&self, row: u32) -> Option<&ModelLineProjectionData> {
        self.projections.get(row as usize)?.as_deref()
    }

    /// Model `row`'s projection with `injected` text instead of its own.
    fn compute_row(
        &self,
        row: u32,
        injected: &[LineInjectedText],
    ) -> Option<Arc<ModelLineProjectionData>> {
        match &self.wrap {
            Some(wrap) => project(wrap, &self.rope.line(row), injected),
            None if injected.is_empty() => None,
            None => Some(Arc::new(ModelLineProjectionData::unwrapped(
                self.rope.line_len_utf16(row),
                LineInjectedText::injections(injected),
            ))),
        }
    }

    /// How many view lines model `row` shows, unless hidden.
    fn row_line_count(&self, row: usize) -> u32 {
        self.projections[row]
            .as_ref()
            .map_or(1, |p| p.output_line_count())
    }

    /// How many view lines model `rows` show once shown, hidden or not
    /// (wrapped rows count each part).
    pub fn unhidden_line_count(&self, rows: Range<u32>) -> u32 {
        let end = (rows.end as usize).min(self.projections.len());
        (rows.start as usize..end)
            .map(|row| self.row_line_count(row))
            .sum()
    }

    fn rebuild_counts(&mut self) {
        self.generation = next_generation();
        let mut counts: Vec<u32> = (0..self.projections.len())
            .map(|row| self.row_line_count(row))
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
        self.projections = match &self.wrap {
            None => {
                let mut projections = vec![None; self.rope.line_count() as usize];
                for (row, injected) in &self.injected {
                    projections[*row as usize] = self.compute_row(*row, injected);
                }
                projections
            }
            Some(wrap) => {
                let mut projections = Vec::with_capacity(self.rope.line_count() as usize);
                let mut injected = self.injected.iter().peekable();
                self.rope.for_each_line(0..self.rope.len(), |_, line| {
                    let row = projections.len() as u32;
                    let text = match injected.next_if(|(r, _)| *r == row) {
                        Some((_, text)) => &text[..],
                        None => &[],
                    };
                    projections.push(project(wrap, line, text));
                    true
                });
                projections
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

    /// Sets the text injected into each of `rows` (sorted by column; `None`
    /// or empty for none) and rewraps the rows whose injected text changed
    /// (Monaco's `onModelLinesChanged` with injected text). Returns whether
    /// any did.
    pub fn set_injected_text(
        &mut self,
        rows: impl IntoIterator<Item = (u32, Option<Arc<[LineInjectedText]>>)>,
    ) -> bool {
        let mut changed = false;
        for (row, text) in rows {
            let ix = row as usize;
            if ix >= self.projections.len() {
                continue;
            }
            let text = text.filter(|text| !text.is_empty());
            let at = self.injected.binary_search_by_key(&row, |(row, _)| *row);
            match (at, text) {
                (Err(_), None) => continue,
                (Ok(at), Some(text)) if self.injected[at].1 == text => continue,
                (Ok(at), Some(text)) => self.injected[at].1 = text,
                (Err(at), Some(text)) => self.injected.insert(at, (row, text)),
                (Ok(at), None) => {
                    self.injected.remove(at);
                }
            }
            let projection = self.compute_row(row, self.injected_text(row));
            self.projections[ix] = projection;
            if !self.is_row_hidden(row) {
                let count = self.row_line_count(ix);
                if self.counts.value(ix) != count {
                    self.counts.set(ix, count);
                }
            }
            changed = true;
        }
        if changed {
            self.generation = next_generation();
        }
        changed
    }

    /// Catches up with `rope` at `version`, given the edits since our version.
    /// Edited rows lose their injected text; set it again with
    /// [`ViewLines::set_injected_text`].
    pub fn apply_edits(&mut self, rope: Rope, version: Version, patch: &Patch) {
        self.generation = next_generation();
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
            let replacement: Vec<Option<Arc<ModelLineProjectionData>>> = new_rows
                .clone()
                .map(|row| self.compute_row(row, &[]))
                .collect();
            let start = old_rows.start as usize;
            let end = (old_rows.end as usize).min(self.projections.len());
            self.projections.splice(start..end, replacement);
        }
        // Injected text moves with its row, and edited rows lose theirs.
        if !self.injected.is_empty() {
            self.injected.retain_mut(|(row, _)| {
                let mut delta: i64 = 0;
                for (old_rows, new_rows) in &spans {
                    if old_rows.end <= *row {
                        delta += new_rows.len() as i64 - old_rows.len() as i64;
                    } else if old_rows.start <= *row {
                        return false;
                    }
                }
                *row = (*row as i64 + delta) as u32;
                true
            });
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
        debug_assert_eq!(self.projections.len(), self.rope.line_count() as usize);
        if unchanged_rows {
            // Typing within lines: update just the edited rows' counts.
            for (_, rows) in &spans {
                for row in rows.clone() {
                    let count = if self.hidden.iter().any(|hidden| hidden.contains(&row)) {
                        0
                    } else {
                        self.row_line_count(row as usize)
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

    /// `line`'s row's projection and its segment (clamped to the last).
    fn projected(&self, line: ViewLine) -> Option<(&ModelLineProjectionData, u32)> {
        let projection = self.projection(line.row)?;
        Some((
            projection,
            line.segment.min(projection.output_line_count() - 1),
        ))
    }

    /// Which part of its model row `line` shows. A segment past the row's
    /// last is clamped to the last.
    pub fn segment_info(&self, line: ViewLine) -> SegmentInfo {
        match self.projected(line) {
            None => {
                let len = self.rope.line_len_utf16(line.row);
                SegmentInfo {
                    columns: 0..len,
                    indent: 0,
                    start_visible_column: 0,
                    is_last: true,
                    offsets: 0..len,
                }
            }
            Some((projection, segment)) => {
                let offsets = projection.output_range(segment);
                SegmentInfo {
                    columns: projection.input_offset(offsets.start)
                        ..projection.input_offset(offsets.end),
                    indent: projection.min_output_offset(segment),
                    start_visible_column: projection.start_visible_column(segment),
                    is_last: segment + 1 == projection.output_line_count(),
                    offsets,
                }
            }
        }
    }

    /// The view line and view column showing model point `point`, before
    /// any injected text there. Points at a wrap boundary show at the start
    /// of the next segment unless `prefer_previous` (Monaco
    /// `convertModelPositionToViewPosition` with `PositionAffinity.None` or
    /// `Left`).
    pub fn to_view(&self, point: PointUtf16, prefer_previous: bool) -> (u32, u32) {
        let affinity = if prefer_previous {
            PositionAffinity::Left
        } else {
            PositionAffinity::None
        };
        self.to_view_with_affinity(point, affinity)
    }

    /// Monaco `convertModelPositionToViewPosition`: the view line and view
    /// column showing model point `point`. At injected text, `Right` shows
    /// it after the text and anything else before; at a wrap boundary,
    /// `Left` shows it at the end of the first segment and anything else at
    /// the start of the next.
    pub fn to_view_with_affinity(
        &self,
        point: PointUtf16,
        affinity: PositionAffinity,
    ) -> (u32, u32) {
        let row = point.row.min(self.rope.max_point_utf16().row);
        let mut row = row;
        // Points in hidden rows show at the end of the fold's start row.
        if self.is_row_hidden(row) {
            while row > 0 && self.is_row_hidden(row) {
                row -= 1;
            }
            let first = self.first_view_line_of_row(row);
            let count = self.counts.value(row as usize).max(1);
            let line = first + count - 1;
            return (line, self.line_max_column(line));
        }
        let first = self.first_view_line_of_row(row);
        match self.projection(row) {
            None => (first, point.column),
            Some(projection) => {
                let output = projection.translate_to_output_position(point.column, affinity);
                (first + output.output_line_index, output.output_offset)
            }
        }
    }

    /// The model point shown at view `line`, view `column` (Monaco
    /// `convertViewPositionToModelPosition`). Columns inside the
    /// continuation indent map to the segment start, and columns inside
    /// injected text to the column it's injected at.
    pub fn to_model(&self, line: u32, column: u32) -> PointUtf16 {
        let view_line = self.view_line(line);
        match self.projected(view_line) {
            None => PointUtf16::new(
                view_line.row,
                column.min(self.rope.line_len_utf16(view_line.row)),
            ),
            Some((projection, segment)) => {
                let column = column.min(projection.max_output_offset(segment));
                PointUtf16::new(
                    view_line.row,
                    projection.translate_to_input_offset(segment, column),
                )
            }
        }
    }

    /// The smallest view column on view line `line` (Monaco
    /// `getLineMinColumn` on the view model): its continuation indentation.
    pub fn line_min_column(&self, line: u32) -> u32 {
        self.projected(self.view_line(line))
            .map_or(0, |(projection, segment)| {
                projection.min_output_offset(segment)
            })
    }

    /// The largest view column on view line `line` (Monaco
    /// `getLineMaxColumn` on the view model): its continuation indentation
    /// plus the text and injected text it shows. The end of a wrapped
    /// segment is the same model position as the start of the next.
    pub fn line_max_column(&self, line: u32) -> u32 {
        let view_line = self.view_line(line);
        match self.projected(view_line) {
            None => self.rope.line_len_utf16(view_line.row),
            Some((projection, segment)) => projection.max_output_offset(segment),
        }
    }

    /// Monaco `normalizePosition` on the view model: view position `point`
    /// (view line, view column) moved out of injected text as `affinity`
    /// and the text's cursor stops say, and with `Left` or `Right`
    /// affinity off a wrap point's other side (with `Right` the end of a
    /// wrapped segment becomes the start of the next, after its
    /// indentation; with `Left` the start of a continuation segment becomes
    /// the end of the previous one, the same model position).
    pub fn normalize_position(&self, point: PointUtf16, affinity: PositionAffinity) -> PointUtf16 {
        let line = self.view_line(point.row);
        match self.projected(line) {
            None => point,
            Some((projection, segment)) => {
                let first = point.row - line.segment;
                let output = projection.normalize_output_position(segment, point.column, affinity);
                PointUtf16::new(first + output.output_line_index, output.output_offset)
            }
        }
    }

    /// [`ViewLines::normalize_position`] with `Right` affinity if `right`,
    /// else `Left`.
    pub fn normalize_view_position(&self, point: PointUtf16, right: bool) -> PointUtf16 {
        let affinity = if right {
            PositionAffinity::Right
        } else {
            PositionAffinity::Left
        };
        self.normalize_position(point, affinity)
    }

    /// The injected text at view position `point` (Monaco
    /// `getInjectedTextAt`), by index into
    /// [`ViewLines::injected_text`] of its row.
    pub fn injected_text_at(&self, point: PointUtf16) -> Option<usize> {
        let (projection, segment) = self.projected(self.view_line(point.row))?;
        projection.injected_text(segment, point.column)
    }

    /// View line content (Monaco `getViewLineContent`), with continuation
    /// indentation as spaces and the injected text it shows.
    pub fn line_content(&self, line: u32) -> String {
        let view_line = self.view_line(line);
        let Some((projection, segment)) = self.projected(view_line) else {
            return self.rope.line(view_line.row);
        };
        let mut text = self.rope.line(view_line.row);
        let injected = self.injected_text(view_line.row);
        if !injected.is_empty() {
            text = LineInjectedText::apply(&text, injected);
        }
        let offsets = projection.output_range(segment);
        let start = utf16_to_byte(&text, offsets.start);
        let end = utf16_to_byte(&text, offsets.end);
        let indent = projection.min_output_offset(segment) as usize;
        let mut content = String::with_capacity(indent + end - start);
        content.extend(std::iter::repeat_n(' ', indent));
        content.push_str(&text[start..end]);
        content
    }

    /// The model text on view line `line`, without its injected text, with
    /// continuation indentation as spaces: the line in "segment columns",
    /// `indent` plus model columns from the segment's start.
    pub fn segment_content(&self, line: u32) -> String {
        let view_line = self.view_line(line);
        let info = self.segment_info(view_line);
        let text = self.rope.line(view_line.row);
        let start = utf16_to_byte(&text, info.columns.start);
        let end = utf16_to_byte(&text, info.columns.end);
        let mut content = " ".repeat(info.indent as usize);
        content.push_str(&text[start..end]);
        content
    }

    /// The largest segment column on view line `line` (see
    /// [`ViewLines::segment_content`]).
    pub fn segment_max_column(&self, line: u32) -> u32 {
        let info = self.segment_info(self.view_line(line));
        info.indent + info.columns.len() as u32
    }

    /// The segment column (see [`ViewLines::segment_content`]) of view
    /// `column` on view line `line`.
    pub fn view_to_segment_column(&self, line: u32, column: u32) -> u32 {
        let info = self.segment_info(self.view_line(line));
        let model = self.to_model(line, column).column;
        info.indent + model.clamp(info.columns.start, info.columns.end) - info.columns.start
    }

    /// The view column of segment `column` (see
    /// [`ViewLines::segment_content`]) on view line `line`: before any
    /// injected text there, and on this view line.
    pub fn segment_to_view_column(&self, line: u32, column: u32) -> u32 {
        let view_line = self.view_line(line);
        let Some((projection, segment)) = self.projected(view_line) else {
            return column;
        };
        let info = self.segment_info(view_line);
        let model = (info.columns.start + column.saturating_sub(info.indent)).min(info.columns.end);
        for affinity in [PositionAffinity::None, PositionAffinity::Left] {
            let output = projection.translate_to_output_position(model, affinity);
            if output.output_line_index == segment {
                return output.output_offset;
            }
        }
        if model <= info.columns.start {
            projection.min_output_offset(segment)
        } else {
            projection.max_output_offset(segment)
        }
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

    fn injected(column: u32, content: &str) -> LineInjectedText {
        LineInjectedText {
            column,
            content: content.to_string().into(),
            cursor_stops: crate::decorations::InjectedTextCursorStops::Both,
        }
    }

    fn set_injected(lines: &mut ViewLines, row: u32, texts: Vec<LineInjectedText>) -> bool {
        lines.set_injected_text([(row, Some(texts.into()))])
    }

    /// Monaco `modelLineProjection.test.ts` (`SplitLinesCollection`): the
    /// text of its `getViewLinesData` tests.
    const NICE: &str = "class Nice {\n\tfunction hi() {\n\t\tconsole.log(\"Hello world\");\n\t}\n\tfunction hello() {\n\t\tconsole.log(\"Hello world, this is a somewhat longer line\");\n\t}\n}";

    /// Monaco's `withSplitLinesCollection` with `wordWrapColumn` 30 and
    /// `wrappingIndent: 'indent'`.
    fn nice_lines() -> ViewLines {
        let mut lines = ViewLines::new(Rope::from(NICE), 0);
        lines.set_wrap(Some(WrapSettings {
            indent: WrappingIndent::Indent,
            ..wrap(30).unwrap()
        }));
        lines
    }

    /// (content, min column, max column) of each view line, 1-based as in
    /// Monaco's tests.
    fn view_lines_data(lines: &ViewLines) -> Vec<(String, u32, u32)> {
        (0..lines.line_count())
            .map(|line| {
                (
                    lines.line_content(line),
                    lines.line_min_column(line) + 1,
                    lines.line_max_column(line) + 1,
                )
            })
            .collect()
    }

    fn data(expected: &[(&str, u32, u32)]) -> Vec<(String, u32, u32)> {
        expected
            .iter()
            .map(|(content, min, max)| (content.to_string(), *min, *max))
            .collect()
    }

    /// Monaco `getViewLinesData - with wrapping`.
    #[test]
    fn get_view_lines_data_with_wrapping() {
        let lines = nice_lines();
        assert_eq!(lines.line_count(), 12);
        assert_eq!(
            view_lines_data(&lines),
            data(&[
                ("class Nice {", 1, 13),
                ("\tfunction hi() {", 1, 17),
                ("\t\tconsole.log(\"Hello ", 1, 22),
                ("            world\");", 13, 21),
                ("\t}", 1, 3),
                ("\tfunction hello() {", 1, 20),
                ("\t\tconsole.log(\"Hello ", 1, 22),
                ("            world, this is a ", 13, 30),
                ("            somewhat longer ", 13, 29),
                ("            line\");", 13, 20),
                ("\t}", 1, 3),
                ("}", 1, 2),
            ])
        );
    }

    /// Monaco `getViewLinesData - with wrapping and injected text`: injected
    /// text counts when wrapping, and wraps mid-text.
    #[test]
    fn get_view_lines_data_with_wrapping_and_injected_text() {
        let mut lines = nice_lines();
        assert!(set_injected(
            &mut lines,
            0,
            vec![injected(
                8,
                "very very long injected text that causes a line break"
            )]
        ));
        assert_eq!(lines.line_count(), 14);
        assert_eq!(lines.line_max_column(0) + 1, 24);
        assert_eq!(
            view_lines_data(&lines),
            data(&[
                ("class Nivery very long ", 1, 24),
                ("    injected text that causes ", 5, 31),
                ("    a line breakce {", 5, 21),
                ("\tfunction hi() {", 1, 17),
                ("\t\tconsole.log(\"Hello ", 1, 22),
                ("            world\");", 13, 21),
                ("\t}", 1, 3),
                ("\tfunction hello() {", 1, 20),
                ("\t\tconsole.log(\"Hello ", 1, 22),
                ("            world, this is a ", 13, 30),
                ("            somewhat longer ", 13, 29),
                ("            line\");", 13, 20),
                ("\t}", 1, 3),
                ("}", 1, 2),
            ])
        );
        // Monaco's inline decorations for the injected text: its view
        // columns on each view line.
        let injected_columns: Vec<Option<(u32, u32)>> = (0..lines.line_count())
            .map(|line| {
                let info = lines.segment_info(lines.view_line(line));
                let projection = lines.projection(lines.view_line(line).row)?;
                let injection = projection.injections().first()?;
                let start = injection.offset.max(info.offsets.start);
                let end = (injection.offset + injection.length).min(info.offsets.end);
                (start < end).then(|| {
                    (
                        info.indent + start - info.offsets.start,
                        info.indent + end - info.offsets.start,
                    )
                })
            })
            .collect();
        let mut expected = vec![None; 14];
        expected[0] = Some((8, 23));
        expected[1] = Some((4, 30));
        expected[2] = Some((4, 16));
        assert_eq!(injected_columns, expected);
        // The model columns each view line shows.
        assert_eq!(lines.segment_info(lines.view_line(0)).columns, 0..8);
        assert_eq!(lines.segment_info(lines.view_line(1)).columns, 8..8);
        assert_eq!(lines.segment_info(lines.view_line(2)).columns, 8..12);
        // Model and view positions.
        assert_eq!(lines.to_view(PointUtf16::new(0, 8), false), (0, 8));
        assert_eq!(
            lines.to_view_with_affinity(PointUtf16::new(0, 8), PositionAffinity::Right),
            (2, 16)
        );
        assert_eq!(lines.to_view(PointUtf16::new(0, 9), false), (2, 17));
        assert_eq!(lines.to_model(1, 10), PointUtf16::new(0, 8));
        assert_eq!(lines.to_model(2, 17), PointUtf16::new(0, 9));
        // Removing it unwraps the line again.
        assert!(lines.set_injected_text([(0, None)]));
        assert_eq!(lines.line_count(), 12);
        assert!(!lines.set_injected_text([(0, None)]));
    }

    /// Monaco `viewModelImpl.test.ts`, "normalizePosition with multiple
    /// touching injected text": `just sobarbzme text`, 1-based columns.
    #[test]
    fn normalize_position_with_multiple_touching_injected_text() {
        let mut lines = ViewLines::new(Rope::from("just some text"), 0);
        set_injected(&mut lines, 0, vec![injected(7, "bar"), injected(7, "bz")]);
        assert_eq!(lines.line_content(0), "just sobarbzme text");
        let normalize = |column: u32, affinity| {
            lines
                .normalize_position(PointUtf16::new(0, column - 1), affinity)
                .column
                + 1
        };
        let columns = [8, 9, 11, 12, 13];
        assert_eq!(
            columns.map(|c| normalize(c, PositionAffinity::None)),
            [8, 8, 11, 11, 13]
        );
        assert_eq!(
            columns.map(|c| normalize(c, PositionAffinity::Left)),
            [8, 8, 8, 8, 8]
        );
        assert_eq!(
            columns.map(|c| normalize(c, PositionAffinity::Right)),
            [13, 13, 13, 13, 13]
        );
    }

    /// Unwrapped rows with injected text: view columns count it, and the
    /// segment columns (the text alone) don't.
    #[test]
    fn injected_text_in_view_columns() {
        let mut lines = ViewLines::new(Rope::from("ab\ncd"), 0);
        set_injected(&mut lines, 0, vec![injected(1, "XYZ")]);
        assert_eq!(lines.line_count(), 2);
        assert_eq!(lines.line_content(0), "aXYZb");
        assert_eq!(lines.line_max_column(0), 5);
        assert_eq!(lines.segment_content(0), "ab");
        assert_eq!(lines.segment_max_column(0), 2);
        assert_eq!(lines.to_view(PointUtf16::new(0, 1), false), (0, 1));
        assert_eq!(lines.to_view(PointUtf16::new(0, 2), false), (0, 5));
        assert_eq!(lines.to_model(0, 3), PointUtf16::new(0, 1));
        assert_eq!(lines.injected_text_at(PointUtf16::new(0, 2)), Some(0));
        assert_eq!(lines.injected_text_at(PointUtf16::new(0, 0)), None);
        assert_eq!(lines.view_to_segment_column(0, 4), 1);
        assert_eq!(lines.view_to_segment_column(0, 5), 2);
        assert_eq!(lines.segment_to_view_column(0, 1), 1);
        assert_eq!(lines.segment_to_view_column(0, 2), 5);
        // Other rows are as they were.
        assert_eq!(lines.line_content(1), "cd");
        assert_eq!(lines.to_view(PointUtf16::new(1, 1), false), (1, 1));
    }

    /// Edits drop the edited rows' injected text (the editor sets it again)
    /// and move the rest with their rows.
    #[test]
    fn edits_move_injected_text_with_rows() {
        let mut buffer = Buffer::new("one\ntwo\nthree");
        let mut lines = ViewLines::new(buffer.rope().clone(), buffer.version());
        lines.set_wrap(wrap(8));
        set_injected(&mut lines, 0, vec![injected(3, ": u32 hint")]);
        set_injected(&mut lines, 2, vec![injected(0, "> ")]);
        assert_eq!(lines.line_count(), 4);
        buffer.edit([(0..0, "zero\n")]).unwrap();
        let patch = buffer.edits_since(lines.version());
        lines.apply_edits(buffer.rope().clone(), buffer.version(), &patch);
        assert_eq!(lines.rows_with_injected_text().collect::<Vec<_>>(), vec![3]);
        assert_eq!(lines.line_content(4), "> three");
        assert_eq!(lines.line_count(), 4);
        set_injected(&mut lines, 1, vec![injected(3, ": u32 hint")]);
        let mut fresh = ViewLines::new(buffer.rope().clone(), buffer.version());
        fresh.set_wrap(wrap(8));
        set_injected(&mut fresh, 1, vec![injected(3, ": u32 hint")]);
        set_injected(&mut fresh, 3, vec![injected(0, "> ")]);
        assert_eq!(lines.line_count(), fresh.line_count());
        for line in 0..lines.line_count() {
            assert_eq!(lines.line_content(line), fresh.line_content(line));
        }
        // Unwrapping keeps the injected text.
        lines.set_wrap(None);
        assert_eq!(lines.line_count(), 4);
        assert_eq!(lines.line_content(1), "one: u32 hint");
    }
}
