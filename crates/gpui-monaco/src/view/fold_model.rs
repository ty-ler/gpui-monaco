//! Folding commands over fold ranges and their collapse state.
//!
//! Ports the region queries of Monaco's `FoldingModel` and the
//! `setCollapseState*`, `toggleCollapseState` and `get*FoldLine` functions
//! of `src/vs/editor/contrib/folding/browser/foldingModel.ts` (MIT), with
//! 0-based rows. The commands return the indexes of the ranges to toggle.

use crate::view::folding::{FoldRange, FoldingRangeKind};

/// Fold ranges (sorted by start, with parents) and which are collapsed.
pub struct FoldingModel<'a> {
    /// The ranges.
    pub ranges: &'a [FoldRange],
    /// `collapsed[i]`: range `i` is collapsed.
    pub collapsed: Vec<bool>,
}

impl<'a> FoldingModel<'a> {
    /// A model of `ranges` with the given collapsed flags.
    pub fn new(ranges: &'a [FoldRange], collapsed: Vec<bool>) -> Self {
        debug_assert_eq!(ranges.len(), collapsed.len());
        Self { ranges, collapsed }
    }

    fn start(&self, ix: usize) -> u32 {
        self.ranges[ix].start_row
    }

    fn end(&self, ix: usize) -> u32 {
        self.ranges[ix].end_row
    }

    fn contains(&self, ix: usize, row: u32) -> bool {
        self.start(ix) <= row && row <= self.end(ix)
    }

    /// Range `a` lies within range `b` (Monaco `containedBy`).
    fn contained_by(&self, a: usize, b: usize) -> bool {
        self.start(b) <= self.start(a) && self.end(a) <= self.end(b)
    }

    /// Monaco `FoldingRegions.findRange`: the innermost range containing
    /// `row`.
    pub fn find_range(&self, row: u32) -> Option<usize> {
        let mut ix = self
            .ranges
            .partition_point(|range| range.start_row <= row)
            .checked_sub(1)?;
        if self.end(ix) >= row {
            return Some(ix);
        }
        while let Some(parent) = self.ranges[ix].parent {
            if self.contains(parent, row) {
                return Some(parent);
            }
            ix = parent;
        }
        None
    }

    /// Monaco `getAllRegionsAtLine`: the ranges containing `row`, innermost
    /// first; `filter` gets each with its level (1 for the innermost).
    pub fn all_regions_at_line(
        &self,
        row: u32,
        filter: impl Fn(usize, usize) -> bool,
    ) -> Vec<usize> {
        let mut result = Vec::new();
        let mut ix = self.find_range(row);
        let mut level = 1;
        while let Some(i) = ix {
            if filter(i, level) {
                result.push(i);
            }
            level += 1;
            ix = self.ranges[i].parent;
        }
        result
    }

    /// Monaco `getRegionsInside`: the ranges inside `region` (all of them
    /// for `None`), in order; `filter` gets each with its nesting level
    /// below `region` (1 for direct children).
    pub fn regions_inside(
        &self,
        region: Option<usize>,
        filter: impl Fn(usize, usize) -> bool,
    ) -> Vec<usize> {
        let mut result = Vec::new();
        let first = region.map_or(0, |r| r + 1);
        let end = region.map_or(u32::MAX, |r| self.end(r));
        let mut stack: Vec<usize> = Vec::new();
        for ix in first..self.ranges.len() {
            if self.start(ix) >= end {
                break;
            }
            while let Some(&top) = stack.last() {
                if self.contained_by(ix, top) {
                    break;
                }
                stack.pop();
            }
            stack.push(ix);
            if filter(ix, stack.len()) {
                result.push(ix);
            }
        }
        result
    }
}

/// Monaco `toggleCollapseState`: toggles the range at each row and, for
/// `levels > 1`, the ranges inside it (up to `levels`) not yet in its new
/// state.
pub fn toggle_collapse_state(model: &FoldingModel, levels: usize, rows: &[u32]) -> Vec<usize> {
    let mut toggle = Vec::new();
    for &row in rows {
        if let Some(region) = model.find_range(row) {
            let collapse = !model.collapsed[region];
            toggle.push(region);
            if levels > 1 {
                toggle.extend(model.regions_inside(Some(region), |r, level| {
                    model.collapsed[r] != collapse && level < levels
                }));
            }
        }
    }
    toggle
}

/// Monaco `setCollapseStateLevelsDown`: collapses (or expands) the range at
/// each row and those inside it up to `levels`; every range for `None`.
pub fn set_collapse_state_levels_down(
    model: &FoldingModel,
    collapse: bool,
    levels: usize,
    rows: Option<&[u32]>,
) -> Vec<usize> {
    let mut toggle = Vec::new();
    match rows.filter(|rows| !rows.is_empty()) {
        Some(rows) => {
            for &row in rows {
                if let Some(region) = model.find_range(row) {
                    if model.collapsed[region] != collapse {
                        toggle.push(region);
                    }
                    if levels > 1 {
                        toggle.extend(model.regions_inside(Some(region), |r, level| {
                            model.collapsed[r] != collapse && level < levels
                        }));
                    }
                }
            }
        }
        None => toggle.extend(model.regions_inside(None, |r, level| {
            model.collapsed[r] != collapse && level < levels
        })),
    }
    toggle
}

/// Monaco `setCollapseStateLevelsUp`: the ranges containing each row, up to
/// `levels` out.
pub fn set_collapse_state_levels_up(
    model: &FoldingModel,
    collapse: bool,
    levels: usize,
    rows: &[u32],
) -> Vec<usize> {
    rows.iter()
        .flat_map(|&row| {
            model.all_regions_at_line(row, |r, level| {
                model.collapsed[r] != collapse && level <= levels
            })
        })
        .collect()
}

/// Monaco `setCollapseStateUp`: the innermost range at each row not yet in
/// the state (so folding again folds the parent).
pub fn set_collapse_state_up(model: &FoldingModel, collapse: bool, rows: &[u32]) -> Vec<usize> {
    rows.iter()
        .filter_map(|&row| {
            model
                .all_regions_at_line(row, |r, _| model.collapsed[r] != collapse)
                .first()
                .copied()
        })
        .collect()
}

/// Monaco `setCollapseStateAtLevel`: the ranges at `level` (1 is the top)
/// not containing a blocked row.
pub fn set_collapse_state_at_level(
    model: &FoldingModel,
    level: usize,
    collapse: bool,
    blocked: &[u32],
) -> Vec<usize> {
    model.regions_inside(None, |r, l| {
        l == level
            && model.collapsed[r] != collapse
            && !blocked.iter().any(|&row| model.contains(r, row))
    })
}

/// Monaco `setCollapseStateForRest`: every range except those containing
/// or contained by the innermost range at a blocked row.
pub fn set_collapse_state_for_rest(
    model: &FoldingModel,
    collapse: bool,
    blocked: &[u32],
) -> Vec<usize> {
    let kept: Vec<usize> = blocked
        .iter()
        .filter_map(|&row| model.all_regions_at_line(row, |_, _| true).first().copied())
        .collect();
    model.regions_inside(None, |r, _| {
        kept.iter()
            .all(|&k| !model.contained_by(k, r) && !model.contained_by(r, k))
            && model.collapsed[r] != collapse
    })
}

/// Monaco `setCollapseStateForMatchingLines`: the ranges whose first line
/// matches.
pub fn set_collapse_state_for_matching_lines(
    model: &FoldingModel,
    matches: impl Fn(u32) -> bool,
    collapse: bool,
) -> Vec<usize> {
    (0..model.ranges.len())
        .rev()
        .filter(|&r| model.collapsed[r] != collapse && matches(model.start(r)))
        .collect()
}

/// Monaco `setCollapseStateForType`: the ranges of `kind`.
pub fn set_collapse_state_for_type(
    model: &FoldingModel,
    kind: &FoldingRangeKind,
    collapse: bool,
) -> Vec<usize> {
    (0..model.ranges.len())
        .rev()
        .filter(|&r| model.collapsed[r] != collapse && model.ranges[r].kind.as_ref() == Some(kind))
        .collect()
}

/// A selection for [`manual_ranges_to_remove`]: its first and last rows,
/// and whether it's empty.
#[derive(Clone, Copy, Debug)]
pub struct SelectedRows {
    /// First row.
    pub start: u32,
    /// Last row.
    pub end: u32,
    /// An empty selection (a cursor).
    pub empty: bool,
}

/// Monaco `FoldingModel.removeManualRanges`: a cursor removes the innermost
/// manual range around it (skipping provider ranges), or every manual range
/// when there's none; a selection removes the manual ranges it touches.
/// Returns the indexes of the manual ranges to remove.
pub fn manual_ranges_to_remove(model: &FoldingModel, selections: &[SelectedRows]) -> Vec<usize> {
    let mut remove_all = false;
    let mut remove: Vec<usize> = Vec::new();
    for selection in selections.iter().filter(|s| s.empty) {
        let mut ix = model.find_range(selection.start);
        while let Some(i) = ix
            && !model.ranges[i].manual
        {
            ix = model.ranges[i].parent;
        }
        match ix {
            Some(i) => remove.push(i),
            None => remove_all = true,
        }
    }
    let intersects = |ix: usize| {
        selections
            .iter()
            .filter(|s| !s.empty)
            .any(|s| !(s.start > model.end(ix) || model.start(ix) > s.end))
    };
    (0..model.ranges.len())
        .filter(|&ix| {
            model.ranges[ix].manual && (remove_all || remove.contains(&ix) || intersects(ix))
        })
        .collect()
}

/// Monaco `getParentFoldLine`: the start of the range at `row`, or of its
/// parent when `row` is that start.
pub fn parent_fold_line(row: u32, model: &FoldingModel) -> Option<u32> {
    let region = model.find_range(row)?;
    if row != model.start(region) {
        return Some(model.start(region));
    }
    model.ranges[region]
        .parent
        .map(|parent| model.start(parent))
}

/// Monaco `getPreviousFoldLine`: the previous sibling's start from a
/// range's first row, else the last range starting above `row`.
pub fn previous_fold_line(row: u32, model: &FoldingModel) -> Option<u32> {
    if let Some(region) = model.find_range(row)
        && model.start(region) == row
    {
        let parent = model.ranges[region].parent;
        let min = parent.map(|p| model.start(p));
        let mut ix = region;
        while ix > 0 {
            ix -= 1;
            if min.is_some_and(|min| model.start(ix) <= min) {
                return None;
            }
            if model.ranges[ix].parent == parent {
                return Some(model.start(ix));
            }
        }
        return None;
    }
    (0..model.ranges.len())
        .rev()
        .map(|ix| model.start(ix))
        .find(|&start| start < row)
}

/// Monaco `getNextFoldLine`: the next sibling's start from a range's first
/// row, else the first range starting below `row`.
pub fn next_fold_line(row: u32, model: &FoldingModel) -> Option<u32> {
    if let Some(region) = model.find_range(row)
        && model.start(region) == row
    {
        let parent = model.ranges[region].parent;
        let max = match parent {
            Some(p) => model.end(p),
            None => model.end(model.ranges.len() - 1),
        };
        for ix in region + 1..model.ranges.len() {
            if model.start(ix) >= max {
                return None;
            }
            if model.ranges[ix].parent == parent {
                return Some(model.start(ix));
            }
        }
        return None;
    }
    model
        .ranges
        .iter()
        .map(|r| r.start_row)
        .find(|&start| start > row)
}
