//! Folding ranges.
//!
//! [`compute_indent_ranges`] is ported from Monaco's
//! `contrib/folding/browser/indentRangeProvider.ts` (MIT): regions from
//! indentation, plus `#region`-style markers from the language configuration.
//! [`sanitize_ranges`] is `sanitizeRanges` from `syntaxRangeProvider.ts`,
//! for provider and syntax ranges. [`sanitize_and_merge`] is
//! `FoldingRegions.sanitizeAndMerge` from `foldingRanges.ts`, and
//! [`FoldState`] ports `foldingModel.ts` and `hiddenRangeModel.ts`.

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
    /// A collapsed region kept after the recomputed ranges dropped it
    /// (Monaco `FoldSource.recovered`).
    pub recovered: bool,
}

impl FoldRange {
    /// Where the region came from (Monaco `FoldSource`).
    pub fn source(&self) -> FoldSource {
        if self.manual {
            FoldSource::UserDefined
        } else if self.recovered {
            FoldSource::Recovered
        } else {
            FoldSource::Provider
        }
    }

    /// The region came from the provider: it isn't manual or recovered
    /// (Monaco shows the others with the manual icons and removes them
    /// with "Remove Manual Folding Ranges").
    pub fn is_provided(&self) -> bool {
        self.source() == FoldSource::Provider
    }
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
            recovered: false,
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
            recovered: false,
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
        let buffer = text::Buffer::new(&lines.join("\n"));
        let rope = buffer.rope().clone();
        let mut state = FoldState::default();
        state.set_ranges(
            compute_indent_ranges(&rope, 4, false, None),
            buffer.version(),
            &rope,
            &buffer,
            &[],
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

/// Where a region came from (Monaco `FoldSource`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FoldSource {
    /// Computed (a folding range provider, the syntax tree or indentation).
    #[default]
    Provider,
    /// Made by "Create Folding Range from Selection".
    UserDefined,
    /// A collapsed region that the latest computed ranges no longer have,
    /// kept collapsed.
    Recovered,
}

/// A region for [`sanitize_and_merge`] (Monaco `FoldRange` of
/// `foldingRanges.ts`), 0-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRange {
    /// The header row.
    pub start: u32,
    /// The last row hidden when collapsed.
    pub end: u32,
    /// Its kind.
    pub kind: Option<FoldingRangeKind>,
    /// Whether it's collapsed.
    pub collapsed: bool,
    /// Where it came from.
    pub source: FoldSource,
}

/// Monaco `FoldingRegions.sanitizeAndMerge`: merges the regions `a` (the
/// computed ones) with `b` (the collapsed and manual ones), both sorted by
/// start. At the same start a user-defined `b` wins; otherwise `a` takes
/// `b`'s collapsed state, unless its end moved and a selection starts in it
/// or right after it. A user-defined `b` drops the `a`s it crosses, and a
/// collapsed computed `b` that matches no `a` is kept, recovered. Ranges
/// that are empty, start where the previous one did, end at or past
/// `line_count`, or aren't inside their enclosing range are dropped.
/// `selection` holds each selection's first row.
pub fn sanitize_and_merge(
    a: &[MergeRange],
    b: &[MergeRange],
    line_count: u32,
    selection: Option<&[u32]>,
) -> Vec<MergeRange> {
    let starts_inside = |start: u32, end: u32| {
        selection.is_some_and(|rows| rows.iter().any(|&row| start <= row && row <= end))
    };
    let (mut ia, mut ib) = (0, 0);
    let mut stacked: Vec<(u32, u32)> = Vec::new();
    let mut top: Option<(u32, u32)> = None;
    let mut previous: Option<u32> = None;
    let mut result = Vec::new();
    while ia < a.len() || ib < b.len() {
        let (next_a, next_b) = (a.get(ia), b.get(ib));
        let mut use_range = None;
        match next_b {
            Some(nb) if next_a.is_none_or(|na| na.start >= nb.start) => {
                match next_a {
                    Some(na) if na.start == nb.start => {
                        if nb.source == FoldSource::UserDefined {
                            use_range = Some(nb.clone());
                        } else {
                            // A previously collapsed or recovered range: it
                            // stays collapsed if it has the same lines, or
                            // no selection starts in it or right after it.
                            let mut range = na.clone();
                            range.collapsed = nb.collapsed
                                && (na.end == nb.end || !starts_inside(na.start + 1, na.end + 1));
                            range.source = FoldSource::Provider;
                            use_range = Some(range);
                        }
                        ia += 1;
                    }
                    _ => {
                        let mut range = nb.clone();
                        if nb.collapsed && nb.source == FoldSource::Provider {
                            range.source = FoldSource::Recovered;
                        }
                        use_range = Some(range);
                    }
                }
                ib += 1;
            }
            _ => {
                let na = next_a.expect("one of the lists has ranges left");
                // A user-defined range in `b` ending after this one wins.
                let mut scan = ib;
                loop {
                    match b.get(scan) {
                        Some(nb) if nb.start <= na.end => {
                            if nb.source == FoldSource::UserDefined && nb.end > na.end {
                                break;
                            }
                            scan += 1;
                        }
                        _ => {
                            use_range = Some(na.clone());
                            break;
                        }
                    }
                }
                ia += 1;
            }
        }
        if let Some(range) = use_range {
            while let Some((_, end)) = top
                && end < range.start
            {
                top = stacked.pop();
            }
            if range.end > range.start
                && previous.is_none_or(|previous| range.start > previous)
                && range.end < line_count
                && top.is_none_or(|(_, end)| end >= range.end)
            {
                previous = Some(range.start);
                if let Some(top) = top {
                    stacked.push(top);
                }
                top = Some((range.start, range.end));
                result.push(range);
            }
        }
    }
    result
}

/// One edit in points, for moving tracked positions.
#[derive(Clone, Copy, Debug)]
struct PointEdit {
    old_start: text::Point,
    old_end: text::Point,
    new_start: text::Point,
    new_end: text::Point,
}

/// Where `point` moves through `edits` (sorted, in old coordinates), as
/// [`text::Patch::transform_offset`] moves an offset: inside an edit it
/// goes to the edit's start (`Bias::Left`) or end (`Bias::Right`, or the
/// end of a deletion).
fn transform_point(point: text::Point, bias: text::Bias, edits: &[PointEdit]) -> text::Point {
    let ix = edits.partition_point(|edit| edit.old_end < point);
    match edits.get(ix) {
        Some(edit) if edit.old_start <= point => {
            let at_end = point == edit.old_end && edit.old_start < edit.old_end;
            if at_end || bias == text::Bias::Right {
                edit.new_end
            } else {
                edit.new_start
            }
        }
        _ => match ix.checked_sub(1).map(|ix| &edits[ix]) {
            None => point,
            Some(edit) if point.row == edit.old_end.row => text::Point::new(
                edit.new_end.row,
                point.column - edit.old_end.column + edit.new_end.column,
            ),
            Some(edit) => text::Point::new(
                point.row - edit.old_end.row + edit.new_end.row,
                point.column,
            ),
        },
    }
}

/// The folding regions and the rows they hide (Monaco's `FoldingModel`
/// and `HiddenRangeModel`).
///
/// Like Monaco's folding decorations, each region is tracked from the end
/// of its header row to the end of its last row and moves with edits at
/// once (text typed at the end of the last row joins it); recomputed ranges
/// are merged in with [`sanitize_and_merge`]. The hidden rows follow edits
/// separately, as Monaco's view model lines do (a line keeps its
/// visibility, and inserted lines take that of the line they're inserted
/// after), and are derived from the collapsed regions again whenever those
/// change, so a line typed after a collapsed header shows until the regions
/// are updated.
#[derive(Clone, Default)]
pub struct FoldState {
    /// The regions, sorted by start row and nested via `parent`.
    ranges: Vec<FoldRange>,
    /// For each region, whether it's collapsed.
    collapsed: Vec<bool>,
    /// For each region, the end of its header row and of its last row.
    positions: Vec<(text::Point, text::Point)>,
    /// The hidden rows, sorted and disjoint.
    hidden_rows: Vec<std::ops::Range<u32>>,
    /// Rows changed since the hidden areas were last derived (Monaco
    /// `HiddenRangeModel._hasLineChanges`).
    hidden_line_changes: bool,
    /// The text the positions are for.
    tracked: Option<(text::Version, Rope)>,
    ranges_version: Option<text::Version>,
    /// The computed ranges came from providers, with kinds (Monaco
    /// `FoldingRegions.hasTypes`).
    has_types: bool,
    /// Renewed whenever the regions or their rows change (unique across
    /// states).
    generation: u64,
}

impl FoldState {
    /// The regions (computed, manual and recovered), sorted by start row,
    /// with their current rows.
    pub fn ranges(&self) -> &[FoldRange] {
        &self.ranges
    }

    /// The text version the ranges were last computed for; `None` until
    /// [`Self::set_ranges`] is first called.
    pub fn ranges_version(&self) -> Option<text::Version> {
        self.ranges_version
    }

    /// Changes whenever the regions or their rows do, for caches.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Moves the regions and hidden areas through the edits made to
    /// `buffer` since the last call, as Monaco's decorations follow edits.
    /// Returns whether any row changed.
    pub fn sync(&mut self, buffer: &text::Buffer) -> bool {
        let version = buffer.version();
        let Some((old_version, old_rope)) = self.tracked.take() else {
            self.tracked = Some((version, buffer.rope().clone()));
            return false;
        };
        self.tracked = Some((version, buffer.rope().clone()));
        if old_version == version || (self.ranges.is_empty() && self.hidden_rows.is_empty()) {
            return false;
        }
        let edits = point_edits(&old_rope, buffer.rope(), &buffer.edits_since(old_version));
        if edits.is_empty() {
            return false;
        }
        // Typing within lines moves only the positions on the edited rows,
        // as an interval tree would find them: no row changes.
        if edits.iter().all(|edit| {
            edit.old_end.row == edit.old_start.row && edit.new_end.row == edit.old_start.row
        }) {
            let mut previous = None;
            for edit in &edits {
                let row = edit.old_start.row;
                if previous != Some(row) {
                    self.move_within_row(row, &edits);
                    previous = Some(row);
                }
            }
            return false;
        }
        let mut rows_changed = false;
        for (range, (start, end)) in self.ranges.iter_mut().zip(&mut self.positions) {
            *start = transform_point(*start, text::Bias::Left, &edits);
            *end = transform_point(*end, text::Bias::Right, &edits);
            let rows = (start.row, end.row.max(start.row));
            if rows != (range.start_row, range.end_row) {
                (range.start_row, range.end_row) = rows;
                rows_changed = true;
            }
        }
        if rows_changed {
            self.normalize();
        }
        let mut hidden_changed = false;
        if !self.hidden_rows.is_empty() {
            // Each edit replaces old rows `a ..= b` (shifted by the edits
            // before it) with new rows `a ..= c`.
            let mut delta: i64 = 0;
            for edit in &edits {
                let a = edit.new_start.row;
                let b = (edit.old_end.row as i64 + delta) as u32;
                let c = edit.new_end.row;
                if b != a || c != a {
                    hidden_changed |= apply_row_edit(&mut self.hidden_rows, a, b, c);
                }
                delta = c as i64 - edit.old_end.row as i64;
            }
        }
        if hidden_changed {
            self.hidden_line_changes = true;
        }
        if rows_changed || hidden_changed {
            self.generation = next_generation();
        }
        rows_changed || hidden_changed
    }

    /// Moves the positions on `row` through `edits`, which stay within
    /// their rows: those of the regions starting on it, and the ends of the
    /// regions enclosing it that end on it.
    fn move_within_row(&mut self, row: u32, edits: &[PointEdit]) {
        let first = self.ranges.partition_point(|range| range.start_row < row);
        let last = self.ranges.partition_point(|range| range.start_row <= row);
        for (start, _) in &mut self.positions[first..last] {
            *start = transform_point(*start, text::Bias::Left, edits);
        }
        // The regions containing `row` are the innermost one starting
        // above it and its ancestors.
        let mut ix = first.checked_sub(1);
        while let Some(i) = ix {
            if self.ranges[i].end_row == row {
                let end = &mut self.positions[i].1;
                *end = transform_point(*end, text::Bias::Right, edits);
            }
            ix = self.ranges[i].parent;
        }
    }

    /// Forgets every region (Monaco drops the folding decorations when the
    /// whole text is replaced).
    pub fn clear(&mut self, buffer: &text::Buffer) {
        self.ranges.clear();
        self.collapsed.clear();
        self.positions.clear();
        self.hidden_rows.clear();
        self.hidden_line_changes = false;
        self.tracked = Some((buffer.version(), buffer.rope().clone()));
        self.generation = next_generation();
    }

    /// Drops the regions edits emptied or made start on the same row as
    /// the one before, or cross their parent, and recomputes the parents.
    fn normalize(&mut self) {
        let mut write = 0;
        let mut stack: Vec<usize> = Vec::new();
        let mut previous: Option<u32> = None;
        for read in 0..self.ranges.len() {
            let (start, end) = (self.ranges[read].start_row, self.ranges[read].end_row);
            while let Some(&top) = stack.last()
                && self.ranges[top].end_row < start
            {
                stack.pop();
            }
            let valid = end > start
                && previous.is_none_or(|previous| start > previous)
                && stack
                    .last()
                    .is_none_or(|&top| self.ranges[top].end_row >= end);
            if !valid {
                continue;
            }
            previous = Some(start);
            self.ranges.swap(write, read);
            self.collapsed.swap(write, read);
            self.positions.swap(write, read);
            self.ranges[write].parent = stack.last().copied();
            stack.push(write);
            write += 1;
        }
        self.ranges.truncate(write);
        self.collapsed.truncate(write);
        self.positions.truncate(write);
    }

    fn merge_range(&self, ix: usize) -> MergeRange {
        let range = &self.ranges[ix];
        MergeRange {
            start: range.start_row,
            end: range.end_row,
            kind: range.kind.clone(),
            collapsed: self.collapsed[ix],
            source: range.source(),
        }
    }

    /// Monaco `FoldingModel._currentFoldedOrManualRanges`: the collapsed
    /// regions (expanded if a selection starts in their hidden rows) and
    /// those not from the provider.
    fn folded_or_manual(&self, selection: Option<&[u32]>) -> Vec<MergeRange> {
        (0..self.ranges.len())
            .filter(|&ix| self.collapsed[ix] || !self.ranges[ix].is_provided())
            .map(|ix| {
                let mut range = self.merge_range(ix);
                if range.collapsed
                    && selection.is_some_and(|rows| {
                        rows.iter()
                            .any(|&row| range.start < row && row <= range.end)
                    })
                {
                    range.collapsed = false;
                }
                range
            })
            .collect()
    }

    /// Monaco `FoldingModel.updatePost`: makes `ranges` the regions, with
    /// positions on `rope`, and derives the hidden areas again. Returns
    /// whether those changed.
    fn update_post(&mut self, ranges: Vec<MergeRange>, rope: &Rope) -> bool {
        self.ranges.clear();
        self.collapsed.clear();
        self.positions.clear();
        let mut stack: Vec<usize> = Vec::new();
        for range in ranges {
            while let Some(&top) = stack.last()
                && self.ranges[top].end_row < range.start
            {
                stack.pop();
            }
            self.positions.push((
                text::Point::new(range.start, rope.line_len(range.start)),
                text::Point::new(range.end, rope.line_len(range.end)),
            ));
            self.collapsed.push(range.collapsed);
            self.ranges.push(FoldRange {
                start_row: range.start,
                end_row: range.end,
                parent: stack.last().copied(),
                kind: range.kind,
                manual: range.source == FoldSource::UserDefined,
                recovered: range.source == FoldSource::Recovered,
            });
            stack.push(self.ranges.len() - 1);
        }
        self.generation = next_generation();
        self.derive_hidden()
    }

    /// Monaco `FoldingModel.update`: merges newly computed `ranges`, for
    /// text `version` (`rope`), into the regions, keeping collapsed ones
    /// collapsed unless a selection starting row in `selection` makes them
    /// expand. Ranges for an older version are moved through the edits made
    /// since. Returns whether the hidden rows changed.
    pub fn set_ranges(
        &mut self,
        ranges: Vec<FoldRange>,
        version: text::Version,
        rope: &Rope,
        buffer: &text::Buffer,
        selection: &[u32],
    ) -> bool {
        self.sync(buffer);
        self.ranges_version = Some(version);
        let edits = if version == buffer.version() {
            Vec::new()
        } else {
            point_edits(rope, buffer.rope(), &buffer.edits_since(version))
        };
        let computed: Vec<MergeRange> = ranges
            .into_iter()
            .filter_map(|range| {
                let (mut start, mut end) = (range.start_row, range.end_row);
                if !edits.is_empty() {
                    let at_end = |row: u32| text::Point::new(row, rope.line_len(row));
                    start = transform_point(at_end(start), text::Bias::Left, &edits).row;
                    end = transform_point(at_end(end), text::Bias::Right, &edits).row;
                }
                (end > start).then_some(MergeRange {
                    start,
                    end,
                    kind: range.kind,
                    collapsed: false,
                    source: FoldSource::Provider,
                })
            })
            .collect();
        let folded = self.folded_or_manual(Some(selection));
        let line_count = buffer.rope().line_count();
        let merged = sanitize_and_merge(&computed, &folded, line_count, Some(selection));
        self.update_post(merged, buffer.rope())
    }

    /// Monaco `HiddenRangeModel.updateHiddenRanges`: the hidden areas from
    /// the collapsed regions, those inside another left out. Returns
    /// whether they changed, or rows changed since they were last derived.
    fn derive_hidden(&mut self) -> bool {
        let mut areas: Vec<(u32, u32)> = Vec::new();
        let mut last: Option<(u32, u32)> = None;
        for (range, _) in self
            .ranges
            .iter()
            .zip(&self.collapsed)
            .filter(|(_, collapsed)| **collapsed)
        {
            let (start, end) = (range.start_row + 1, range.end_row);
            if last.is_some_and(|(last_start, last_end)| last_start <= start && end <= last_end) {
                continue;
            }
            areas.push((start, end));
            last = Some((start, end));
        }
        let unchanged = areas.len() == self.hidden_rows.len()
            && areas
                .iter()
                .zip(&self.hidden_rows)
                .all(|((start, end), rows)| *start == rows.start && end + 1 == rows.end);
        let line_changes = std::mem::take(&mut self.hidden_line_changes);
        if unchanged {
            return line_changes;
        }
        self.hidden_rows = areas
            .into_iter()
            .map(|(start, end)| start..end + 1)
            .collect();
        true
    }

    /// Adds collapsed manual ranges over rows `start ..= end` (Monaco
    /// "Create Folding Range from Selection"). Returns whether the hidden
    /// rows changed.
    pub fn add_manual(&mut self, buffer: &text::Buffer, rows: &[(u32, u32)]) -> bool {
        self.sync(buffer);
        let mut added: Vec<MergeRange> = rows
            .iter()
            .filter(|(start, end)| end > start)
            .map(|&(start, end)| MergeRange {
                start,
                end,
                kind: None,
                collapsed: true,
                source: FoldSource::UserDefined,
            })
            .collect();
        if added.is_empty() {
            return false;
        }
        added.sort_by_key(|range| range.start);
        let current: Vec<MergeRange> = (0..self.ranges.len())
            .map(|ix| self.merge_range(ix))
            .collect();
        let merged = sanitize_and_merge(&current, &added, buffer.rope().line_count(), None);
        self.update_post(merged, buffer.rope())
    }

    /// Removes manual ranges (Monaco "Remove Manual Folding Ranges"), with
    /// their collapsed state. Returns whether the hidden rows changed.
    pub fn remove_manual(
        &mut self,
        buffer: &text::Buffer,
        selections: &[crate::view::fold_model::SelectedRows],
    ) -> bool {
        self.sync(buffer);
        let model =
            crate::view::fold_model::FoldingModel::new(&self.ranges, self.collapsed.clone());
        let removed = crate::view::fold_model::manual_ranges_to_remove(&model, selections);
        if removed.is_empty() {
            return false;
        }
        let kept: Vec<MergeRange> = (0..self.ranges.len())
            .filter(|ix| !removed.contains(ix))
            .map(|ix| self.merge_range(ix))
            .collect();
        self.update_post(kept, buffer.rope())
    }

    /// Monaco `FoldingModel.getMemento`: the collapsed regions and those
    /// not from the provider.
    pub fn memento(&self) -> Vec<MergeRange> {
        self.folded_or_manual(None)
    }

    /// Monaco `FoldingModel.applyMemento`: merges saved regions (sorted by
    /// start) into the current ones. Returns whether the hidden rows
    /// changed.
    pub fn apply_memento(&mut self, buffer: &text::Buffer, mut saved: Vec<MergeRange>) -> bool {
        self.sync(buffer);
        saved.sort_by_key(|range| range.start);
        let current: Vec<MergeRange> = (0..self.ranges.len())
            .map(|ix| self.merge_range(ix))
            .collect();
        let merged = sanitize_and_merge(&current, &saved, buffer.rope().line_count(), None);
        self.update_post(merged, buffer.rope())
    }

    /// The manual ranges as inclusive model row ranges `(start, end)`.
    pub fn manual_rows(&self) -> Vec<(u32, u32)> {
        self.ranges
            .iter()
            .filter(|range| range.manual)
            .map(|range| (range.start_row, range.end_row))
            .collect()
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
    pub fn collapsed_flags(&self) -> Vec<bool> {
        self.collapsed.clone()
    }

    /// Toggles the ranges at `indexes` (Monaco
    /// `FoldingModel.toggleCollapseState`): each once, however often listed.
    /// Returns whether the hidden rows changed.
    pub fn toggle(&mut self, mut indexes: Vec<usize>) -> bool {
        indexes.sort_unstable();
        indexes.dedup();
        for ix in indexes {
            if let Some(collapsed) = self.collapsed.get_mut(ix) {
                *collapsed = !*collapsed;
            }
        }
        self.generation = next_generation();
        self.derive_hidden()
    }

    /// Monaco `FoldingController.revealCursor`: expands the collapsed
    /// regions around each of `rows` that's hidden. Returns whether any
    /// was.
    pub fn reveal_rows(&mut self, rows: &[u32]) -> bool {
        let model =
            crate::view::fold_model::FoldingModel::new(&self.ranges, self.collapsed.clone());
        let mut toggle = Vec::new();
        for &row in rows.iter().filter(|&&row| self.is_hidden(row)) {
            toggle.extend(model.all_regions_at_line(row, |ix, _| {
                model.collapsed[ix] && row > model.ranges[ix].start_row
            }));
        }
        if toggle.is_empty() {
            return false;
        }
        self.toggle(toggle);
        true
    }

    /// Collapsed regions as inclusive model row ranges `(start, end)`,
    /// nested ones included.
    pub fn collapsed_rows(&self) -> Vec<(u32, u32)> {
        self.ranges
            .iter()
            .zip(&self.collapsed)
            .filter(|(_, collapsed)| **collapsed)
            .map(|(range, _)| (range.start_row, range.end_row))
            .collect()
    }

    /// Model rows hidden by collapsed regions, sorted.
    pub fn hidden_rows(&self) -> &[std::ops::Range<u32>] {
        &self.hidden_rows
    }

    /// Whether model `row` is hidden (Monaco `HiddenRangeModel.isHidden`).
    pub fn is_hidden(&self, row: u32) -> bool {
        let ix = self.hidden_rows.partition_point(|rows| rows.start <= row);
        ix > 0 && self.hidden_rows[ix - 1].contains(&row)
    }

    /// Whether any row is hidden (Monaco `HiddenRangeModel.hasRanges`).
    pub fn has_hidden(&self) -> bool {
        !self.hidden_rows.is_empty()
    }

    /// True if the region starting at model row `start_row` is collapsed.
    pub fn is_collapsed(&self, start_row: u32) -> bool {
        let ix = self
            .ranges
            .partition_point(|range| range.start_row < start_row);
        self.ranges
            .get(ix)
            .is_some_and(|range| range.start_row == start_row && self.collapsed[ix])
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

    /// The fold range whose start row is `row`, if any.
    pub fn range_starting_at(&self, row: u32) -> Option<&FoldRange> {
        let ix = self.ranges.partition_point(|range| range.start_row < row);
        self.ranges.get(ix).filter(|range| range.start_row == row)
    }

    /// Expands every collapsed region. Returns whether the hidden rows
    /// changed.
    pub fn expand_all(&mut self) -> bool {
        let collapsed: Vec<usize> = (0..self.ranges.len())
            .filter(|&ix| self.collapsed[ix])
            .collect();
        self.toggle(collapsed)
    }

    /// True if any region is collapsed.
    pub fn has_collapsed(&self) -> bool {
        self.collapsed.contains(&true)
    }
}

/// Monaco's view model lines through an edit replacing rows `a ..= b` with
/// rows `a ..= c` (`TextModel._doApplyEdits`): the first `min(b, c) - a`
/// rows after `a` are edited in place and keep their visibility, extra old
/// rows go and later ones shift, and extra new rows are hidden if the row
/// they're inserted after is and isn't the first
/// (`ViewModelLinesFromProjectedModel.onModelLinesInserted`). Returns
/// whether any hidden row changed.
fn apply_row_edit(hidden: &mut Vec<std::ops::Range<u32>>, a: u32, b: u32, c: u32) -> bool {
    let a = b.min(c).max(a);
    let inserted_hidden = a > 0 && hidden.iter().any(|rows| rows.contains(&a));
    let shift = |row: u32| (row as i64 + c as i64 - b as i64) as u32;
    let mut result: Vec<std::ops::Range<u32>> = Vec::with_capacity(hidden.len() + 1);
    let mut push = |rows: std::ops::Range<u32>| {
        if rows.is_empty() {
            return;
        }
        match result.last_mut() {
            Some(last) if last.end >= rows.start => last.end = last.end.max(rows.end),
            _ => result.push(rows),
        }
    };
    for rows in hidden.iter() {
        // Up to row `a`, then the inserted rows, then what's after `b`.
        push(rows.start..rows.end.min(a + 1));
        if rows.contains(&a) && inserted_hidden {
            push(a + 1..c + 1);
        }
        if rows.end > b + 1 {
            push(shift(rows.start.max(b + 1))..shift(rows.end));
        }
    }
    let changed = result != *hidden;
    *hidden = result;
    changed
}

fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn point_edits(old: &Rope, new: &Rope, patch: &text::Patch) -> Vec<PointEdit> {
    patch
        .edits()
        .iter()
        .map(|edit| PointEdit {
            old_start: old.offset_to_point(edit.old.start),
            old_end: old.offset_to_point(edit.old.end),
            new_start: new.offset_to_point(edit.new.start),
            new_end: new.offset_to_point(edit.new.end),
        })
        .collect()
}

/// The folding controls' icons (Monaco `foldingDecorations.ts`'s
/// `registerIcon`s): manual ranges ("Create Folding Range from Selection")
/// get their own, which default to the others' codicons, as Monaco's
/// `foldingManualCollapsedIcon` and `foldingManualExpandedIcon` do. A host
/// can change any of them with [`set_folding_icon`], as a product icon
/// theme does in VS Code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FoldingIcon {
    /// `folding-expanded` (Monaco `foldingExpandedIcon`).
    Expanded,
    /// `folding-collapsed` (Monaco `foldingCollapsedIcon`).
    Collapsed,
    /// `folding-manual-expanded` (Monaco `foldingManualExpandedIcon`).
    ManualExpanded,
    /// `folding-manual-collapsed` (Monaco `foldingManualCollapsedIcon`).
    ManualCollapsed,
}

impl FoldingIcon {
    /// The icon of a region's control (Monaco
    /// `FoldingDecorationProvider.getDecorationOption`).
    pub fn for_region(collapsed: bool, manual: bool) -> Self {
        match (collapsed, manual) {
            (false, false) => Self::Expanded,
            (true, false) => Self::Collapsed,
            (false, true) => Self::ManualExpanded,
            (true, true) => Self::ManualCollapsed,
        }
    }

    /// Monaco's icon id (the `codicon-<id>` class name).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Expanded => "folding-expanded",
            Self::Collapsed => "folding-collapsed",
            Self::ManualExpanded => "folding-manual-expanded",
            Self::ManualCollapsed => "folding-manual-collapsed",
        }
    }

    /// The codicon Monaco registers it with: `chevron-down` when expanded
    /// and `chevron-right` when collapsed, manual or not.
    pub const fn default_codicon(self) -> crate::codicons::Codicon {
        match self {
            Self::Expanded | Self::ManualExpanded => crate::codicons::Codicon::ChevronDown,
            Self::Collapsed | Self::ManualCollapsed => crate::codicons::Codicon::ChevronRight,
        }
    }

    /// The codicon drawn: the one [`set_folding_icon`] gave, else the
    /// default.
    pub fn codicon(self, cx: &gpui::App) -> crate::codicons::Codicon {
        cx.try_global::<FoldingIcons>()
            .and_then(|icons| icons.0[self as usize])
            .unwrap_or(self.default_codicon())
    }
}

/// The codicons [`set_folding_icon`] gave, by [`FoldingIcon`].
#[derive(Default)]
struct FoldingIcons([Option<crate::codicons::Codicon>; 4]);

impl gpui::Global for FoldingIcons {}

/// Draws `icon` as `codicon` in every editor (`None` restores Monaco's
/// default), as a product icon theme's `folding-*` icons do in VS Code.
pub fn set_folding_icon(
    icon: FoldingIcon,
    codicon: Option<crate::codicons::Codicon>,
    cx: &mut gpui::App,
) {
    let icons = cx.default_global::<FoldingIcons>();
    if icons.0[icon as usize] != codicon {
        icons.0[icon as usize] = codicon;
        cx.refresh_windows();
    }
}
