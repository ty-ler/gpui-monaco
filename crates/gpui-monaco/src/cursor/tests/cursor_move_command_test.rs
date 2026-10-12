//! Port of the viewport cases of Monaco's
//! `src/vs/editor/test/browser/controller/cursorMoveCommand.test.ts` (MIT),
//! with `getCompletelyVisibleViewRange` stubbed as there: the row
//! [`viewport_row`] picks and the first non-whitespace column the cursor
//! goes to. `tests/reveal.rs` runs them against a real viewport, and
//! checks that partly visible lines don't count.

use text::Rope;

use crate::cursor::{Position, ViewportTarget, first_non_whitespace_column, viewport_row};

const TEXT: &str = "    \tMy First Line\t \n\tMy Second Line\n    Third Line🐶\n\n1";

/// Where `cursorMove` to `target` (`value` lines in) puts the cursor with
/// Monaco's stubbed visible range `(startLine, startColumn, endLine,
/// endColumn)`, converted to the model as Monaco does (clamped to the
/// text). Monaco 1-based (line, column) in and out.
fn moved(visible: (u32, u32, u32, u32), target: ViewportTarget, value: u32) -> (u32, u32) {
    let rope = Rope::from(TEXT);
    let last = rope.max_point_utf16();
    let clamp = |line: u32, column: u32| {
        if line - 1 > last.row {
            last
        } else {
            Position::new(line - 1, column - 1)
        }
    };
    let range = clamp(visible.0, visible.1)..clamp(visible.2, visible.3);
    let row = viewport_row(range, target, value);
    let column = first_non_whitespace_column(&rope, row).unwrap_or(0);
    (row + 1, column + 1)
}

#[test]
fn move_to_view_top_line_moves_to_first_visible_line_if_it_is_first_line() {
    assert_eq!(moved((1, 1, 10, 1), ViewportTarget::Top, 1), (1, 6));
}

#[test]
fn move_to_view_top_line_moves_to_top_visible_line_when_first_line_is_not_visible() {
    assert_eq!(moved((2, 1, 10, 1), ViewportTarget::Top, 1), (2, 2));
}

#[test]
fn move_to_view_top_line_moves_to_nth_line_from_top() {
    assert_eq!(moved((1, 1, 10, 1), ViewportTarget::Top, 3), (3, 5));
}

#[test]
fn move_to_view_top_line_moves_to_last_line_if_n_is_greater_than_last_visible_line_number() {
    assert_eq!(moved((1, 1, 3, 1), ViewportTarget::Top, 4), (3, 5));
}

#[test]
fn move_to_view_center_line_moves_to_the_center_line() {
    assert_eq!(moved((3, 1, 3, 1), ViewportTarget::Center, 1), (3, 5));
}

#[test]
fn move_to_view_bottom_line_moves_to_last_visible_line_if_it_is_last_line() {
    assert_eq!(moved((1, 1, 5, 1), ViewportTarget::Bottom, 1), (5, 1));
}

#[test]
fn move_to_view_bottom_line_moves_to_last_visible_line_when_last_line_is_not_visible() {
    assert_eq!(moved((2, 1, 3, 1), ViewportTarget::Bottom, 1), (3, 5));
}

#[test]
fn move_to_view_bottom_line_moves_to_nth_line_from_bottom() {
    assert_eq!(moved((1, 1, 5, 1), ViewportTarget::Bottom, 3), (3, 5));
}

#[test]
fn move_to_view_bottom_line_moves_to_first_line_if_n_is_lesser_than_first_visible_line_number() {
    assert_eq!(moved((2, 1, 5, 1), ViewportTarget::Bottom, 5), (2, 2));
}

/// Monaco `_firstLineNumberInRange`: a first line whose start is wrapped
/// off screen doesn't count.
#[test]
fn a_first_line_starting_above_the_viewport_is_skipped() {
    assert_eq!(moved((1, 3, 5, 1), ViewportTarget::Top, 1), (2, 2));
    assert_eq!(moved((1, 3, 2, 1), ViewportTarget::Bottom, 5), (2, 2));
    // The center doesn't skip it.
    assert_eq!(moved((1, 3, 2, 1), ViewportTarget::Center, 1), (2, 2));
    assert_eq!(moved((1, 3, 1, 10), ViewportTarget::Center, 1), (1, 6));
}
