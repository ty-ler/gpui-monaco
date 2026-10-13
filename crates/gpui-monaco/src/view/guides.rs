//! Indent guides and whitespace rendering.
//!
//! Ports `getLinesIndentGuides`, `getActiveIndentGuide` and
//! `_getIndentLevelForWhitespaceLine` from Monaco's
//! `src/vs/editor/common/model/guidesTextModelPart.ts`, and the whitespace
//! selection of `_applyRenderWhitespace` in
//! `browser/viewParts/whitespace/whitespace.ts` (MIT). Rows are 0-based.

use std::ops::Range;

use text::{Point, PointUtf16, Rope};

use crate::options::RenderWhitespace;

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
    // `indent_level(&rope.line(row))`, reading only the leading whitespace
    // (and the char after it), not the whole line.
    if row > rope.max_point().row {
        return None;
    }
    let start = rope.point_to_offset(Point::new(row, 0));
    let mut indent = 0;
    for ch in rope.chars_at(start) {
        match ch {
            ' ' => indent += 1,
            '\t' => indent = indent - indent % ctx.tab_size + ctx.tab_size,
            '\n' => return None,
            _ => return Some(indent),
        }
    }
    None
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

/// The UTF-16 columns of a view line whose whitespace is drawn (Monaco
/// `WhitespaceOverlay._applyRenderWhitespace` in
/// `browser/viewParts/whitespace/whitespace.ts`). `segment` is the text the
/// view line shows, starting at model column `start_column`, after
/// `faux_indent` columns of wrapped-line indentation;
/// `continues_with_wrapped_line` says the model line goes on on the next
/// view line. `selections` are the selected model column ranges on the line,
/// for [`RenderWhitespace::Selection`]. Returns model columns.
///
/// With [`RenderWhitespace::Boundary`], a single space between other
/// characters isn't drawn: Monaco looks only for a space on either side, so
/// the space in `"a\t b"` isn't drawn but the tab is.
pub fn whitespace_columns(
    segment: &str,
    start_column: u32,
    faux_indent: u32,
    continues_with_wrapped_line: bool,
    mode: RenderWhitespace,
    selections: &[Range<u32>],
) -> Vec<u32> {
    let space = b' ' as u16;
    let tab = b'\t' as u16;
    if mode == RenderWhitespace::None
        || (mode == RenderWhitespace::Trailing && continues_with_wrapped_line)
    {
        return Vec::new();
    }
    let faux = faux_indent as usize;
    let content: Vec<u16> = std::iter::repeat_n(space, faux)
        .chain(segment.encode_utf16())
        .collect();
    let model_column = |ix: usize| start_column + (ix - faux) as u32;
    let selected = |ix: usize| {
        let column = model_column(ix);
        selections
            .iter()
            .any(|r| r.start <= column && column < r.end)
    };
    if mode == RenderWhitespace::Selection && !(faux..content.len()).any(selected) {
        return Vec::new();
    }
    let len = content.len();
    let is_whitespace = |c: u16| c == space || c == tab;
    let (first, last, empty_or_whitespace) = match content.iter().position(|&c| !is_whitespace(c)) {
        Some(first) => (
            first,
            content
                .iter()
                .rposition(|&c| !is_whitespace(c))
                .unwrap_or(first),
            false,
        ),
        None => (len, len, true),
    };
    let only_boundary = mode == RenderWhitespace::Boundary;
    let at = |ix: Option<usize>| ix.and_then(|ix| content.get(ix)).copied();
    let mut result = Vec::new();
    for (ix, &c) in content.iter().enumerate().skip(faux) {
        if !is_whitespace(c) {
            continue;
        }
        if mode == RenderWhitespace::Trailing && !empty_or_whitespace && ix <= last {
            continue;
        }
        if only_boundary && ix >= first && ix <= last && c == space {
            let prev = at(ix.checked_sub(1));
            let next = at(Some(ix + 1));
            if prev != Some(space) && next != Some(space) {
                continue;
            }
        }
        if only_boundary && continues_with_wrapped_line && ix == len - 1 {
            let prev = at(ix.checked_sub(1));
            if c == space && prev != Some(space) && prev != Some(tab) {
                continue;
            }
        }
        if mode == RenderWhitespace::Selection && !selected(ix) {
            continue;
        }
        result.push(model_column(ix));
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
/// opening bracket. With the active position outside `rows`,
/// `pairs_at_active` are the pairs intersecting its row (the same way), for
/// Monaco's query of the pairs at the cursor.
pub fn lines_bracket_guides(
    rope: &Rope,
    rows: Range<u32>,
    pairs: &[GuidePair],
    active_position: Option<PointUtf16>,
    pairs_at_active: Option<&[GuidePair]>,
    options: BracketGuideOptions,
    tab_size: u32,
) -> Vec<Vec<BracketGuide>> {
    use crate::options::BracketPairGuides;
    let mut result: Vec<Vec<BracketGuide>> = rows.clone().map(|_| Vec::new()).collect();
    if rows.is_empty() {
        return result;
    }
    let (start_row, end_row) = (rows.start, rows.end - 1);
    // Monaco: the innermost pair strictly containing the cursor, among the
    // pairs in view when the cursor is, else among the pairs at the
    // cursor; it's highlighted only if in view.
    let active = active_position
        .filter(|_| !pairs.is_empty())
        .and_then(|position| {
            let candidates = if rows.contains(&position.row) {
                pairs
            } else {
                pairs_at_active.unwrap_or_default()
            };
            candidates
                .iter()
                .rfind(|pair| pair.open.start < position && position < pair.close.end)
        })
        .map(|pair| (pair.open.clone(), pair.close.clone()));
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
    for pair in pairs {
        let is_active = active
            .as_ref()
            .is_some_and(|(open, close)| *open == pair.open && *close == pair.close);
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
        let pairs: Vec<_> = [("(", ")"), ("{", "}")]
            .iter()
            .map(|(a, b)| crate::view::bracket_pairs::BracketKind::colorized(a, b))
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
        let guides = lines_bracket_guides(
            &rope,
            0..5,
            &pairs,
            Some(PointUtf16::new(2, 8)),
            None,
            options,
            4,
        );
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
            None,
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

    /// Monaco `getLinesBracketGuides` with the cursor out of view: the
    /// active pair is the innermost pair at the cursor, visible or not, and
    /// only a visible one is highlighted.
    #[test]
    fn active_pair_with_the_cursor_out_of_view() {
        let text = "{\n\n  {\n    a\n    b\n    c\n  }\n}";
        let rope = Rope::from(text);
        let all = guide_pairs(text);
        let in_rows = |rows: Range<u32>| -> Vec<GuidePair> {
            all.iter()
                .filter(|p| p.open.start.row < rows.end && p.close.end.row >= rows.start)
                .cloned()
                .collect()
        };
        let options = BracketGuideOptions {
            include_inactive: true,
            horizontal: crate::options::BracketPairGuides::Active,
            highlight_active: true,
        };
        let cursor = PointUtf16::new(5, 4);
        let guides = |rows: Range<u32>| {
            lines_bracket_guides(
                &rope,
                rows.clone(),
                &in_rows(rows),
                Some(cursor),
                Some(&in_rows(5..6)),
                options,
                4,
            )
        };
        // Only the outer block's top in view: the cursor's pair (the inner
        // one) isn't, so nothing is active (not the outer pair, which
        // contains the cursor too).
        let top = guides(0..2);
        assert_eq!(top[0].len(), 1);
        assert!(top.iter().flatten().all(|g| !g.active));
        // With the inner block in view, it's active.
        let shown = guides(0..5);
        assert!(shown[3].iter().any(|g| g.active && g.level == 1));
        assert!(shown.iter().flatten().all(|g| !g.active || g.level == 1));
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
        let ws = |line: &str, mode, selections: &[Range<u32>]| {
            whitespace_columns(line, 0, 0, false, mode, selections)
        };
        let line = "  a b  c\t ";
        assert_eq!(
            ws(line, RenderWhitespace::All, &[]),
            vec![0, 1, 3, 5, 6, 8, 9]
        );
        // Single spaces between words aren't boundary whitespace.
        assert_eq!(
            ws(line, RenderWhitespace::Boundary, &[]),
            vec![0, 1, 5, 6, 8, 9]
        );
        assert_eq!(ws(line, RenderWhitespace::Trailing, &[]), vec![8, 9]);
        assert_eq!(
            ws(line, RenderWhitespace::Selection, &[2..4, 5..6]),
            vec![3, 5]
        );
        assert!(ws(line, RenderWhitespace::Selection, &[]).is_empty());
        assert!(ws(line, RenderWhitespace::None, &[]).is_empty());
        // A whitespace-only line is all trailing.
        assert_eq!(ws("   ", RenderWhitespace::Trailing, &[]), vec![0, 1, 2]);
    }

    /// Monaco `whitespace.ts`: the boundary rule looks for spaces only, so a
    /// tab next to a single space doesn't make it boundary whitespace.
    #[test]
    fn boundary_whitespace_next_to_tabs() {
        let ws =
            |line: &str| whitespace_columns(line, 0, 0, false, RenderWhitespace::Boundary, &[]);
        assert_eq!(ws("a\t b"), vec![1]);
        assert_eq!(ws("a \tb"), vec![2]);
        assert_eq!(ws("a  \tb"), vec![1, 2, 3]);
    }

    /// Ported from Monaco's `viewLineRenderer.test.ts` `renderWhitespace`
    /// cases (`ws-middle`, `ws-all-middle`, `ws-mixed`, `ws-faux-indent`,
    /// `ws-trail-*`), as the columns each draws.
    #[test]
    fn monaco_render_whitespace_cases() {
        let ws = |line: &str, faux: u32, mode| whitespace_columns(line, 0, faux, false, mode, &[]);
        // ws-middle: one space between words isn't boundary whitespace.
        assert_eq!(
            ws("it  it it  it", 0, RenderWhitespace::Boundary),
            vec![2, 3, 9, 10]
        );
        assert_eq!(
            ws(" Hello world!\t", 0, RenderWhitespace::All),
            vec![0, 6, 13]
        );
        // ws-mixed: every leading and trailing space and tab, and the
        // double space inside.
        let mixed = "  \t\t  Hello world! \t  \t   \t    ";
        let expected: Vec<u32> = (0..6).chain(18..31).collect();
        assert_eq!(ws(mixed, 0, RenderWhitespace::Boundary), expected);
        // ws-faux-indent: the wrapped line's indentation isn't drawn; its
        // columns start after it.
        let wrapped = "  Hello world! \t  \t   \t    ";
        let columns = whitespace_columns(wrapped, 6, 2, false, RenderWhitespace::Boundary, &[]);
        assert_eq!(
            columns,
            [0, 1]
                .into_iter()
                .chain(14..27)
                .map(|c| c + 6)
                .collect::<Vec<u32>>()
        );
        // ws-trail-with-trail and ws-trail-only.
        assert_eq!(
            ws(" Hello world! \t", 0, RenderWhitespace::Trailing),
            vec![13, 14]
        );
        assert_eq!(ws(" \t ", 0, RenderWhitespace::Trailing), vec![0, 1, 2]);
        // A segment that continues on the next view line has no trailing
        // whitespace, and its single space at the wrap isn't boundary
        // whitespace.
        assert!(whitespace_columns("ab ", 0, 0, true, RenderWhitespace::Trailing, &[]).is_empty());
        assert!(whitespace_columns("ab ", 0, 0, true, RenderWhitespace::Boundary, &[]).is_empty());
        assert_eq!(
            whitespace_columns("ab  ", 0, 0, true, RenderWhitespace::Boundary, &[]),
            vec![2, 3]
        );
    }
}
