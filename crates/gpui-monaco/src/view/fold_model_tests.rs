//! Ported from Monaco's `contrib/folding/test/browser/foldingModel.test.ts`.
//!
//! Line numbers in the test data are Monaco's (1-based); the helpers convert
//! to and from our 0-based rows.
//!
//! Not ported:
//! - `update` and `delete`: they test how `FoldingModel.update` carries the
//!   collapse state across text edits, which is editor integration rather
//!   than the pure fold commands in `fold_model.rs`.
//! - `folding decoration`: it tests the decorations (rendering) Monaco
//!   derives from the collapse state.

use regex::Regex;
use syntax::Pattern;
use text::Rope;

use crate::view::fold_model::{
    FoldingModel, SelectedRows, manual_ranges_to_remove, next_fold_line, parent_fold_line,
    previous_fold_line, set_collapse_state_at_level, set_collapse_state_for_matching_lines,
    set_collapse_state_for_rest, set_collapse_state_levels_down, set_collapse_state_levels_up,
    set_collapse_state_up, toggle_collapse_state,
};
use crate::view::folding::{FoldRange, compute_indent_ranges};

/// Monaco's `ExpectedRegion`, with 1-based lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct R {
    start: u32,
    end: u32,
    collapsed: bool,
}

fn r(start: u32, end: u32, collapsed: bool) -> R {
    R {
        start,
        end,
        collapsed,
    }
}

/// An empty selection (cursor) on 1-based `line`.
fn cursor(line: u32) -> SelectedRows {
    SelectedRows {
        start: line - 1,
        end: line - 1,
        empty: true,
    }
}

/// A non-empty selection over 1-based lines `start..=end`.
fn selection(start: u32, end: u32) -> SelectedRows {
    SelectedRows {
        start: start - 1,
        end: end - 1,
        empty: false,
    }
}

/// Monaco's `/^\/\/#region$/` and `/^\/\/#endregion$/`.
const REGION: (&str, &str) = (r"^//#region$", r"^//#endregion$");

/// Stands in for Monaco's mutable `FoldingModel`.
struct Harness {
    lines: Vec<String>,
    ranges: Vec<FoldRange>,
    collapsed: Vec<bool>,
}

impl Harness {
    /// Monaco `computeRanges(textModel, false, markers)` with the default tab
    /// size of 4, all expanded.
    fn from_text(lines: &[&str], markers: Option<(&str, &str)>) -> Self {
        let rope = Rope::from(lines.join("\n").as_str());
        let patterns = markers.map(|(start, end)| (Pattern::new(start, ""), Pattern::new(end, "")));
        let ranges = compute_indent_ranges(&rope, 4, false, patterns.as_ref().map(|(s, e)| (s, e)));
        let collapsed = vec![false; ranges.len()];
        Self {
            lines: lines.iter().map(|line| line.to_string()).collect(),
            ranges,
            collapsed,
        }
    }

    /// Monaco `FoldingRegions.fromFoldRanges` of `(start, end, manual)`
    /// with 1-based lines, all expanded.
    fn from_ranges(ranges: &[(u32, u32, bool)]) -> Self {
        let ranges: Vec<FoldRange> = ranges
            .iter()
            .map(|&(start, end, manual)| FoldRange {
                start_row: start - 1,
                end_row: end - 1,
                parent: None,
                kind: None,
                manual,
            })
            .collect();
        let mut harness = Self {
            lines: Vec::new(),
            collapsed: vec![false; ranges.len()],
            ranges,
        };
        harness.compute_parents();
        harness
    }

    /// Monaco `FoldingRegions.ensureParentIndices`.
    fn compute_parents(&mut self) {
        let mut stack: Vec<usize> = Vec::new();
        for i in 0..self.ranges.len() {
            let (start, end) = (self.ranges[i].start_row, self.ranges[i].end_row);
            while let Some(&top) = stack.last() {
                let t = &self.ranges[top];
                if t.start_row <= start && t.end_row >= end {
                    break;
                }
                stack.pop();
            }
            self.ranges[i].parent = stack.last().copied();
            stack.push(i);
        }
    }

    fn model(&self) -> FoldingModel<'_> {
        FoldingModel::new(&self.ranges, self.collapsed.clone())
    }

    /// Monaco `FoldingModel.toggleCollapseState`: sort, dedupe, flip each.
    fn toggle(&mut self, mut indexes: Vec<usize>) {
        indexes.sort_unstable();
        indexes.dedup();
        for ix in indexes {
            self.collapsed[ix] = !self.collapsed[ix];
        }
    }

    /// Runs a command on the model and applies the toggles it returns.
    fn apply(&mut self, command: impl FnOnce(&FoldingModel) -> Vec<usize>) {
        let indexes = command(&self.model());
        self.toggle(indexes);
    }

    /// Monaco `FoldingModel.removeManualRanges`.
    fn remove_manual_ranges(&mut self, selections: &[SelectedRows]) {
        let remove = manual_ranges_to_remove(&self.model(), selections);
        let mut ix = 0;
        self.collapsed.retain(|_| {
            ix += 1;
            !remove.contains(&(ix - 1))
        });
        let mut ix = 0;
        self.ranges.retain(|_| {
            ix += 1;
            !remove.contains(&(ix - 1))
        });
        self.compute_parents();
    }

    fn region(&self, ix: usize) -> R {
        let range = &self.ranges[ix];
        r(range.start_row + 1, range.end_row + 1, self.collapsed[ix])
    }

    fn regions(&self, indexes: &[usize]) -> Vec<R> {
        indexes.iter().map(|&ix| self.region(ix)).collect()
    }

    /// Monaco `getRegionAtLine`, as an index.
    fn region_at(&self, line: u32) -> Option<usize> {
        self.model().find_range(line - 1)
    }

    fn assert_region(&self, line: u32, expected: Option<R>, message: &str) {
        assert_eq!(
            self.region_at(line).map(|ix| self.region(ix)),
            expected,
            "{message}"
        );
    }

    fn assert_ranges(&self, expected: &[R]) {
        let actual: Vec<R> = (0..self.ranges.len()).map(|ix| self.region(ix)).collect();
        assert_eq!(actual, expected);
    }

    fn assert_folded_ranges(&self, expected: &[R], message: &str) {
        let actual: Vec<R> = (0..self.ranges.len())
            .filter(|&ix| self.collapsed[ix])
            .map(|ix| {
                let range = &self.ranges[ix];
                r(range.start_row + 1, range.end_row + 1, false)
            })
            .collect();
        assert_eq!(actual, expected, "{message}");
    }

    fn all_regions_at_line(&self, line: u32) -> Vec<R> {
        self.regions(&self.model().all_regions_at_line(line - 1, |_, _| true))
    }

    fn regions_inside(
        &self,
        region: Option<usize>,
        filter: impl Fn(usize, usize) -> bool,
    ) -> Vec<R> {
        self.regions(&self.model().regions_inside(region, filter))
    }

    fn parent_fold_line(&self, line: u32) -> Option<u32> {
        parent_fold_line(line - 1, &self.model()).map(|row| row + 1)
    }

    fn previous_fold_line(&self, line: u32) -> Option<u32> {
        previous_fold_line(line - 1, &self.model()).map(|row| row + 1)
    }

    fn next_fold_line(&self, line: u32) -> Option<u32> {
        next_fold_line(line - 1, &self.model()).map(|row| row + 1)
    }
}

/// 1-based lines to 0-based rows.
fn rows(lines: &[u32]) -> Vec<u32> {
    lines.iter().map(|line| line - 1).collect()
}

const JAVA: [&str; 8] = [
    /* 1*/ "/**",
    /* 2*/ " * Comment",
    /* 3*/ " */",
    /* 4*/ "class A {",
    /* 5*/ "  void foo() {",
    /* 6*/ "    // comment {",
    /* 7*/ "  }",
    /* 8*/ "}",
];

/// The source shared by the `setCollapseState*` tests.
const NESTED: [&str; 13] = [
    /* 1*/ "//#region",
    /* 2*/ "//#endregion",
    /* 3*/ "class A {",
    /* 4*/ "  void foo() {",
    /* 5*/ "    if (true) {",
    /* 6*/ "        return;",
    /* 7*/ "    }",
    /* 8*/ "",
    /* 9*/ "    if (true) {",
    /* 10*/ "      return;",
    /* 11*/ "    }",
    /* 12*/ "  }",
    /* 13*/ "}",
];

/// The ranges of [`NESTED`].
fn nested() -> (Harness, [R; 5]) {
    let h = Harness::from_text(&NESTED, Some(REGION));
    let rs = [
        r(1, 2, false),
        r(3, 12, false),
        r(4, 11, false),
        r(5, 6, false),
        r(9, 10, false),
    ];
    h.assert_ranges(&rs);
    (h, rs)
}

#[test]
fn get_region_at_line() {
    let h = Harness::from_text(&JAVA, None);
    let r1 = r(1, 3, false);
    let r2 = r(4, 7, false);
    let r3 = r(5, 6, false);
    h.assert_ranges(&[r1, r2, r3]);

    h.assert_region(1, Some(r1), "1");
    h.assert_region(2, Some(r1), "2");
    h.assert_region(3, Some(r1), "3");
    h.assert_region(4, Some(r2), "4");
    h.assert_region(5, Some(r3), "5");
    h.assert_region(6, Some(r3), "5");
    h.assert_region(7, Some(r2), "6");
    h.assert_region(8, None, "7");
}

#[test]
fn collapse() {
    let mut h = Harness::from_text(&JAVA, None);
    let r2 = r(4, 7, false);
    let r3 = r(5, 6, false);
    h.assert_ranges(&[r(1, 3, false), r2, r3]);

    h.apply(|m| toggle_collapse_state(m, 1, &rows(&[1])));
    h.assert_ranges(&[r(1, 3, true), r2, r3]);

    h.apply(|m| toggle_collapse_state(m, 1, &rows(&[5])));
    h.assert_ranges(&[r(1, 3, true), r2, r(5, 6, true)]);

    h.apply(|m| toggle_collapse_state(m, 1, &rows(&[7])));
    h.assert_ranges(&[r(1, 3, true), r(4, 7, true), r(5, 6, true)]);
}

#[test]
fn get_regions_inside() {
    let h = Harness::from_text(&JAVA, None);
    let r1 = r(1, 3, false);
    let r2 = r(4, 7, false);
    let r3 = r(5, 6, false);
    h.assert_ranges(&[r1, r2, r3]);
    let region1 = h.region_at(r1.start);
    let region2 = h.region_at(r2.start);
    let region3 = h.region_at(r3.start);

    assert_eq!(h.regions_inside(None, |_, _| true), [r1, r2, r3], "1");
    assert_eq!(h.regions_inside(region1, |_, _| true), [], "2");
    assert_eq!(h.regions_inside(region2, |_, _| true), [r3], "3");
    assert_eq!(h.regions_inside(region3, |_, _| true), [], "4");
}

#[test]
fn get_regions_inside_with_level() {
    let lines = [
        /* 1*/ "//#region",
        /* 2*/ "//#endregion",
        /* 3*/ "class A {",
        /* 4*/ "  void foo() {",
        /* 5*/ "    if (true) {",
        /* 6*/ "        return;",
        /* 7*/ "    }",
        /* 8*/ "    if (true) {",
        /* 9*/ "      return;",
        /* 10*/ "    }",
        /* 11*/ "  }",
        /* 12*/ "}",
    ];
    let h = Harness::from_text(&lines, Some(REGION));
    let r1 = r(1, 2, false);
    let r2 = r(3, 11, false);
    let r3 = r(4, 10, false);
    let r4 = r(5, 6, false);
    let r5 = r(8, 9, false);

    let region1 = h.region_at(r1.start);
    let region2 = h.region_at(r2.start);
    let region3 = h.region_at(r3.start);

    h.assert_ranges(&[r1, r2, r3, r4, r5]);

    assert_eq!(h.regions_inside(None, |_, level| level == 1), [r1, r2], "1");
    assert_eq!(h.regions_inside(None, |_, level| level == 2), [r3], "2");
    assert_eq!(h.regions_inside(None, |_, level| level == 3), [r4, r5], "3");

    assert_eq!(h.regions_inside(region2, |_, level| level == 1), [r3], "4");
    assert_eq!(
        h.regions_inside(region2, |_, level| level == 2),
        [r4, r5],
        "5"
    );
    assert_eq!(
        h.regions_inside(region3, |_, level| level == 1),
        [r4, r5],
        "6"
    );

    // Monaco `r.hidesLine(9)`: start < 9 <= end, in 1-based lines.
    let hides_line_9 = |ix: usize, _| {
        let range = &h.ranges[ix];
        range.start_row < 8 && 8 <= range.end_row
    };
    assert_eq!(h.regions_inside(region2, hides_line_9), [r3, r5], "7");

    assert_eq!(h.regions_inside(region1, |_, level| level == 1), [], "8");
}

#[test]
fn get_region_at_line_2() {
    let lines = [
        /* 1*/ "//#region",
        /* 2*/ "class A {",
        /* 3*/ "  void foo() {",
        /* 4*/ "    if (true) {",
        /* 5*/ "      //hello",
        /* 6*/ "    }",
        /* 7*/ "",
        /* 8*/ "  }",
        /* 9*/ "}",
        /* 10*/ "//#endregion",
        /* 11*/ "",
    ];
    let h = Harness::from_text(&lines, Some(REGION));
    let r1 = r(1, 10, false);
    let r2 = r(2, 8, false);
    let r3 = r(3, 7, false);
    let r4 = r(4, 5, false);
    h.assert_ranges(&[r1, r2, r3, r4]);

    // Innermost first: Monaco's `[...].reverse()`.
    assert_eq!(h.all_regions_at_line(1), [r1], "1");
    assert_eq!(h.all_regions_at_line(2), [r2, r1], "2");
    assert_eq!(h.all_regions_at_line(3), [r3, r2, r1], "3");
    assert_eq!(h.all_regions_at_line(4), [r4, r3, r2, r1], "4");
    assert_eq!(h.all_regions_at_line(5), [r4, r3, r2, r1], "5");
    assert_eq!(h.all_regions_at_line(6), [r3, r2, r1], "6");
    assert_eq!(h.all_regions_at_line(7), [r3, r2, r1], "7");
    assert_eq!(h.all_regions_at_line(8), [r2, r1], "8");
    assert_eq!(h.all_regions_at_line(9), [r1], "9");
    assert_eq!(h.all_regions_at_line(10), [r1], "10");
    assert_eq!(h.all_regions_at_line(11), [], "10");
}

#[test]
fn set_collapse_state_recursivly() {
    let (mut h, [r1, r2, r3, r4, r5]) = nested();
    let max = usize::MAX;

    h.apply(|m| set_collapse_state_levels_down(m, true, max, Some(&rows(&[4]))));
    h.assert_folded_ranges(&[r3, r4, r5], "1");

    h.apply(|m| set_collapse_state_levels_down(m, false, max, Some(&rows(&[8]))));
    h.assert_folded_ranges(&[], "2");

    h.apply(|m| set_collapse_state_levels_down(m, true, max, Some(&rows(&[12]))));
    h.assert_folded_ranges(&[r2, r3, r4, r5], "1");

    h.apply(|m| set_collapse_state_levels_down(m, false, max, Some(&rows(&[7]))));
    h.assert_folded_ranges(&[r2], "1");

    h.apply(|m| set_collapse_state_levels_down(m, false, max, None));
    h.assert_folded_ranges(&[], "1");

    h.apply(|m| set_collapse_state_levels_down(m, true, max, None));
    h.assert_folded_ranges(&[r1, r2, r3, r4, r5], "1");
}

#[test]
fn monaco_set_collapse_state_at_level() {
    let lines = [
        /* 1*/ "//#region",
        /* 2*/ "//#endregion",
        /* 3*/ "class A {",
        /* 4*/ "  void foo() {",
        /* 5*/ "    if (true) {",
        /* 6*/ "        return;",
        /* 7*/ "    }",
        /* 8*/ "",
        /* 9*/ "    if (true) {",
        /* 10*/ "      return;",
        /* 11*/ "    }",
        /* 12*/ "  }",
        /* 13*/ "  //#region",
        /* 14*/ "  const bar = 9;",
        /* 15*/ "  //#endregion",
        /* 16*/ "}",
    ];
    let mut h = Harness::from_text(&lines, Some((r"^\s*//#region$", r"^\s*//#endregion$")));
    let r1 = r(1, 2, false);
    let r2 = r(3, 15, false);
    let r3 = r(4, 11, false);
    let r4 = r(5, 6, false);
    let r5 = r(9, 10, false);
    let r6 = r(13, 15, false);
    h.assert_ranges(&[r1, r2, r3, r4, r5, r6]);

    h.apply(|m| set_collapse_state_at_level(m, 1, true, &[]));
    h.assert_folded_ranges(&[r1, r2], "1");

    h.apply(|m| set_collapse_state_at_level(m, 1, false, &rows(&[5])));
    h.assert_folded_ranges(&[r2], "2");

    h.apply(|m| set_collapse_state_at_level(m, 1, false, &rows(&[1])));
    h.assert_folded_ranges(&[], "3");

    h.apply(|m| set_collapse_state_at_level(m, 2, true, &[]));
    h.assert_folded_ranges(&[r3, r6], "4");

    h.apply(|m| set_collapse_state_at_level(m, 2, false, &rows(&[5, 6])));
    h.assert_folded_ranges(&[r3], "5");

    h.apply(|m| set_collapse_state_at_level(m, 3, true, &rows(&[4, 9])));
    h.assert_folded_ranges(&[r3, r4], "6");

    h.apply(|m| set_collapse_state_at_level(m, 3, false, &rows(&[4, 9])));
    h.assert_folded_ranges(&[r3], "7");
}

#[test]
fn monaco_set_collapse_state_levels_down() {
    let (mut h, [r1, _r2, r3, r4, r5]) = nested();

    h.apply(|m| set_collapse_state_levels_down(m, true, 1, Some(&rows(&[4]))));
    h.assert_folded_ranges(&[r3], "1");

    h.apply(|m| set_collapse_state_levels_down(m, true, 2, Some(&rows(&[4]))));
    h.assert_folded_ranges(&[r3, r4, r5], "2");

    h.apply(|m| set_collapse_state_levels_down(m, false, 2, Some(&rows(&[3]))));
    h.assert_folded_ranges(&[r4, r5], "3");

    h.apply(|m| set_collapse_state_levels_down(m, false, 2, Some(&rows(&[2]))));
    h.assert_folded_ranges(&[r4, r5], "4");

    h.apply(|m| set_collapse_state_levels_down(m, true, 4, Some(&rows(&[2]))));
    h.assert_folded_ranges(&[r1, r4, r5], "5");

    h.apply(|m| set_collapse_state_levels_down(m, false, 4, Some(&rows(&[2, 3]))));
    h.assert_folded_ranges(&[], "6");
}

#[test]
fn monaco_set_collapse_state_levels_up() {
    let (mut h, [_r1, r2, r3, _r4, r5]) = nested();

    h.apply(|m| set_collapse_state_levels_up(m, true, 1, &rows(&[4])));
    h.assert_folded_ranges(&[r3], "1");

    h.apply(|m| set_collapse_state_levels_up(m, true, 2, &rows(&[4])));
    h.assert_folded_ranges(&[r2, r3], "2");

    h.apply(|m| set_collapse_state_levels_up(m, false, 4, &rows(&[1, 3, 4])));
    h.assert_folded_ranges(&[], "3");

    h.apply(|m| set_collapse_state_levels_up(m, true, 2, &rows(&[10])));
    h.assert_folded_ranges(&[r3, r5], "4");
}

#[test]
fn monaco_set_collapse_state_up() {
    let (mut h, [_r1, r2, r3, r4, _r5]) = nested();

    h.apply(|m| set_collapse_state_up(m, true, &rows(&[5])));
    h.assert_folded_ranges(&[r4], "1");

    h.apply(|m| set_collapse_state_up(m, true, &rows(&[5])));
    h.assert_folded_ranges(&[r3, r4], "2");

    h.apply(|m| set_collapse_state_up(m, true, &rows(&[4])));
    h.assert_folded_ranges(&[r2, r3, r4], "2");
}

#[test]
fn monaco_set_collapse_state_for_matching_lines() {
    let lines = [
        /* 1*/ "/**",
        /* 2*/ " * the class",
        /* 3*/ " */",
        /* 4*/ "class A {",
        /* 5*/ "  /**",
        /* 6*/ "   * the foo",
        /* 7*/ "   */",
        /* 8*/ "  void foo() {",
        /* 9*/ "    /*",
        /* 10*/ "     * the comment",
        /* 11*/ "     */",
        /* 12*/ "  }",
        /* 13*/ "}",
    ];
    let mut h = Harness::from_text(&lines, Some(REGION));
    let r1 = r(1, 3, false);
    let r2 = r(4, 12, false);
    let r3 = r(5, 7, false);
    let r4 = r(8, 11, false);
    let r5 = r(9, 11, false);
    h.assert_ranges(&[r1, r2, r3, r4, r5]);

    let regex = Regex::new(&format!(r"^\s*{}", regex::escape("/*"))).unwrap();
    let text = h.lines.clone();
    h.apply(|m| {
        set_collapse_state_for_matching_lines(m, |row| regex.is_match(&text[row as usize]), true)
    });
    h.assert_folded_ranges(&[r1, r3, r5], "1");
}

#[test]
fn monaco_set_collapse_state_for_rest() {
    let (mut h, [r1, r2, r3, r4, r5]) = nested();

    h.apply(|m| set_collapse_state_for_rest(m, true, &rows(&[5])));
    h.assert_folded_ranges(&[r1, r5], "1");

    h.apply(|m| set_collapse_state_for_rest(m, false, &rows(&[5])));
    h.assert_folded_ranges(&[], "2");

    h.apply(|m| set_collapse_state_for_rest(m, true, &rows(&[1])));
    h.assert_folded_ranges(&[r2, r3, r4, r5], "3");

    h.apply(|m| set_collapse_state_for_rest(m, true, &rows(&[3])));
    h.assert_folded_ranges(&[r1, r2, r3, r4, r5], "3");
}

#[test]
fn fold_jumping() {
    let lines = [
        /* 1*/ "class A {",
        /* 2*/ "  void foo() {",
        /* 3*/ "    if (1) {",
        /* 4*/ "      a();",
        /* 5*/ "    } else if (2) {",
        /* 6*/ "      if (true) {",
        /* 7*/ "        b();",
        /* 8*/ "      }",
        /* 9*/ "    } else {",
        /* 10*/ "      c();",
        /* 11*/ "    }",
        /* 12*/ "  }",
        /* 13*/ "}",
    ];
    let h = Harness::from_text(&lines, None);
    h.assert_ranges(&[
        r(1, 12, false),
        r(2, 11, false),
        r(3, 4, false),
        r(5, 8, false),
        r(6, 7, false),
        r(9, 10, false),
    ]);

    // Jump to parent.
    assert_eq!(h.parent_fold_line(7), Some(6));
    assert_eq!(h.parent_fold_line(6), Some(5));
    assert_eq!(h.parent_fold_line(5), Some(2));
    assert_eq!(h.parent_fold_line(2), Some(1));
    assert_eq!(h.parent_fold_line(1), None);

    // Jump to previous.
    assert_eq!(h.previous_fold_line(10), Some(9));
    assert_eq!(h.previous_fold_line(9), Some(5));
    assert_eq!(h.previous_fold_line(5), Some(3));
    assert_eq!(h.previous_fold_line(3), None);
    // Not on a folding region start line.
    assert_eq!(h.previous_fold_line(4), Some(3));
    assert_eq!(h.previous_fold_line(7), Some(6));
    assert_eq!(h.previous_fold_line(8), Some(6));

    // Jump to next.
    assert_eq!(h.next_fold_line(3), Some(5));
    assert_eq!(h.next_fold_line(5), Some(9));
    assert_eq!(h.next_fold_line(9), None);
    // Not on a folding region start line.
    assert_eq!(h.next_fold_line(4), Some(5));
    assert_eq!(h.next_fold_line(7), Some(9));
    assert_eq!(h.next_fold_line(8), Some(9));
}

#[test]
fn fold_jumping_issue_129503() {
    let lines = [
        /* 1*/ "",
        /* 2*/ "if True:",
        /* 3*/ "  print(1)",
        /* 4*/ "if True:",
        /* 5*/ "  print(1)",
        /* 6*/ "",
    ];
    let h = Harness::from_text(&lines, None);
    h.assert_ranges(&[r(2, 3, false), r(4, 6, false)]);

    // Jump to next.
    assert_eq!(h.next_fold_line(1), Some(2));
    assert_eq!(h.next_fold_line(2), Some(4));
    assert_eq!(h.next_fold_line(3), Some(4));
    assert_eq!(h.next_fold_line(4), None);
    assert_eq!(h.next_fold_line(5), None);
    assert_eq!(h.next_fold_line(6), None);

    // Jump to previous.
    assert_eq!(h.previous_fold_line(1), None);
    assert_eq!(h.previous_fold_line(2), None);
    assert_eq!(h.previous_fold_line(3), Some(2));
    assert_eq!(h.previous_fold_line(4), Some(2));
    assert_eq!(h.previous_fold_line(5), Some(4));
    assert_eq!(h.previous_fold_line(6), Some(4));
}

#[test]
fn remove_manual_ranges_cursor_on_manual_range_removes_innermost_only() {
    // Outer provider range plus two nested manual ranges.
    let mut h = Harness::from_ranges(&[(1, 6, false), (2, 5, true), (3, 4, true)]);
    h.assert_ranges(&[r(1, 6, false), r(2, 5, false), r(3, 4, false)]);

    // Cursor on line 4 (inside the innermost manual range 3-4).
    h.remove_manual_ranges(&[cursor(4)]);
    h.assert_ranges(&[r(1, 6, false), r(2, 5, false)]);

    // Cursor on line 3 (inside the remaining manual range 2-5).
    h.remove_manual_ranges(&[cursor(3)]);
    h.assert_ranges(&[r(1, 6, false)]);
}

#[test]
fn remove_manual_ranges_cursor_skips_provider_ranges() {
    let mut h = Harness::from_ranges(&[(1, 5, true), (2, 4, false)]);

    h.remove_manual_ranges(&[cursor(3)]);
    h.assert_ranges(&[r(2, 4, false)]);
}

#[test]
fn remove_manual_ranges_cursor_not_on_manual_range_removes_all() {
    // Provider range at 2-4, manual range at 3-4.
    let mut h = Harness::from_ranges(&[(2, 4, false), (3, 4, true)]);
    h.assert_ranges(&[r(2, 4, false), r(3, 4, false)]);

    // A single-line selection outside manual ranges preserves them.
    h.remove_manual_ranges(&[selection(6, 6)]);
    h.assert_ranges(&[r(2, 4, false), r(3, 4, false)]);

    // A cursor on line 6 (not inside any manual range) removes them all.
    h.remove_manual_ranges(&[cursor(6)]);
    h.assert_ranges(&[r(2, 4, false)]);
}

#[test]
fn remove_manual_ranges_single_line_selection_removes_all_intersecting() {
    let mut h = Harness::from_ranges(&[(1, 6, false), (2, 5, true), (3, 4, true)]);

    h.remove_manual_ranges(&[selection(4, 4)]);
    h.assert_ranges(&[r(1, 6, false)]);
}

#[test]
fn remove_manual_ranges_selection_removes_intersecting() {
    let mut h = Harness::from_ranges(&[(1, 5, false), (2, 3, true), (4, 5, true)]);
    h.assert_ranges(&[r(1, 5, false), r(2, 3, false), r(4, 5, false)]);

    // Selection spanning lines 2-3 removes only the first manual range.
    h.remove_manual_ranges(&[selection(2, 3)]);
    h.assert_ranges(&[r(1, 5, false), r(4, 5, false)]);
}
