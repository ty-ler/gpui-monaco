//! Hiding unchanged regions (Monaco `UnchangedRegion` in
//! `diffEditorViewModel.ts`, MIT): long runs of unchanged lines collapse to a
//! "N hidden lines" bar, keeping some context around each change.

use crate::diff::range::LineRangeSet;
use crate::diff::{DetailedLineRangeMapping, LineRange, LineRangeMapping};

/// Monaco `RevealPreference`: which end of a region grows to reveal a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(clippy::enum_variant_names, reason = "Monaco's names")]
pub(crate) enum RevealPreference {
    FromCloserSide,
    FromTop,
    /// Monaco's breadcrumbs reveal from the bottom.
    FromBottom,
}

/// A run of unchanged lines, and how much of it shows at its top and bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnchangedRegion {
    pub original_line_number: usize,
    pub modified_line_number: usize,
    pub line_count: usize,
    pub visible_top: usize,
    pub visible_bottom: usize,
}

/// Monaco `LineRangeMapping.inverse`: the unchanged mappings between
/// `mappings`.
pub(crate) fn inverse(
    mappings: impl IntoIterator<Item = LineRangeMapping>,
    original_line_count: usize,
    modified_line_count: usize,
) -> Vec<LineRangeMapping> {
    let mut result = Vec::new();
    let (mut original_end, mut modified_end) = (1, 1);
    for m in mappings {
        let r = LineRangeMapping::new(
            LineRange::new(original_end, m.original.start_line_number),
            LineRange::new(modified_end, m.modified.start_line_number),
        );
        if !r.modified.is_empty() {
            result.push(r);
        }
        original_end = m.original.end_line_number_exclusive;
        modified_end = m.modified.end_line_number_exclusive;
    }
    let r = LineRangeMapping::new(
        LineRange::new(original_end, original_line_count + 1),
        LineRange::new(modified_end, modified_line_count + 1),
    );
    if !r.modified.is_empty() {
        result.push(r);
    }
    result
}

/// `LineRange.ofLength`.
fn of_length(start: usize, length: usize) -> LineRange {
    LineRange::new(start, start + length)
}

/// Monaco `LineRange.intersectsStrict`.
fn intersects_strict(a: &LineRange, b: &LineRange) -> bool {
    a.start_line_number.max(b.start_line_number)
        < a.end_line_number_exclusive.min(b.end_line_number_exclusive)
}

impl UnchangedRegion {
    pub fn new(
        original_line_number: usize,
        modified_line_number: usize,
        line_count: usize,
        visible_top: usize,
        visible_bottom: usize,
    ) -> Self {
        let visible_top = visible_top.min(line_count);
        let visible_bottom = visible_bottom.min(line_count - visible_top);
        Self {
            original_line_number,
            modified_line_number,
            line_count,
            visible_top,
            visible_bottom,
        }
    }

    /// Monaco `UnchangedRegion.fromDiffs`: the hideable regions, leaving
    /// `min_context` lines around changes, if at least `min_hidden_line_count`
    /// lines would hide.
    pub fn from_diffs(
        changes: &[DetailedLineRangeMapping],
        original_line_count: usize,
        modified_line_count: usize,
        min_hidden_line_count: usize,
        min_context: usize,
    ) -> Vec<Self> {
        let mut result = Vec::new();
        for mapping in inverse(
            changes.iter().map(|c| c.line_range_mapping()),
            original_line_count,
            modified_line_count,
        ) {
            let mut original_start = mapping.original.start_line_number;
            let mut modified_start = mapping.modified.start_line_number;
            let mut length = mapping.original.len();
            let at_start = original_start == 1 && modified_start == 1;
            let at_end = original_start + length == original_line_count + 1
                && modified_start + length == modified_line_count + 1;
            if (at_start || at_end) && length >= min_context + min_hidden_line_count {
                if at_start && !at_end {
                    length -= min_context;
                }
                if at_end && !at_start {
                    original_start += min_context;
                    modified_start += min_context;
                    length -= min_context;
                }
                result.push(Self::new(original_start, modified_start, length, 0, 0));
            } else if length >= min_context * 2 + min_hidden_line_count {
                original_start += min_context;
                modified_start += min_context;
                length -= min_context * 2;
                result.push(Self::new(original_start, modified_start, length, 0, 0));
            }
        }
        result
    }

    pub fn original_unchanged_range(&self) -> LineRange {
        of_length(self.original_line_number, self.line_count)
    }

    pub fn modified_unchanged_range(&self) -> LineRange {
        of_length(self.modified_line_number, self.line_count)
    }

    /// Monaco `shouldHideControls`: fully shown.
    pub fn should_hide_controls(&self) -> bool {
        self.visible_top + self.visible_bottom == self.line_count
    }

    pub fn hidden_original_range(&self) -> LineRange {
        of_length(
            self.original_line_number + self.visible_top,
            self.line_count - self.visible_top - self.visible_bottom,
        )
    }

    pub fn hidden_modified_range(&self) -> LineRange {
        of_length(
            self.modified_line_number + self.visible_top,
            self.line_count - self.visible_top - self.visible_bottom,
        )
    }

    /// Monaco `setVisibleRanges`: this region split around the ranges that
    /// were visible before, so they stay visible.
    pub fn set_visible_ranges(mut self, visible: &[LineRangeMapping]) -> Vec<Self> {
        let mut set = LineRangeSet::default();
        for r in visible {
            set.add_range(r.modified);
        }
        let hidden = set.subtract_from(self.modified_unchanged_range());
        let hidden = hidden.ranges();
        if hidden.is_empty() {
            self.show_all();
            return vec![self];
        }
        let mut result = Vec::new();
        let mut original_start = self.original_line_number;
        let mut modified_start = self.modified_line_number;
        let modified_end = self.modified_line_number + self.line_count;
        for (i, r) in hidden.iter().enumerate() {
            let is_last = i == hidden.len() - 1;
            let length = if is_last {
                modified_end
            } else {
                r.end_line_number_exclusive
            } - modified_start;
            let mut region = Self::new(original_start, modified_start, length, 0, 0);
            region.set_hidden_modified_range(*r);
            original_start = region.original_unchanged_range().end_line_number_exclusive;
            modified_start = region.modified_unchanged_range().end_line_number_exclusive;
            result.push(region);
        }
        result
    }

    pub fn set_hidden_modified_range(&mut self, range: LineRange) {
        let top = range.start_line_number as isize - self.modified_line_number as isize;
        let bottom = (self.modified_line_number + self.line_count) as isize
            - range.end_line_number_exclusive as isize;
        self.set_state(top, bottom);
    }

    pub fn max_visible_top(&self) -> usize {
        self.line_count - self.visible_bottom
    }

    pub fn max_visible_bottom(&self) -> usize {
        self.line_count - self.visible_top
    }

    /// Monaco `showMoreAbove`.
    pub fn show_more_above(&mut self, count: usize) {
        self.visible_top = (self.visible_top + count).min(self.max_visible_top());
    }

    /// Monaco `showMoreBelow`.
    pub fn show_more_below(&mut self, count: usize) {
        self.visible_bottom = (self.visible_bottom + count).min(self.max_visible_bottom());
    }

    /// Monaco `showAll`.
    pub fn show_all(&mut self) {
        self.visible_bottom = self.line_count - self.visible_top;
    }

    /// Monaco `collapseAll`.
    pub fn collapse_all(&mut self) {
        self.visible_top = 0;
        self.visible_bottom = 0;
    }

    /// Monaco `showModifiedLine`.
    pub fn show_modified_line(&mut self, line_number: usize, preference: RevealPreference) {
        let top =
            line_number as isize + 1 - (self.modified_line_number + self.visible_top) as isize;
        let bottom = (self.modified_line_number + self.line_count) as isize
            - self.visible_bottom as isize
            - line_number as isize;
        if preference == RevealPreference::FromCloserSide && top < bottom
            || preference == RevealPreference::FromTop
        {
            self.visible_top = (self.visible_top as isize + top).max(0) as usize;
        } else {
            self.visible_bottom = (self.visible_bottom as isize + bottom).max(0) as usize;
        }
    }

    /// Monaco `showOriginalLine`.
    pub fn show_original_line(&mut self, line_number: usize, preference: RevealPreference) {
        let top = line_number as isize - self.original_line_number as isize;
        let bottom = (self.original_line_number + self.line_count) as isize - line_number as isize;
        if preference == RevealPreference::FromCloserSide && top < bottom
            || preference == RevealPreference::FromTop
        {
            self.visible_top = ((self.visible_top as isize + bottom - top).max(0) as usize)
                .min(self.max_visible_top());
        } else {
            self.visible_bottom = ((self.visible_bottom as isize + top - bottom).max(0) as usize)
                .min(self.max_visible_bottom());
        }
    }

    /// Monaco `setState`.
    pub fn set_state(&mut self, top: isize, bottom: isize) {
        let top = top.clamp(0, self.line_count as isize) as usize;
        let bottom = bottom.clamp(0, (self.line_count - top) as isize) as usize;
        self.visible_top = top;
        self.visible_bottom = bottom;
    }
}

/// Monaco's state transfer in `updateUnchangedRegions`: new regions keep
/// what the user revealed in `previous` (with their ranges as of now).
pub(crate) fn transfer_state(
    regions: Vec<UnchangedRegion>,
    previous: &[UnchangedRegion],
    original_line_count: usize,
    modified_line_count: usize,
) -> Vec<UnchangedRegion> {
    if previous.is_empty() {
        return regions;
    }
    let clip = |range: LineRange, count: usize| {
        range
            .intersect(&LineRange::new(1, count + 1))
            .unwrap_or(LineRange::new(1, 1))
    };
    let hidden = previous.iter().map(|r| {
        LineRangeMapping::new(
            clip(r.hidden_original_range(), original_line_count),
            clip(r.hidden_modified_range(), modified_line_count),
        )
    });
    let visible = inverse(hidden, original_line_count, modified_line_count);
    let mut result = Vec::new();
    for r in regions {
        let intersecting: Vec<_> = visible
            .iter()
            .filter(|v| {
                intersects_strict(&v.original, &r.original_unchanged_range())
                    && intersects_strict(&v.modified, &r.modified_unchanged_range())
            })
            .cloned()
            .collect();
        result.extend(r.set_visible_ranges(&intersecting));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialize(regions: &[UnchangedRegion]) -> Vec<String> {
        regions
            .iter()
            .map(|r| {
                format!(
                    "{} - {}",
                    r.original_unchanged_range(),
                    r.modified_unchanged_range()
                )
            })
            .collect()
    }

    fn change(original: (usize, usize), modified: (usize, usize)) -> DetailedLineRangeMapping {
        DetailedLineRangeMapping::new(
            LineRange::new(original.0, original.1),
            LineRange::new(modified.0, modified.1),
            Vec::new(),
        )
    }

    // Monaco `diffEditorWidget.test.ts` suite `UnchangedRegion`.
    #[test]
    fn everything_changed() {
        let regions = UnchangedRegion::from_diffs(&[change((1, 10), (1, 10))], 10, 10, 3, 3);
        assert!(serialize(&regions).is_empty());
    }

    #[test]
    fn nothing_changed() {
        let regions = UnchangedRegion::from_diffs(&[], 10, 10, 3, 3);
        assert_eq!(serialize(&regions), ["[1,11) - [1,11)"]);
    }

    #[test]
    fn change_in_the_middle() {
        let regions = UnchangedRegion::from_diffs(&[change((50, 60), (50, 60))], 100, 100, 3, 3);
        assert_eq!(
            serialize(&regions),
            ["[1,47) - [1,47)", "[63,101) - [63,101)"]
        );
    }

    #[test]
    fn change_at_the_end() {
        let regions = UnchangedRegion::from_diffs(&[change((99, 100), (100, 100))], 100, 100, 3, 3);
        assert_eq!(serialize(&regions), ["[1,96) - [1,96)"]);
    }

    #[test]
    fn revealing_lines() {
        let mut r = UnchangedRegion::new(10, 12, 30, 0, 0);
        assert_eq!(r.hidden_modified_range().to_string(), "[12,42)");
        r.show_more_above(20);
        r.show_more_below(20);
        // Capped at the region.
        assert_eq!((r.visible_top, r.visible_bottom), (20, 10));
        assert!(r.should_hide_controls());
        r.collapse_all();
        // Line 15 is closer to the top.
        r.show_modified_line(15, RevealPreference::FromCloserSide);
        assert_eq!(r.hidden_modified_range().to_string(), "[16,42)");
        r.show_modified_line(40, RevealPreference::FromCloserSide);
        assert_eq!(r.hidden_modified_range().to_string(), "[16,40)");
    }

    #[test]
    fn revealed_lines_survive_new_diffs() {
        let mut previous =
            UnchangedRegion::from_diffs(&[change((50, 51), (50, 51))], 100, 100, 3, 3);
        previous[0].show_more_below(10);
        assert_eq!(previous[0].hidden_modified_range().to_string(), "[1,37)");
        // Another change at line 90: the first region keeps its revealed
        // bottom.
        let regions = UnchangedRegion::from_diffs(
            &[change((50, 51), (50, 51)), change((90, 91), (90, 91))],
            100,
            100,
            3,
            3,
        );
        let regions = transfer_state(regions, &previous, 100, 100);
        assert_eq!(regions[0].hidden_modified_range().to_string(), "[1,37)");
    }
}
