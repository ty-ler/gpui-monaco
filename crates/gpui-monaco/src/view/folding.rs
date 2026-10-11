//! Folding ranges.
//!
//! [`compute_indent_ranges`] is ported from Monaco's
//! `contrib/folding/browser/indentRangeProvider.ts` (MIT): regions from
//! indentation, plus `#region`-style markers from the language configuration.
//! [`sanitize_ranges`] is `sanitizeRanges` from `syntaxRangeProvider.ts`,
//! for provider and syntax ranges.

use syntax::Pattern;
use text::Rope;

pub use crate::providers::FoldingRangeKind;

/// A foldable region: `start_row` stays visible; rows
/// `start_row + 1 ..= end_row` hide when collapsed. Rows are 0-based.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldRange {
    /// The header row, which stays visible.
    pub start_row: u32,
    /// The last row hidden when collapsed (inclusive).
    pub end_row: u32,
    /// Index of the enclosing range in the same list.
    pub parent: Option<usize>,
    /// What the range holds; markers make [`FoldingRangeKind::Region`]s.
    pub kind: Option<FoldingRangeKind>,
    /// Made by "Create Folding Range from Selection" (Monaco
    /// `FoldSource.userDefined`).
    pub manual: bool,
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
    compute_indent_ranges_limited(rope, tab_size, off_side, markers, usize::MAX)
}

/// [`compute_indent_ranges`] keeping at most `limit` ranges (Monaco's
/// indent `RangesCollector`): the least indented first.
pub fn compute_indent_ranges_limited(
    rope: &Rope,
    tab_size: u32,
    off_side: bool,
    markers: Option<(&Pattern, &Pattern)>,
    limit: usize,
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
    if collected.len() > limit {
        let indent = |line: u32| indent_level(&rope.line(line - 1), tab_size).unwrap_or(0) as usize;
        let mut occurrences: Vec<usize> = Vec::new();
        for (start, _, _) in &collected {
            let i = indent(*start);
            if i < 1000 {
                if i >= occurrences.len() {
                    occurrences.resize(i + 1, 0);
                }
                occurrences[i] += 1;
            }
        }
        let mut entries = 0;
        let mut max_indent = occurrences.len();
        for (i, n) in occurrences.iter().enumerate() {
            if *n > 0 {
                if n + entries > limit {
                    max_indent = i;
                    break;
                }
                entries += n;
            }
        }
        collected.retain(|(start, _, _)| {
            let i = indent(*start);
            if i < max_indent {
                true
            } else if i == max_indent && entries < limit {
                entries += 1;
                true
            } else {
                false
            }
        });
    }
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
            kind: is_marker.then_some(FoldingRangeKind::Region),
            manual: false,
        });
        stack.push(ranges.len() - 1);
    }
    ranges
}

/// A range from a provider or the syntax tree, before sanitizing (Monaco
/// `IFoldingRangeData`, 0-based).
#[derive(Clone, Debug)]
pub struct FoldingRangeData {
    /// First row.
    pub start: u32,
    /// Last row folded.
    pub end: u32,
    /// Ranges with lower ranks win at the same start (provider order).
    pub rank: usize,
    /// Kind.
    pub kind: Option<FoldingRangeKind>,
    /// A manual range.
    pub manual: bool,
}

/// Monaco `sanitizeRanges` and `RangesCollector`: sorts ranges, drops
/// those that would cross an enclosing one, and keeps at most `limit`,
/// dropping the most deeply nested first. Returns the ranges with parents,
/// and whether some were dropped for the limit.
pub fn sanitize_ranges(mut data: Vec<FoldingRangeData>, limit: usize) -> (Vec<FoldRange>, bool) {
    data.sort_by(|a, b| a.start.cmp(&b.start).then(a.rank.cmp(&b.rank)));
    // (start, end, kind, nesting level)
    let mut collected: Vec<(u32, u32, Option<FoldingRangeKind>, bool, usize)> = Vec::new();
    let mut top: Option<FoldingRangeData> = None;
    let mut previous: Vec<FoldingRangeData> = Vec::new();
    for entry in data {
        let Some(current) = top.clone() else {
            collected.push((
                entry.start,
                entry.end,
                entry.kind.clone(),
                entry.manual,
                previous.len(),
            ));
            top = Some(entry);
            continue;
        };
        if entry.start <= current.start {
            continue;
        }
        if entry.end <= current.end {
            previous.push(current);
            collected.push((
                entry.start,
                entry.end,
                entry.kind.clone(),
                entry.manual,
                previous.len(),
            ));
            top = Some(entry);
        } else {
            if entry.start > current.end {
                let mut next = previous.pop();
                while next.as_ref().is_some_and(|t| entry.start > t.end) {
                    next = previous.pop();
                }
                if let Some(next) = next {
                    previous.push(next);
                }
                collected.push((
                    entry.start,
                    entry.end,
                    entry.kind.clone(),
                    entry.manual,
                    previous.len(),
                ));
                top = Some(entry);
            } else {
                // Crosses the enclosing range's end: Monaco still adds it.
                collected.push((
                    entry.start,
                    entry.end,
                    entry.kind.clone(),
                    entry.manual,
                    previous.len(),
                ));
            }
        }
    }
    let limited = collected.len() > limit;
    if limited {
        // Keep whole nesting levels from the top while they fit, then fill
        // with the next level in order.
        let mut counts: Vec<usize> = Vec::new();
        for (_, _, _, _, level) in &collected {
            if *level >= counts.len() {
                counts.resize(level + 1, 0);
            }
            counts[*level] += 1;
        }
        let mut entries = 0;
        let mut max_level = counts.len();
        for (level, n) in counts.iter().enumerate() {
            if *n > 0 {
                if n + entries > limit {
                    max_level = level;
                    break;
                }
                entries += n;
            }
        }
        collected.retain(|(_, _, _, _, level)| {
            if *level < max_level {
                true
            } else if *level == max_level && entries < limit {
                entries += 1;
                true
            } else {
                false
            }
        });
    }
    let mut ranges: Vec<FoldRange> = Vec::with_capacity(collected.len());
    let mut stack: Vec<usize> = Vec::new();
    for (start, end, kind, manual, _) in collected {
        while let Some(&parent) = stack.last() {
            if ranges[parent].start_row < start && end <= ranges[parent].end_row {
                break;
            }
            stack.pop();
        }
        ranges.push(FoldRange {
            start_row: start,
            end_row: end,
            parent: stack.last().copied(),
            kind,
            manual,
        });
        stack.push(ranges.len() - 1);
    }
    (ranges, limited)
}

/// Region marker ranges (`#region` … `#endregion`), for syntax folding,
/// whose trees don't know them.
pub fn marker_ranges(rope: &Rope, start: &Pattern, end: &Pattern) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    let mut open: Vec<u32> = Vec::new();
    for row in 0..rope.line_count() {
        let line = rope.line(row);
        if start.is_match(&line) {
            open.push(row);
        } else if end.is_match(&line)
            && let Some(first) = open.pop()
            && row > first
        {
            ranges.push((first, row));
        }
    }
    ranges.sort_unstable();
    ranges
}

#[cfg(test)]
mod syntax_tests {
    //! Ported from Monaco's `contrib/folding/test/browser/syntaxFold.test.ts`.
    use super::*;

    #[test]
    fn limit_by_nesting_level() {
        // Monaco's 1-based lines.
        let r = |start: u32, end: u32| (start, end);
        let (r1, r2, r3, r4, r5, r6, r7, r8, r9) = (
            r(1, 20),
            r(3, 19),
            r(4, 5),
            r(7, 18),
            r(9, 10),
            r(12, 17),
            r(13, 16),
            r(14, 15),
            r(22, 23),
        );
        let all = [r1, r2, r3, r4, r5, r6, r7, r8, r9];
        let data = || {
            all.iter()
                .map(|&(start, end)| FoldingRangeData {
                    start: start - 1,
                    end: end - 1,
                    rank: 0,
                    kind: None,
                    manual: false,
                })
                .collect::<Vec<_>>()
        };
        let check = |limit: usize, expected: &[(u32, u32)]| {
            let (ranges, limited) = sanitize_ranges(data(), limit);
            let actual: Vec<(u32, u32)> = ranges
                .iter()
                .map(|r| (r.start_row + 1, r.end_row + 1))
                .collect();
            assert_eq!(actual, expected, "{limit}");
            assert_eq!(limited, limit < 9, "limited {limit}");
        };
        check(1000, &all);
        check(9, &all);
        check(8, &[r1, r2, r3, r4, r5, r6, r7, r9]);
        check(7, &[r1, r2, r3, r4, r5, r6, r9]);
        check(6, &[r1, r2, r3, r4, r5, r9]);
        check(5, &[r1, r2, r3, r4, r9]);
        check(4, &[r1, r2, r3, r9]);
        check(3, &[r1, r2, r9]);
        check(2, &[r1, r9]);
        check(1, &[r1]);
        check(0, &[]);
    }

    #[test]
    fn crossing_and_duplicate_starts() {
        let data = |ranges: &[(u32, u32, usize)]| {
            ranges
                .iter()
                .map(|&(start, end, rank)| FoldingRangeData {
                    start,
                    end,
                    rank,
                    kind: None,
                    manual: false,
                })
                .collect()
        };
        // The same start: the lower rank wins.
        let (ranges, _) = sanitize_ranges(data(&[(0, 5, 1), (0, 3, 0)]), 100);
        assert_eq!((ranges[0].start_row, ranges[0].end_row), (0, 3));
        assert_eq!(ranges.len(), 1);
        // Parents.
        let (ranges, _) = sanitize_ranges(data(&[(0, 9, 0), (2, 4, 0), (6, 8, 0)]), 100);
        let parents: Vec<_> = ranges.iter().map(|r| r.parent).collect();
        assert_eq!(parents, [None, Some(0), Some(0)]);
    }

    #[test]
    fn markers() {
        let rope = Rope::from(
            "#region a
x
  #region b
  y
  #endregion
#endregion",
        );
        let start = Pattern::new(r"^\s*#region\b", "");
        let end = Pattern::new(r"^\s*#endregion\b", "");
        assert_eq!(marker_ranges(&rope, &start, &end), [(0, 5), (2, 4)]);
    }
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
        let buffer = text::Buffer::new("");
        state.set_ranges(
            compute_indent_ranges(&rope, 4, false, None),
            buffer.version(),
            &buffer,
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
    /// Computed and manual ranges, merged.
    ranges: Vec<FoldRange>,
    /// The computed ranges.
    computed: Vec<FoldRange>,
    ranges_version: Option<text::Version>,
    /// (start of the start row, end of the end row).
    collapsed: Vec<std::ops::Range<text::Anchor>>,
    /// Manual ranges (Monaco's user-defined folding ranges), which survive
    /// recomputing.
    manual: Vec<std::ops::Range<text::Anchor>>,
    /// The computed ranges came from providers, with kinds (Monaco
    /// `FoldingRegions.hasTypes`).
    has_types: bool,
}

impl FoldState {
    /// The fold ranges (computed and manual), sorted by start row.
    pub fn ranges(&self) -> &[FoldRange] {
        &self.ranges
    }

    /// The text version [`Self::ranges`] were computed for; `None` until
    /// [`Self::set_ranges`] is first called.
    pub fn ranges_version(&self) -> Option<text::Version> {
        self.ranges_version
    }

    /// Replaces the computed fold ranges with ones for text `version`.
    /// Collapsed regions and manual ranges are kept.
    pub fn set_ranges(
        &mut self,
        ranges: Vec<FoldRange>,
        version: text::Version,
        buffer: &text::Buffer,
    ) {
        self.computed = ranges;
        self.ranges_version = Some(version);
        self.merge(buffer);
    }

    fn anchor_rows(buffer: &text::Buffer, range: &std::ops::Range<text::Anchor>) -> (u32, u32) {
        let start = buffer.offset_to_point(buffer.resolve(&range.start)).row;
        let end = buffer.offset_to_point(buffer.resolve(&range.end)).row;
        (start, end.max(start))
    }

    /// Monaco `FoldingRegions.sanitizeAndMerge`: the manual ranges join the
    /// computed ones, winning ties.
    fn merge(&mut self, buffer: &text::Buffer) {
        if self.manual.is_empty() {
            self.ranges = self.computed.clone();
            return;
        }
        let mut data: Vec<FoldingRangeData> = self
            .manual
            .iter()
            .map(|range| Self::anchor_rows(buffer, range))
            .filter(|(start, end)| end > start)
            .map(|(start, end)| FoldingRangeData {
                start,
                end,
                rank: 0,
                kind: None,
                manual: true,
            })
            .collect();
        data.extend(self.computed.iter().map(|range| FoldingRangeData {
            start: range.start_row,
            end: range.end_row,
            rank: 1,
            kind: range.kind.clone(),
            manual: false,
        }));
        self.ranges = sanitize_ranges(data, usize::MAX).0;
    }

    /// Adds a collapsed manual range over rows `start_row ..= end_row`
    /// (Monaco "Create Folding Range from Selection").
    pub fn add_manual(&mut self, buffer: &text::Buffer, start_row: u32, end_row: u32) {
        if end_row <= start_row {
            return;
        }
        let start = buffer.anchor_before(buffer.point_to_offset(text::Point::new(start_row, 0)));
        let end = buffer.anchor_after(buffer.point_to_offset(text::Point::new(end_row, u32::MAX)));
        self.manual.push(start..end);
        self.merge(buffer);
        self.expand(buffer, start_row);
        self.collapse(buffer, start_row, end_row);
    }

    /// Removes manual ranges (Monaco "Remove Manual Folding Ranges") and
    /// expands them.
    pub fn remove_manual(
        &mut self,
        buffer: &text::Buffer,
        selections: &[crate::view::fold_model::SelectedRows],
    ) {
        let collapsed = self.collapsed_flags(buffer);
        let model = crate::view::fold_model::FoldingModel::new(&self.ranges, collapsed);
        let removed: Vec<(u32, u32)> =
            crate::view::fold_model::manual_ranges_to_remove(&model, selections)
                .into_iter()
                .map(|ix| (self.ranges[ix].start_row, self.ranges[ix].end_row))
                .collect();
        if removed.is_empty() {
            return;
        }
        self.manual
            .retain(|range| !removed.contains(&Self::anchor_rows(buffer, range)));
        for (start, _) in &removed {
            self.expand(buffer, *start);
        }
        self.merge(buffer);
    }

    /// The manual ranges as inclusive model row ranges `(start, end)`.
    pub fn manual_rows(&self, buffer: &text::Buffer) -> Vec<(u32, u32)> {
        let mut rows: Vec<(u32, u32)> = self
            .manual
            .iter()
            .map(|range| Self::anchor_rows(buffer, range))
            .filter(|(start, end)| end > start)
            .collect();
        rows.sort();
        rows
    }

    /// Whether the ranges came from providers, with kinds: commands then
    /// pick comments and regions by kind rather than by their first line.
    pub fn has_types(&self) -> bool {
        self.has_types
    }

    /// Records whether the computed ranges came from providers.
    pub fn set_has_types(&mut self, has_types: bool) {
        self.has_types = has_types;
    }

    /// For each range, whether it's collapsed.
    pub fn collapsed_flags(&self, buffer: &text::Buffer) -> Vec<bool> {
        let collapsed: Vec<u32> = self
            .collapsed_rows(buffer)
            .into_iter()
            .map(|(start, _)| start)
            .collect();
        self.ranges
            .iter()
            .map(|range| collapsed.binary_search(&range.start_row).is_ok())
            .collect()
    }

    /// Toggles the ranges at `indexes` (Monaco
    /// `FoldingModel.toggleCollapseState`): each once, however often listed.
    pub fn toggle(&mut self, buffer: &text::Buffer, mut indexes: Vec<usize>) {
        indexes.sort_unstable();
        indexes.dedup();
        let collapsed = self.collapsed_flags(buffer);
        for ix in indexes {
            let range = &self.ranges[ix];
            let (start, end) = (range.start_row, range.end_row);
            if collapsed[ix] {
                self.expand(buffer, start);
            } else {
                self.collapse(buffer, start, end);
            }
        }
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
