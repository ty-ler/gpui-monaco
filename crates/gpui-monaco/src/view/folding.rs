//! Folding ranges.
//!
//! [`compute_indent_ranges`] is ported from Monaco's
//! `contrib/folding/browser/indentRangeProvider.ts` (MIT): regions from
//! indentation, plus `#region`-style markers from the language configuration.

use syntax::Pattern;
use text::Rope;

/// A foldable region: `start_row` stays visible; rows
/// `start_row + 1 ..= end_row` hide when collapsed. Rows are 0-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldRange {
    /// The header row, which stays visible.
    pub start_row: u32,
    /// The last row hidden when collapsed (inclusive).
    pub end_row: u32,
    /// Index of the enclosing range in the same list.
    pub parent: Option<usize>,
    /// From a start/end marker rather than indentation.
    pub is_marker: bool,
}

/// Monaco `computeIndentLevel`: visible indent width, or `None` for a
/// whitespace-only line.
pub fn indent_level(line: &str, tab_size: u32) -> Option<u32> {
    let mut indent = 0;
    for ch in line.chars() {
        match ch {
            ' ' => indent += 1,
            '\t' => indent = indent - indent % tab_size + tab_size,
            _ => return Some(indent),
        }
    }
    None
}

struct Previous {
    /// -1: sentinel, -2: end marker.
    indent: i64,
    end_above: u32,
    line: u32,
}

/// Folding ranges for `rope` (Monaco 1-based logic, returned 0-based), sorted
/// by start row with parents set.
pub fn compute_indent_ranges(
    rope: &Rope,
    tab_size: u32,
    off_side: bool,
    markers: Option<(&Pattern, &Pattern)>,
) -> Vec<FoldRange> {
    let tab_size = tab_size.max(1);
    let line_count = rope.line_count();
    // (start, end, is_marker), collected bottom-up like Monaco's insertFirst.
    let mut collected: Vec<(u32, u32, bool)> = Vec::new();
    let mut previous = vec![Previous {
        indent: -1,
        end_above: line_count + 1,
        line: line_count + 1,
    }];
    // (indent, is marker start, is marker end) per row.
    let mut rows: Vec<(Option<u32>, bool, bool)> = Vec::with_capacity(line_count as usize);
    rope.for_each_line(0..rope.len(), |_, content| {
        let indent = indent_level(content, tab_size);
        let (is_start, is_end) = match (indent, markers) {
            (Some(_), Some((start, end))) => {
                let is_start = start.is_match(content);
                (is_start, !is_start && end.is_match(content))
            }
            _ => (false, false),
        };
        rows.push((indent, is_start, is_end));
        true
    });

    for line in (1..=line_count).rev() {
        let (indent, is_start, is_end) = rows[line as usize - 1];
        let Some(indent) = indent else {
            if off_side && let Some(last) = previous.last_mut() {
                last.end_above = line;
            }
            continue;
        };
        let indent = indent as i64;
        if is_start {
            let mut i = previous.len() - 1;
            while i > 0 && previous[i].indent != -2 {
                i -= 1;
            }
            if i > 0 {
                previous.truncate(i + 1);
                let end = previous[i].line;
                collected.push((line, end, true));
                let last = &mut previous[i];
                last.line = line;
                last.indent = indent;
                last.end_above = line;
                continue;
            }
        } else if is_end {
            previous.push(Previous {
                indent: -2,
                end_above: line,
                line,
            });
            continue;
        }
        let mut last_ix = previous.len() - 1;
        if previous[last_ix].indent > indent {
            while previous[last_ix].indent > indent {
                previous.pop();
                last_ix = previous.len() - 1;
            }
            let end_line = previous[last_ix].end_above - 1;
            if end_line > line {
                collected.push((line, end_line, false));
            }
        }
        if previous[last_ix].indent == indent {
            previous[last_ix].end_above = line;
        } else {
            previous.push(Previous {
                indent,
                end_above: line,
                line,
            });
        }
    }
    collected.reverse();
    let mut ranges: Vec<FoldRange> = Vec::with_capacity(collected.len());
    let mut stack: Vec<usize> = Vec::new();
    for (start, end, is_marker) in collected {
        // The parent is the nearest enclosing range (Monaco `FoldingRegions`).
        while let Some(&top) = stack.last() {
            let top = &ranges[top];
            if top.start_row < start - 1 && end - 1 <= top.end_row {
                break;
            }
            stack.pop();
        }
        let parent = stack.last().copied();
        ranges.push(FoldRange {
            start_row: start - 1,
            end_row: end - 1,
            parent,
            is_marker,
        });
        stack.push(ranges.len() - 1);
    }
    ranges
}

#[cfg(test)]
mod tests {
    //! Ported from Monaco's
    //! `contrib/folding/test/browser/indentRangeProvider.test.ts`.
    use super::*;

    /// (start, end, parent) with Monaco's 1-based lines and -1 for no parent.
    fn assert_ranges(lines: &[&str], expected: &[(u32, u32, i64)], off_side: bool, markers: bool) {
        let rope = Rope::from(lines.join("\n").as_str());
        let start = Pattern::new(r"^\s*#region\b", "");
        let end = Pattern::new(r"^\s*#endregion\b", "");
        let markers = markers.then_some((&start, &end));
        let actual: Vec<(u32, u32, i64)> = compute_indent_ranges(&rope, 4, off_side, markers)
            .iter()
            .map(|r| {
                (
                    r.start_row + 1,
                    r.end_row + 1,
                    r.parent.map_or(-1, |p| p as i64),
                )
            })
            .collect();
        assert_eq!(actual, expected, "{lines:?}");
    }

    #[test]
    fn indentation_folding() {
        let one = ["A", "  A", "  A", "  A"];
        assert_ranges(&one, &[(1, 4, -1)], true, false);
        assert_ranges(&one, &[(1, 4, -1)], false, false);
        let two = ["A", "  A", "  A", "    A", "    A"];
        assert_ranges(&two, &[(1, 5, -1), (3, 5, 0)], true, false);
        assert_ranges(&two, &[(1, 5, -1), (3, 5, 0)], false, false);
        let three = ["A", "  A", "    A", "      A", "A"];
        assert_ranges(&three, &[(1, 4, -1), (2, 4, 0), (3, 4, 1)], true, false);
        assert_ranges(&["    A", "  A", "A"], &[], true, false);
    }

    #[test]
    fn fold_java_javadoc_whitespace_tabs() {
        assert_ranges(
            &[
                "class A {",
                "  void foo() {",
                "    console.log();",
                "    console.log();",
                "  }",
                "",
                "  void bar() {",
                "    console.log();",
                "  }",
                "}",
                "interface B {",
                "  void bar();",
                "}",
            ],
            &[(1, 9, -1), (2, 4, 0), (7, 8, 0), (11, 12, -1)],
            false,
            false,
        );
        assert_ranges(
            &[
                "/**",
                " * Comment",
                " */",
                "class A {",
                "  void foo() {",
                "  }",
                "}",
            ],
            &[(1, 3, -1), (4, 6, -1)],
            false,
            false,
        );
        assert_ranges(
            &[
                "class A {",
                "",
                "  void foo() {",
                "     ",
                "     return 0;",
                "  }",
                "      ",
                "}",
            ],
            &[(1, 7, -1), (3, 5, 0)],
            false,
            false,
        );
        assert_ranges(
            &[
                "def a:",
                "  pass",
                "   ",
                "  def b:",
                "    pass",
                "  ",
                "      ",
                "def c: # since there was a deintent here",
            ],
            &[(1, 5, -1), (4, 5, 0)],
            true,
            false,
        );
        assert_ranges(
            &[
                "class A {",
                "\t\t",
                "\tvoid foo() {",
                "\t \t//hello",
                "\t    return 0;",
                "  \t}",
                "      ",
                "}",
            ],
            &[(1, 7, -1), (3, 5, 0)],
            false,
            false,
        );
    }

    #[test]
    fn folding_with_regions() {
        assert_ranges(
            &[
                "class A {",
                "  #region",
                "  void foo() {",
                "     ",
                "     return 0;",
                "  }",
                "  #endregion",
                "}",
            ],
            &[(1, 7, -1), (2, 7, 0), (3, 5, 1)],
            false,
            true,
        );
        assert_ranges(
            &[
                "var x;",
                "#region",
                "void foo() {",
                "     ",
                "     return 0;",
                "  }",
                "#endregion",
                "",
            ],
            &[(2, 7, -1), (3, 6, 0)],
            false,
            true,
        );
        assert_ranges(
            &[
                "var x;",
                "#region",
                "#endregion",
                "#region",
                "",
                "#endregion",
                "var y;",
            ],
            &[(2, 3, -1), (4, 6, -1)],
            false,
            true,
        );
        assert_ranges(
            &[
                "var x;",
                "#region",
                "#region",
                "",
                "#endregion",
                "#endregion",
                "var y;",
            ],
            &[(2, 6, -1), (3, 5, 0)],
            false,
            true,
        );
        assert_ranges(
            &[
                "class A {",
                "  #region",
                "",
                "  #region",
                "",
                "  #endregion",
                "  // comment",
                "  #endregion",
                "}",
            ],
            &[(1, 8, -1), (2, 8, 0), (4, 6, 1)],
            false,
            true,
        );
        assert_ranges(
            &["class A {", "#region", "  // comment", "}"],
            &[(2, 3, -1)],
            false,
            true,
        );
        assert_ranges(
            &[
                "",
                "#region",
                "#region",
                "#region",
                "  // comment",
                "#endregion",
                "#endregion",
                " // hello",
            ],
            &[(3, 7, -1), (4, 6, 0)],
            false,
            true,
        );
        assert_ranges(
            &[
                "if (x)",
                "  return;",
                "",
                "#region",
                "  // comment",
                "#endregion",
            ],
            &[(1, 3, -1), (4, 6, -1)],
            false,
            true,
        );
    }

    #[test]
    fn collapsed_regions_shrunk_to_one_row_are_pruned() {
        let mut buffer = text::Buffer::new("a\n  b\n  c\nd");
        let mut state = FoldState::default();
        state.collapse(&buffer, 0, 2);
        assert_eq!(state.collapsed_rows(&buffer), vec![(0, 2)]);
        // Delete the folded lines: the region is a single row now.
        buffer.edit([(1..9, "")]).unwrap();
        assert!(state.collapsed_rows(&buffer).is_empty());
        state.prune(&buffer);
        assert!(!state.has_collapsed());
    }

    #[test]
    fn enclosing_lookups_match_brute_force() {
        // Deterministic pseudo-random indentation.
        let mut seed = 7u64;
        let mut lines = Vec::new();
        for _ in 0..400 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let indent = (seed >> 33) % 6;
            lines.push(format!("{}x", "  ".repeat(indent as usize)));
        }
        let rope = Rope::from(lines.join("\n").as_str());
        let mut state = FoldState::default();
        state.set_ranges(
            compute_indent_ranges(&rope, 4, false, None),
            text::Buffer::new("").version(),
        );
        for row in 0..402 {
            let brute: Vec<_> = state
                .ranges()
                .iter()
                .filter(|r| r.start_row < row && row <= r.end_row)
                .map(|r| (r.start_row, r.end_row))
                .collect();
            let fast: Vec<_> = state
                .ranges_enclosing(row)
                .iter()
                .map(|r| (r.start_row, r.end_row))
                .collect();
            assert_eq!(fast, brute, "row {row}");
            let innermost = state
                .ranges()
                .iter()
                .filter(|r| r.start_row <= row && row <= r.end_row)
                .max_by_key(|r| r.start_row)
                .map(|r| (r.start_row, r.end_row));
            assert_eq!(
                state
                    .innermost_range_containing(row)
                    .map(|r| (r.start_row, r.end_row)),
                innermost,
                "row {row}"
            );
        }
    }
}

/// Fold ranges plus which are collapsed. Collapsed regions are tracked with
/// anchors so they follow edits.
#[derive(Clone, Default)]
pub struct FoldState {
    ranges: Vec<FoldRange>,
    ranges_version: Option<text::Version>,
    /// (start of the start row, end of the end row).
    collapsed: Vec<std::ops::Range<text::Anchor>>,
}

impl FoldState {
    /// The fold ranges, sorted by start row (as from
    /// [`compute_indent_ranges`]).
    pub fn ranges(&self) -> &[FoldRange] {
        &self.ranges
    }

    /// The text version [`Self::ranges`] were computed for; `None` until
    /// [`Self::set_ranges`] is first called.
    pub fn ranges_version(&self) -> Option<text::Version> {
        self.ranges_version
    }

    /// Replaces the fold ranges with ones computed for text `version`.
    /// Collapsed regions are kept.
    pub fn set_ranges(&mut self, ranges: Vec<FoldRange>, version: text::Version) {
        self.ranges = ranges;
        self.ranges_version = Some(version);
    }

    /// Collapsed regions as inclusive model row ranges `(start, end)`.
    pub fn collapsed_rows(&self, buffer: &text::Buffer) -> Vec<(u32, u32)> {
        let mut rows: Vec<(u32, u32)> = self
            .collapsed
            .iter()
            .map(|range| {
                let start = buffer.offset_to_point(buffer.resolve(&range.start)).row;
                let end = buffer.offset_to_point(buffer.resolve(&range.end)).row;
                (start, end.max(start))
            })
            .filter(|(start, end)| end > start)
            .collect();
        rows.sort();
        rows
    }

    /// Model rows hidden by collapsed regions.
    pub fn hidden_rows(&self, buffer: &text::Buffer) -> Vec<std::ops::Range<u32>> {
        self.collapsed_rows(buffer)
            .into_iter()
            .map(|(start, end)| start + 1..end + 1)
            .collect()
    }

    /// True if a collapsed region starts at model row `start_row`.
    pub fn is_collapsed(&self, buffer: &text::Buffer, start_row: u32) -> bool {
        self.collapsed_rows(buffer)
            .iter()
            .any(|(start, _)| *start == start_row)
    }

    /// The innermost fold range containing `row` (whose start row is `row`
    /// or above it).
    pub fn innermost_range_containing(&self, row: u32) -> Option<&FoldRange> {
        self.chain_from(self.ranges.partition_point(|r| r.start_row <= row), row)
            .next()
    }

    /// The ranges starting above `row` and ending at or below it, outermost
    /// first (the sticky scroll headers for `row`).
    pub fn ranges_enclosing(&self, row: u32) -> Vec<&FoldRange> {
        let mut chain: Vec<&FoldRange> = self
            .chain_from(self.ranges.partition_point(|r| r.start_row < row), row)
            .collect();
        chain.reverse();
        chain
    }

    /// Ranges sorted by start and nested via `parent`: the last range
    /// starting before `end` that contains `row`, then its ancestors.
    fn chain_from(&self, end: usize, row: u32) -> impl Iterator<Item = &FoldRange> {
        // A range starting before `end` that contains `row` encloses the last
        // such range or ends before it starts, so walk up the parents.
        let mut ix = end.checked_sub(1);
        while let Some(i) = ix
            && self.ranges[i].end_row < row
        {
            ix = self.ranges[i].parent;
        }
        std::iter::successors(ix.map(|i| &self.ranges[i]), |range| {
            range.parent.map(|p| &self.ranges[p])
        })
    }

    /// The first fold range whose start row is `row`, if any.
    pub fn range_starting_at(&self, row: u32) -> Option<&FoldRange> {
        let ix = self.ranges.partition_point(|range| range.start_row < row);
        self.ranges.get(ix).filter(|range| range.start_row == row)
    }

    /// Collapses model rows `start_row + 1 ..= end_row` under `start_row`.
    /// Does nothing if `end_row <= start_row` or a region starting at
    /// `start_row` is already collapsed.
    pub fn collapse(&mut self, buffer: &text::Buffer, start_row: u32, end_row: u32) {
        if end_row <= start_row || self.is_collapsed(buffer, start_row) {
            return;
        }
        let start = buffer.anchor_before(buffer.point_to_offset(text::Point::new(start_row, 0)));
        let end = buffer.anchor_after(buffer.point_to_offset(text::Point::new(end_row, u32::MAX)));
        self.collapsed.push(start..end);
    }

    /// Forgets collapsed regions that edits shrank to a single row (Monaco
    /// drops folding regions whose range became empty).
    pub fn prune(&mut self, buffer: &text::Buffer) {
        self.collapsed.retain(|range| {
            let start = buffer.offset_to_point(buffer.resolve(&range.start)).row;
            let end = buffer.offset_to_point(buffer.resolve(&range.end)).row;
            end > start
        });
    }

    /// Expands collapsed regions starting at `start_row`.
    pub fn expand(&mut self, buffer: &text::Buffer, start_row: u32) -> bool {
        let before = self.collapsed.len();
        self.collapsed
            .retain(|range| buffer.offset_to_point(buffer.resolve(&range.start)).row != start_row);
        self.collapsed.len() != before
    }

    /// Expands collapsed regions whose hidden rows intersect `rows`.
    pub fn expand_intersecting(
        &mut self,
        buffer: &text::Buffer,
        rows: std::ops::Range<u32>,
    ) -> bool {
        let before = self.collapsed.len();
        self.collapsed.retain(|range| {
            let start = buffer.offset_to_point(buffer.resolve(&range.start)).row;
            let end = buffer.offset_to_point(buffer.resolve(&range.end)).row;
            !(start + 1 < rows.end && rows.start <= end)
        });
        self.collapsed.len() != before
    }

    /// Collapses every fold range.
    pub fn collapse_all(&mut self, buffer: &text::Buffer) {
        let ranges = self.ranges.clone();
        for range in ranges {
            self.collapse(buffer, range.start_row, range.end_row);
        }
    }

    /// Expands every collapsed region.
    pub fn expand_all(&mut self) {
        self.collapsed.clear();
    }

    /// True if any region is collapsed.
    pub fn has_collapsed(&self) -> bool {
        !self.collapsed.is_empty()
    }
}
