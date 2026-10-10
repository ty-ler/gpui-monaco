//! Indent guides and whitespace rendering.
//!
//! Ports `getLinesIndentGuides`, `getActiveIndentGuide` and
//! `_getIndentLevelForWhitespaceLine` from Monaco's
//! `src/vs/editor/common/model/guidesTextModelPart.ts`, and the whitespace
//! selection of `_applyRenderWhitespace` in
//! `common/viewLayout/viewLineRenderer.ts` (MIT). Rows are 0-based.

use std::ops::Range;

use text::Rope;

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

#[cfg(test)]
mod tests {
    use super::*;

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
