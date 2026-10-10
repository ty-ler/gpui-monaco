//! Indent guides and whitespace rendering.
//!
//! Ports `getLinesIndentGuides`, `getActiveIndentGuide` and
//! `_getIndentLevelForWhitespaceLine` from Monaco's
//! `src/vs/editor/common/model/guidesTextModelPart.ts`, and the whitespace
//! selection of `_applyRenderWhitespace` in
//! `common/viewLayout/viewLineRenderer.ts` (MIT). Rows are 0-based.

use std::ops::Range;

use text::{PointUtf16, Rope};

use crate::options::RenderWhitespace;
use crate::view::folding::indent_level;

/// The indentation context for guides.
#[derive(Clone, Copy, Debug)]
pub struct GuideContext {
    /// Visible columns per tab.
    pub tab_size: u32,
    /// Visible columns per indent level.
    pub indent_size: u32,
    /// The language's `folding.offSide` (blank lines belong to the next
    /// block, as in Python).
    pub off_side: bool,
}

/// Monaco `computeIndentLevel` for `row`, `None` for a blank line.
fn indent_of(rope: &Rope, row: u32, ctx: GuideContext) -> Option<u32> {
    indent_level(&rope.line(row), ctx.tab_size)
}

/// Monaco `_getIndentLevelForWhitespaceLine`.
fn whitespace_line_level(ctx: GuideContext, above: Option<u32>, below: Option<u32>) -> u32 {
    let indent_size = ctx.indent_size.max(1);
    match (above, below) {
        (Some(above), Some(below)) if above < below => 1 + above / indent_size,
        (Some(above), Some(below)) if above == below => below.div_ceil(indent_size),
        (Some(_), Some(below)) => {
            if ctx.off_side {
                below.div_ceil(indent_size)
            } else {
                1 + below / indent_size
            }
        }
        // At the top or bottom of the file.
        _ => 0,
    }
}

fn content_above(rope: &Rope, row: u32, ctx: GuideContext) -> Option<u32> {
    (0..row).rev().find_map(|r| indent_of(rope, r, ctx))
}

fn content_below(rope: &Rope, row: u32, ctx: GuideContext) -> Option<u32> {
    (row + 1..rope.line_count()).find_map(|r| indent_of(rope, r, ctx))
}

/// The number of indent guides on each of `rows` (Monaco
/// `getLinesIndentGuides`). Blank lines get the level of the block they're in.
pub fn lines_indent_guides(rope: &Rope, rows: Range<u32>, ctx: GuideContext) -> Vec<u32> {
    let indent_size = ctx.indent_size.max(1);
    let mut above: Option<Option<u32>> = None;
    let mut below: Option<(u32, Option<u32>)> = None;
    rows.map(|row| {
        if let Some(indent) = indent_of(rope, row, ctx) {
            above = Some(Some(indent));
            return indent.div_ceil(indent_size);
        }
        let above = *above.get_or_insert_with(|| content_above(rope, row, ctx));
        let below_indent = match below {
            Some((below_row, indent)) if below_row > row => indent,
            _ => {
                let found = (row + 1..rope.line_count())
                    .find_map(|r| indent_of(rope, r, ctx).map(|indent| (r, indent)));
                below = Some(found.map_or((u32::MAX, None), |(r, i)| (r, Some(i))));
                found.map(|(_, i)| i)
            }
        };
        whitespace_line_level(ctx, above, below_indent)
    })
    .collect()
}

/// The guide to highlight for a cursor on a row (Monaco
/// `IActiveIndentGuideInfo`): `indent` levels from `start_row` to `end_row`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveIndentGuide {
    /// First row of the active block.
    pub start_row: u32,
    /// Last row of the active block.
    pub end_row: u32,
    /// The active guide's level (1-based; 0 for none).
    pub indent: u32,
}

/// Monaco `getActiveIndentGuide`: the indentation block around `row`,
/// searched within `min_row..=max_row` (the visible rows).
pub fn active_indent_guide(
    rope: &Rope,
    row: u32,
    min_row: u32,
    max_row: u32,
    ctx: GuideContext,
) -> ActiveIndentGuide {
    let indent_size = ctx.indent_size.max(1);
    let line_count = rope.line_count();
    let level = |row: u32| -> u32 {
        match indent_of(rope, row, ctx) {
            Some(indent) => indent.div_ceil(indent_size),
            None => whitespace_line_level(
                ctx,
                content_above(rope, row, ctx),
                content_below(rope, row, ctx),
            ),
        }
    };
    let (mut start, mut end, mut indent) = (row, row, 0);
    let (mut go_up, mut go_down) = (true, true);
    let initial = level(row);
    let mut distance = 1u32;
    while go_up || go_down {
        let up = row.checked_sub(distance);
        let down = row + distance;
        if distance > 1 && up.is_none_or(|up| up < min_row) {
            go_up = false;
        }
        if distance > 1 && (down >= line_count || down > max_row) {
            go_down = false;
        }
        if distance > 50_000 {
            break;
        }
        let up_level = up.filter(|_| go_up).map(level);
        let down_level = (go_down && down < line_count).then(|| level(down));
        if distance == 1 {
            if down_level.is_some_and(|l| initial + 1 == l) {
                // The start of a scope: the child scope's guide is active.
                go_up = false;
                start = down;
                end = down;
                indent = down_level.unwrap_or(0);
                distance += 1;
                continue;
            }
            if let (Some(up), Some(l)) = (up, up_level)
                && l >= 1
                && l - 1 == initial
            {
                // The end of a scope.
                go_down = false;
                start = up;
                end = up;
                indent = l;
                distance += 1;
                continue;
            }
            indent = initial;
            if indent == 0 {
                return ActiveIndentGuide {
                    start_row: row,
                    end_row: row,
                    indent: 0,
                };
            }
        }
        if go_up {
            match (up, up_level) {
                (Some(up), Some(l)) if l >= indent => start = up,
                _ => go_up = false,
            }
        }
        if go_down {
            match down_level {
                Some(l) if l >= indent => end = down,
                _ => go_down = false,
            }
        }
        distance += 1;
    }
    ActiveIndentGuide {
        start_row: start,
        end_row: end,
        indent,
    }
}

/// The UTF-16 columns of `line` whose whitespace is drawn (Monaco
/// `_applyRenderWhitespace`). `selections` are the selected column ranges on
/// the line, for [`RenderWhitespace::Selection`].
pub fn whitespace_columns(
    line: &str,
    mode: RenderWhitespace,
    selections: &[Range<u32>],
) -> Vec<u32> {
    if mode == RenderWhitespace::None {
        return Vec::new();
    }
    let units: Vec<u16> = line.encode_utf16().collect();
    let is_space = |c: u16| c == b' ' as u16 || c == b'\t' as u16;
    let first = units.iter().position(|&c| !is_space(c));
    let last = units.iter().rposition(|&c| !is_space(c));
    let (first, last, blank) = match (first, last) {
        (Some(first), Some(last)) => (first, last, false),
        _ => (units.len(), units.len(), true),
    };
    let mut result = Vec::new();
    let mut was_whitespace = false;
    for (i, &c) in units.iter().enumerate() {
        let mut whitespace = if i < first || i > last || c == b'\t' as u16 {
            is_space(c)
        } else if c == b' ' as u16 {
            if mode == RenderWhitespace::Boundary {
                was_whitespace || units.get(i + 1).is_some_and(|&n| is_space(n))
            } else {
                true
            }
        } else {
            false
        };
        if whitespace && mode == RenderWhitespace::Selection {
            whitespace = selections
                .iter()
                .any(|r| r.start <= i as u32 && (i as u32) < r.end);
        }
        if whitespace && mode == RenderWhitespace::Trailing {
            whitespace = blank || i > last;
        }
        if whitespace {
            result.push(i as u32);
        }
        was_whitespace = whitespace;
    }
    result
}

/// A bracket pair for guides, in current coordinates (0-based rows,
/// UTF-16 columns).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidePair {
    /// The opening bracket.
    pub open: Range<PointUtf16>,
    /// The closing bracket.
    pub close: Range<PointUtf16>,
    /// Nesting level, for the color.
    pub level: u32,
    /// Monaco `minVisibleColumnIndentation`.
    pub min_indentation: u32,
}

/// A horizontal guide (Monaco `IndentGuideHorizontalLine`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HorizontalGuide {
    /// Along the top of the line, else the bottom.
    pub top: bool,
    /// Where it ends (1-based UTF-16 column).
    pub end_column: u32,
}

/// A bracket pair guide on a line (Monaco `IndentGuide`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BracketGuide {
    /// 1-based visible column, used when `column` is `None`.
    pub visible_column: i64,
    /// 1-based UTF-16 column it starts at instead.
    pub column: Option<u32>,
    /// Nesting level, for the color.
    pub level: u32,
    /// The pair around the cursor, highlighted.
    pub active: bool,
    /// A horizontal guide instead of a vertical one.
    pub horizontal: Option<HorizontalGuide>,
    /// On a wrapped line, only the parts after this (1-based) column.
    pub for_wrapped_lines_after_column: Option<u32>,
    /// On a wrapped line, only the parts up to this (1-based) column.
    pub for_wrapped_lines_before_or_at_column: Option<u32>,
}

/// Monaco `BracketGuideOptions`.
#[derive(Clone, Copy, Debug)]
pub struct BracketGuideOptions {
    /// Guides for every pair, not just the active one.
    pub include_inactive: bool,
    /// Which pairs get horizontal guides.
    pub horizontal: crate::options::BracketPairGuides,
    /// Mark the active pair's guides active.
    pub highlight_active: bool,
}

/// Monaco `getLinesBracketGuides`: the guides of each of `rows`, sorted by
/// visible column. `pairs` are the pairs intersecting the rows, sorted by
/// opening bracket.
pub fn lines_bracket_guides(
    rope: &Rope,
    rows: Range<u32>,
    pairs: &[GuidePair],
    active_position: Option<PointUtf16>,
    options: BracketGuideOptions,
    tab_size: u32,
) -> Vec<Vec<BracketGuide>> {
    use crate::options::BracketPairGuides;
    let mut result: Vec<Vec<BracketGuide>> = rows.clone().map(|_| Vec::new()).collect();
    if rows.is_empty() {
        return result;
    }
    let (start_row, end_row) = (rows.start, rows.end - 1);
    // Monaco: the innermost pair strictly containing the cursor.
    let active = active_position.and_then(|position| {
        pairs
            .iter()
            .rposition(|pair| pair.open.start < position && position < pair.close.end)
    });
    let visible = |point: PointUtf16| -> i64 {
        crate::cursor::strings::visible_column_from_column(
            &rope.line(point.row),
            point.column,
            tab_size,
        ) as i64
            + 1
    };
    let mut push = |row: u32, guide: BracketGuide| {
        result[(row - start_row) as usize].push(guide);
    };
    for (ix, pair) in pairs.iter().enumerate() {
        let is_active = active == Some(ix);
        if !is_active && !options.include_inactive {
            continue;
        }
        let active = options.highlight_active && is_active;
        let start = pair.open.start;
        let end = pair.close.start;
        let horizontal_guides = options.horizontal == BracketPairGuides::On
            || (options.horizontal == BracketPairGuides::Active && is_active);
        let guide = |visible_column: i64| BracketGuide {
            visible_column,
            column: None,
            level: pair.level,
            active,
            horizontal: None,
            for_wrapped_lines_after_column: None,
            for_wrapped_lines_before_or_at_column: None,
        };
        if pair.open.start.row == pair.close.end.row {
            if horizontal_guides {
                push(
                    start.row,
                    BracketGuide {
                        column: Some(pair.open.end.column + 1),
                        horizontal: Some(HorizontalGuide {
                            top: false,
                            end_column: end.column + 1,
                        }),
                        ..guide(-1)
                    },
                );
            }
            continue;
        }
        let end_visible = visible(end);
        let start_visible = visible(start);
        let guide_visible = start_visible
            .min(end_visible)
            .min(pair.min_indentation as i64 + 1);
        // Text before the closing bracket: the guide runs through its line
        // and turns along the bottom.
        let close_line = rope.line(end.row);
        let first_non_ws = close_line
            .encode_utf16()
            .position(|c| c != b' ' as u16 && c != b'\t' as u16)
            .unwrap_or(close_line.encode_utf16().count()) as u32;
        let bottom = first_non_ws < end.column;
        let first = start.row.max(start_row);
        let last = end.row.min(end_row) + u32::from(bottom);
        for row in first..last {
            if row > end_row {
                break;
            }
            push(
                row,
                BracketGuide {
                    for_wrapped_lines_after_column: (row == start.row).then_some(start.column + 1),
                    for_wrapped_lines_before_or_at_column: (row == end.row)
                        .then_some(end.column + 1),
                    ..guide(guide_visible)
                },
            );
        }
        if horizontal_guides {
            if start.row >= start_row && start_visible > guide_visible {
                push(
                    start.row,
                    BracketGuide {
                        horizontal: Some(HorizontalGuide {
                            top: false,
                            end_column: start.column + 1,
                        }),
                        ..guide(guide_visible)
                    },
                );
            }
            if end.row <= end_row && end_visible > guide_visible {
                push(
                    end.row,
                    BracketGuide {
                        horizontal: Some(HorizontalGuide {
                            top: !bottom,
                            end_column: end.column + 1,
                        }),
                        ..guide(guide_visible)
                    },
                );
            }
        }
    }
    for guides in &mut result {
        guides.sort_by_key(|g| g.visible_column);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guide_pairs(text: &str) -> Vec<GuidePair> {
        let rope = Rope::from(text);
        let pairs: Vec<(String, String)> = [("(", ")"), ("{", "}")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let (_, found) = crate::view::bracket_pairs::bracket_pairs(&rope, &pairs, &[], 4);
        found
            .into_iter()
            .map(|p| GuidePair {
                open: rope.offset_to_point_utf16(p.open.start)
                    ..rope.offset_to_point_utf16(p.open.end),
                close: rope.offset_to_point_utf16(p.close.start)
                    ..rope.offset_to_point_utf16(p.close.end),
                level: p.level,
                min_indentation: p.min_indentation,
            })
            .collect()
    }

    /// (visible column, column, level, active, horizontal) per guide.
    type Summary = Vec<Vec<(i64, Option<u32>, u32, bool, Option<(bool, u32)>)>>;

    fn summarize(guides: Vec<Vec<BracketGuide>>) -> Summary {
        guides
            .into_iter()
            .map(|line| {
                line.into_iter()
                    .map(|g| {
                        (
                            g.visible_column,
                            g.column,
                            g.level,
                            g.active,
                            g.horizontal.map(|h| (h.top, h.end_column)),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn bracket_guides_follow_monaco() {
        let text = "fn f() {\n    if x {\n        y\n    }\n}";
        let rope = Rope::from(text);
        let pairs = guide_pairs(text);
        let options = BracketGuideOptions {
            include_inactive: true,
            horizontal: crate::options::BracketPairGuides::On,
            highlight_active: true,
        };
        let guides =
            lines_bracket_guides(&rope, 0..5, &pairs, Some(PointUtf16::new(2, 8)), options, 4);
        assert_eq!(
            summarize(guides.clone()),
            vec![
                vec![
                    // `()` on one line: a horizontal guide under it.
                    (-1, Some(6), 0, false, Some((false, 6))),
                    // The outer block: at its least indentation, with a
                    // horizontal line to its opening bracket.
                    (1, None, 0, false, None),
                    (1, None, 0, false, Some((false, 8))),
                ],
                vec![
                    (1, None, 0, false, None),
                    // The inner block (active: the cursor is inside).
                    (5, None, 1, true, None),
                    (5, None, 1, true, Some((false, 10))),
                ],
                vec![(1, None, 0, false, None), (5, None, 1, true, None)],
                vec![(1, None, 0, false, None)],
                vec![],
            ]
        );
        // The opening line's vertical guide only shows on wrapped parts
        // after the bracket.
        assert_eq!(guides[0][1].for_wrapped_lines_after_column, Some(8));

        // Only the active pair, with its horizontal guides.
        let active_only = lines_bracket_guides(
            &rope,
            0..5,
            &pairs,
            Some(PointUtf16::new(2, 8)),
            BracketGuideOptions {
                include_inactive: false,
                horizontal: crate::options::BracketPairGuides::Active,
                highlight_active: true,
            },
            4,
        );
        assert_eq!(
            summarize(active_only),
            vec![
                vec![],
                vec![
                    (5, None, 1, true, None),
                    (5, None, 1, true, Some((false, 10)))
                ],
                vec![(5, None, 1, true, None)],
                vec![],
                vec![],
            ]
        );
    }

    #[test]
    fn text_before_the_closing_bracket_turns_at_the_bottom() {
        // `x }`: the guide runs through the closing line and turns along
        // its bottom.
        let text = "{\n  a\n  x }";
        let rope = Rope::from(text);
        let guides = lines_bracket_guides(
            &rope,
            0..3,
            &guide_pairs(text),
            None,
            BracketGuideOptions {
                include_inactive: true,
                horizontal: crate::options::BracketPairGuides::On,
                highlight_active: true,
            },
            4,
        );
        assert_eq!(
            summarize(guides),
            vec![
                vec![(1, None, 0, false, None)],
                vec![(1, None, 0, false, None)],
                vec![
                    (1, None, 0, false, None),
                    (1, None, 0, false, Some((false, 5)))
                ],
            ]
        );
    }

    const CTX: GuideContext = GuideContext {
        tab_size: 4,
        indent_size: 4,
        off_side: false,
    };

    fn rope(lines: &[&str]) -> Rope {
        Rope::from(lines.join("\n").as_str())
    }

    #[test]
    fn indent_guides_with_blank_lines() {
        // Monaco `textModel.test.ts` "getLinesIndentGuides" style cases.
        let r = rope(&[
            "fn a() {",
            "    if x {",
            "",
            "        y();",
            "    }",
            "",
            "}",
        ]);
        assert_eq!(
            lines_indent_guides(&r, 0..7, CTX),
            vec![0, 1, 2, 2, 1, 1, 0]
        );
        // Top and bottom blank lines have no guides.
        let r = rope(&["", "    a", ""]);
        assert_eq!(lines_indent_guides(&r, 0..3, CTX), vec![0, 1, 0]);
        // Off-side languages: a blank line belongs to the block below.
        let off_side = GuideContext {
            off_side: true,
            ..CTX
        };
        let r = rope(&["def a():", "        x", "", "    y"]);
        assert_eq!(lines_indent_guides(&r, 0..4, off_side), vec![0, 2, 1, 1]);
        assert_eq!(lines_indent_guides(&r, 0..4, CTX), vec![0, 2, 2, 1]);
    }

    #[test]
    fn active_guide() {
        let r = rope(&[
            "fn a() {",
            "    if x {",
            "        y();",
            "        z();",
            "    }",
            "}",
        ]);
        // Inside the `if`: its body's guide.
        assert_eq!(
            active_indent_guide(&r, 2, 0, 5, CTX),
            ActiveIndentGuide {
                start_row: 2,
                end_row: 3,
                indent: 2
            }
        );
        // On the `if` line, the scope it starts is active.
        assert_eq!(
            active_indent_guide(&r, 1, 0, 5, CTX),
            ActiveIndentGuide {
                start_row: 2,
                end_row: 3,
                indent: 2
            }
        );
        // At top level: none.
        assert_eq!(active_indent_guide(&r, 5, 0, 5, CTX).indent, 1);
        let r = rope(&["a", "b"]);
        assert_eq!(active_indent_guide(&r, 0, 0, 1, CTX).indent, 0);
    }

    #[test]
    fn whitespace_modes() {
        let line = "  a b  c\t ";
        assert_eq!(
            whitespace_columns(line, RenderWhitespace::All, &[]),
            vec![0, 1, 3, 5, 6, 8, 9]
        );
        // Single spaces between words aren't boundary whitespace.
        assert_eq!(
            whitespace_columns(line, RenderWhitespace::Boundary, &[]),
            vec![0, 1, 5, 6, 8, 9]
        );
        assert_eq!(
            whitespace_columns(line, RenderWhitespace::Trailing, &[]),
            vec![8, 9]
        );
        assert_eq!(
            whitespace_columns(line, RenderWhitespace::Selection, &[2..4, 5..6]),
            vec![3, 5]
        );
        assert!(whitespace_columns(line, RenderWhitespace::None, &[]).is_empty());
        // A whitespace-only line is all trailing.
        assert_eq!(
            whitespace_columns("   ", RenderWhitespace::Trailing, &[]),
            vec![0, 1, 2]
        );
    }
}
