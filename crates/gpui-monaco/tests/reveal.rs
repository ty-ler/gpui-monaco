//! Where Monaco reveals the cursors and where it doesn't (#323): the API
//! setters don't scroll, cursor commands reveal every cursor
//! (`revealAllCursors`) or the one Monaco picks, fold clicks reveal the
//! clicked line, `editorScroll` with `revealCursor` keeps its scroll, and
//! PageUp/PageDown move by Monaco's `pageSize` and reveal minimally.

use gpui::{AppContext as _, Entity, Focusable, Pixels, TestAppContext, VisualTestContext, px};
use gpui_monaco::actions::{
    AddCursorAbove, AddCursorBelow, CursorColumnSelectDown, CursorColumnSelectUp, CursorPageDown,
    CursorPageUp, ScrollPageDown, ScrollPageUp,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, ScrollType, TextModel};
use serde_json::json;

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// An editor without sticky scroll, so reveals keep no context lines
/// (Monaco's default `cursorSurroundingLines: 0`).
fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///a.txt"));
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |e, cx| {
        e.update_options_with(
            |opts| {
                opts.sticky_scroll.enabled = false;
                opts.minimap.enabled = false;
                options(opts);
            },
            cx,
        );
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

/// The line height and the editor's height.
fn metrics(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (Pixels, Pixels) {
    editor.read_with(cx, |e, _| {
        let info = e.layout_info().expect("laid out");
        (e.options().resolved_line_height(), info.height)
    })
}

/// Monaco `pageSize`: the lines the editor holds, less two.
fn page_size(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> u32 {
    let (lh, height) = metrics(editor, cx);
    ((height / lh).floor() as u32).saturating_sub(2).max(1)
}

fn scroll_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.scroll_top())
}

fn set_scroll_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, top: Pixels) {
    editor.update(cx, |e, cx| e.set_scroll_top(top, ScrollType::Immediate, cx));
    draw(cx);
}

fn select(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, selections: &[Selection]) {
    editor.update(cx, |e, cx| e.set_selections(selections, cx));
    draw(cx);
}

/// The first and last rows on screen, partly or fully.
fn visible(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (u32, u32) {
    let ranges = editor.read_with(cx, |e, cx| e.visible_ranges(cx));
    (ranges[0].start.row, ranges.last().unwrap().end.row)
}

/// Whether `row` shows in full.
fn fully_visible(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32) -> bool {
    let (lh, height) = metrics(editor, cx);
    let top = scroll_top(editor, cx);
    let y = lh * row as f32;
    top <= y && y + lh <= top + height
}

fn rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<u32> {
    editor.read_with(cx, |e, _| {
        e.selections().iter().map(|s| s.head.row).collect()
    })
}

fn trigger_with(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    id: &str,
    payload: serde_json::Value,
) {
    editor
        .update_in(cx, |e, window, cx| {
            e.trigger_with_payload(id, payload, window, cx)
        })
        .unwrap_or_else(|err| panic!("{id}: {err}"));
    draw(cx);
}

#[gpui::test]
fn api_setters_dont_scroll(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), |_| {}, cx);
    // Monaco `setPosition`, `setSelection`, `setSelections`: no reveal.
    editor.update(cx, |e, cx| e.set_position(p(300, 0), cx));
    draw(cx);
    assert_eq!(scroll_top(&editor, cx), px(0.));
    editor.update(cx, |e, cx| {
        e.set_selection(
            Selection {
                anchor: p(200, 0),
                head: p(210, 2),
            },
            cx,
        )
    });
    draw(cx);
    assert_eq!(scroll_top(&editor, cx), px(0.));
    select(
        &editor,
        cx,
        &[
            Selection::collapsed(p(100, 0)),
            Selection::collapsed(p(400, 0)),
        ],
    );
    assert_eq!(scroll_top(&editor, cx), px(0.));
    assert_eq!(rows(&editor, cx), [100, 400]);

    // A restored scroll position stays when a cursor is set off screen.
    let (lh, _) = metrics(&editor, cx);
    set_scroll_top(&editor, cx, lh * 50.);
    editor.update(cx, |e, cx| e.set_position(p(450, 0), cx));
    draw(cx);
    assert_eq!(scroll_top(&editor, cx), lh * 50.);

    // The host reveals itself, once: the setter doesn't undo it.
    editor.update(cx, |e, cx| {
        e.set_position(p(300, 0), cx);
        e.reveal_position_in_center(p(300, 0), cx);
    });
    draw(cx);
    let centered = scroll_top(&editor, cx);
    let (first, last) = visible(&editor, cx);
    assert!(first < 300 && 300 < last, "{first}..{last}");
    editor.update(cx, |e, cx| {
        e.reveal_position_in_center(p(300, 0), cx);
        e.set_position(p(300, 0), cx);
    });
    draw(cx);
    assert_eq!(scroll_top(&editor, cx), centered);
}

#[gpui::test]
fn keyboard_moves_reveal_every_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), |_| {}, cx);
    let page = page_size(&editor, cx);
    // The primary cursor near the top, another on the last line that
    // fully shows.
    let bottom = page + 1;
    select(
        &editor,
        cx,
        &[
            Selection::collapsed(p(2, 0)),
            Selection::collapsed(p(bottom, 0)),
        ],
    );
    assert_eq!(scroll_top(&editor, cx), px(0.));
    // Down: Monaco `revealAllCursors` shows the second cursor too.
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(rows(&editor, cx), [3, bottom + 1]);
    assert!(scroll_top(&editor, cx) > px(0.));
    assert!(fully_visible(&editor, cx, 3));
    assert!(fully_visible(&editor, cx, bottom + 1));

    // Cursors further apart than the viewport: no scrolling at all.
    set_scroll_top(&editor, cx, px(0.));
    select(
        &editor,
        cx,
        &[
            Selection::collapsed(p(2, 0)),
            Selection::collapsed(p(300, 0)),
        ],
    );
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(rows(&editor, cx), [3, 301]);
    assert_eq!(scroll_top(&editor, cx), px(0.));
}

#[gpui::test]
fn add_cursor_above_and_below_reveal_the_new_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), |_| {}, cx);
    let page = page_size(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(0, 0))]);
    // Monaco `revealBottomMostCursor`: the cursors leave the viewport, the
    // view follows the newest.
    let count = page + 10;
    for _ in 0..count {
        cx.dispatch_action(AddCursorBelow::default());
    }
    draw(cx);
    assert_eq!(rows(&editor, cx).len(), count as usize + 1);
    assert!(fully_visible(&editor, cx, count));

    // Monaco `revealTopMostCursor`.
    let (lh, _) = metrics(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(300, 0))]);
    set_scroll_top(&editor, cx, lh * 300.);
    for _ in 0..3 {
        cx.dispatch_action(AddCursorAbove::default());
    }
    draw(cx);
    assert!(fully_visible(&editor, cx, 297));
}

#[gpui::test]
fn add_selection_to_next_find_match_reveals_the_match(cx: &mut TestAppContext) {
    let mut text = lines(500);
    text.insert_str(0, "needle ");
    let at = text.find("line 300").unwrap();
    text.insert_str(at, "needle ");
    let (editor, cx) = setup(&text, |_| {}, cx);
    select(&editor, cx, &[Selection::collapsed(p(0, 0))]);
    // The word, then the match on row 300, centered (Monaco
    // `revealRangeInCenterIfOutsideViewport`).
    editor.update(cx, |e, cx| e.add_selection_to_next_find_match(cx));
    draw(cx);
    assert_eq!(scroll_top(&editor, cx), px(0.));
    editor.update(cx, |e, cx| e.add_selection_to_next_find_match(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx), [0, 300]);
    let (first, last) = visible(&editor, cx);
    assert!(first < 300 && 300 < last, "{first}..{last}");
    let middle = (first + last) / 2;
    assert!(middle.abs_diff(300) <= 1, "{first}..{last}");

    // Back to the match on row 0: centered too, as far as the top allows.
    editor.update(cx, |e, cx| e.move_selection_to_previous_find_match(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx), [0]);
    assert_eq!(scroll_top(&editor, cx), px(0.));
}

#[gpui::test]
fn column_select_reveals_the_moving_end(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), |_| {}, cx);
    let page = page_size(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(page, 0))]);
    // Down: Monaco `revealBottomMostCursor`.
    for _ in 0..5 {
        cx.dispatch_action(CursorColumnSelectDown);
    }
    draw(cx);
    assert_eq!(rows(&editor, cx).len(), 6);
    assert!(fully_visible(&editor, cx, page + 5));

    // Up from the start (reversed): `revealTopMostCursor`.
    let (lh, _) = metrics(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(300, 0))]);
    set_scroll_top(&editor, cx, lh * 300.);
    for _ in 0..4 {
        cx.dispatch_action(CursorColumnSelectUp);
    }
    draw(cx);
    assert_eq!(rows(&editor, cx).len(), 5);
    assert!(fully_visible(&editor, cx, 296));
}

#[gpui::test]
fn fold_chevron_clicks_reveal_the_line_not_the_cursor(cx: &mut TestAppContext) {
    let text = "fn a() {\n    one\n}\n".repeat(200);
    let (editor, cx) = setup(&text, |_| {}, cx);
    // Wait for the indentation folding ranges.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    draw(cx);
    let (lh, _) = metrics(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(0, 0))]);
    // Wheel down to row 300, a fold start, and click its chevron.
    set_scroll_top(&editor, cx, lh * 300.);
    editor.update(cx, |e, cx| e.click_fold(300, true, false, false, cx));
    draw(cx);
    let folds = editor.read_with(cx, |e, cx| e.collapsed_folds(cx));
    assert_eq!(folds, [(300, 301)]);
    // Monaco: no jump back to the cursor on row 0.
    assert_eq!(scroll_top(&editor, cx), lh * 300.);
    assert_eq!(rows(&editor, cx), [0]);

    // A fold command reveals the cursor, centered if off screen
    // (`FoldingAction`, `revealPositionInCenterIfOutsideViewport`).
    editor.update(cx, |e, cx| e.fold(cx));
    draw(cx);
    assert_eq!(scroll_top(&editor, cx), px(0.));
}

#[gpui::test]
fn editor_scroll_reveal_cursor_keeps_the_scroll(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), |_| {}, cx);
    let (lh, _) = metrics(&editor, cx);
    let page = page_size(&editor, cx);
    select(
        &editor,
        cx,
        &[Selection::collapsed(p(1, 3)), Selection::collapsed(p(2, 0))],
    );
    // Monaco scrolls a page and puts the primary cursor, alone, on the
    // first line that fully shows.
    trigger_with(
        &editor,
        cx,
        "editorScroll",
        json!({ "to": "down", "by": "page", "revealCursor": true }),
    );
    assert_eq!(scroll_top(&editor, cx), lh * page as f32);
    let selections = editor.read_with(cx, |e, _| e.selections());
    assert_eq!(selections, [Selection::collapsed(p(page, 3))]);

    // Scrolled past by a line: onto the new first line.
    trigger_with(
        &editor,
        cx,
        "editorScroll",
        json!({ "to": "down", "by": "line", "revealCursor": true }),
    );
    assert_eq!(scroll_top(&editor, cx), lh * (page + 1) as f32);
    assert_eq!(rows(&editor, cx), [page + 1]);

    // A cursor that still shows stays where it is.
    select(&editor, cx, &[Selection::collapsed(p(page + 5, 0))]);
    trigger_with(
        &editor,
        cx,
        "editorScroll",
        json!({ "to": "down", "by": "line", "revealCursor": true }),
    );
    assert_eq!(scroll_top(&editor, cx), lh * (page + 2) as f32);
    assert_eq!(rows(&editor, cx), [page + 5]);

    // Up: onto the last line but one that fully shows (Monaco keeps the
    // cursor off the range's last line).
    let (_, height) = metrics(&editor, cx);
    let fit = (height / lh).floor() as u32;
    trigger_with(
        &editor,
        cx,
        "editorScroll",
        json!({ "to": "up", "by": "page", "revealCursor": true }),
    );
    assert_eq!(scroll_top(&editor, cx), lh * 2.);
    assert_eq!(rows(&editor, cx), [fit]);
}

#[gpui::test]
fn editor_scroll_reveal_cursor_keeps_surrounding_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), |opts| opts.cursor_surrounding_lines = 3, cx);
    let (lh, _) = metrics(&editor, cx);
    let page = page_size(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(0, 0))]);
    // Monaco `getViewRangeWithCursorPadding`: three lines in from the top.
    trigger_with(
        &editor,
        cx,
        "editorScroll",
        json!({ "to": "down", "by": "page", "revealCursor": true }),
    );
    assert_eq!(scroll_top(&editor, cx), lh * page as f32);
    assert_eq!(rows(&editor, cx), [page + 3]);
}

#[gpui::test]
fn page_down_moves_a_page_and_reveals_minimally(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), |_| {}, cx);
    let (lh, height) = metrics(&editor, cx);
    let page = page_size(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(0, 0))]);
    // Monaco: `pageSize` lines down, and the cursor shows already.
    cx.dispatch_action(CursorPageDown);
    draw(cx);
    assert_eq!(rows(&editor, cx), [page]);
    assert_eq!(scroll_top(&editor, cx), px(0.));
    // Again: just enough scrolling to show it, with a line below.
    cx.dispatch_action(CursorPageDown);
    draw(cx);
    assert_eq!(rows(&editor, cx), [2 * page]);
    let expected = lh * (2 * page + 2) as f32 - height;
    assert!(
        (scroll_top(&editor, cx) - expected).abs() < px(0.5),
        "{:?} != {expected:?}",
        scroll_top(&editor, cx)
    );
    // Back up: the scroll stays while the cursor shows.
    let top = scroll_top(&editor, cx);
    cx.dispatch_action(CursorPageUp);
    draw(cx);
    assert_eq!(rows(&editor, cx), [page]);
    assert_eq!(scroll_top(&editor, cx), top);
}

#[gpui::test]
fn scroll_page_down_keeps_two_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), |_| {}, cx);
    let (lh, _) = metrics(&editor, cx);
    let page = page_size(&editor, cx);
    // Monaco `scrollPageDown`: `pageSize` lines, two less than show.
    cx.dispatch_action(ScrollPageDown);
    draw(cx);
    assert_eq!(scroll_top(&editor, cx), lh * page as f32);
    cx.dispatch_action(ScrollPageUp);
    draw(cx);
    assert_eq!(scroll_top(&editor, cx), px(0.));
    // The cursor doesn't move.
    assert_eq!(rows(&editor, cx), [0]);
}

// Ports of Monaco's `cursorMoveCommand.test.ts` viewport cases, with a real
// viewport instead of a stubbed `getCompletelyVisibleViewRange`: the cursor
// moves to a line that shows, so the reveal that follows doesn't scroll.

const TEXT: &str = "    \tMy First Line\t \n\tMy Second Line\n    Third Line🐶\n\n1";

/// Monaco's `TEXT`, then enough filler lines to scroll.
fn viewport_text() -> String {
    format!("{TEXT}\n{}", lines(200))
}

fn cursor_move(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, args: serde_json::Value) {
    trigger_with(editor, cx, "cursorMove", args);
}

#[gpui::test]
fn move_to_view_top_line_moves_to_first_visible_line_if_it_is_first_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&viewport_text(), |_| {}, cx);
    select(&editor, cx, &[Selection::collapsed(p(1, 1))]);
    cursor_move(&editor, cx, json!({ "to": "viewPortTop" }));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()),
        [Selection::collapsed(p(0, 5))]
    );
    assert_eq!(scroll_top(&editor, cx), px(0.));
}

#[gpui::test]
fn move_to_view_top_line_moves_to_top_visible_line_when_first_line_is_not_visible(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(&viewport_text(), |_| {}, cx);
    let (lh, _) = metrics(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(3, 0))]);
    set_scroll_top(&editor, cx, lh);
    cursor_move(&editor, cx, json!({ "to": "viewPortTop" }));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()),
        [Selection::collapsed(p(1, 1))]
    );
    assert_eq!(scroll_top(&editor, cx), lh);
}

#[gpui::test]
fn move_to_view_top_line_moves_to_nth_line_from_top(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&viewport_text(), |_| {}, cx);
    select(&editor, cx, &[Selection::collapsed(p(3, 0))]);
    cursor_move(&editor, cx, json!({ "to": "viewPortTop", "value": 3 }));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()),
        [Selection::collapsed(p(2, 4))]
    );
    assert_eq!(scroll_top(&editor, cx), px(0.));
}

/// Monaco `FoldingAction.runEditorCommand`: Go to Parent Fold reveals the
/// fold's start centered when it's off screen.
#[gpui::test]
fn go_to_parent_fold_centers_a_far_parent(cx: &mut TestAppContext) {
    let body = "    x\n".repeat(400);
    let text = format!("{}fn a() {{\n{body}}}\n", lines(200) + "\n");
    let (editor, cx) = setup(&text, |_| {}, cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    draw(cx);
    let (lh, height) = metrics(&editor, cx);
    select(&editor, cx, &[Selection::collapsed(p(590, 0))]);
    set_scroll_top(&editor, cx, lh * 580.);
    editor.update(cx, |e, cx| e.go_to_parent_fold(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx), [200]);
    // Centered: the line's middle at the viewport's.
    let middle = scroll_top(&editor, cx) + height / 2.;
    let y = lh * 200.;
    assert!(y <= middle && middle <= y + lh, "{middle:?} {y:?}");
}

/// Monaco's viewport targets count only the lines that show in full
/// (`getCompletelyVisibleViewRange`, #333): with half a line cut off at
/// the top and bottom, neither is a target.
#[gpui::test]
fn viewport_targets_skip_partly_visible_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&viewport_text(), |_| {}, cx);
    let (lh, height) = metrics(&editor, cx);
    let top = lh * 10.5;
    set_scroll_top(&editor, cx, top);
    let (first, last) = (11, ((top + height) / lh).floor() as u32 - 1);
    // Rows 10 and `last + 1` show in part.
    assert_eq!(visible(&editor, cx), (10, last + 1));
    assert!(!fully_visible(&editor, cx, 10) && !fully_visible(&editor, cx, last + 1));
    assert!(fully_visible(&editor, cx, first) && fully_visible(&editor, cx, last));

    for (args, row) in [
        (json!({ "to": "viewPortTop" }), first),
        (json!({ "to": "viewPortTop", "value": 2 }), first + 1),
        (json!({ "to": "viewPortBottom" }), last),
        (json!({ "to": "viewPortBottom", "value": 2 }), last - 1),
        (
            json!({ "to": "viewPortCenter" }),
            (first + last).div_ceil(2),
        ),
    ] {
        set_scroll_top(&editor, cx, top);
        select(&editor, cx, &[Selection::collapsed(p(0, 0))]);
        cursor_move(&editor, cx, args.clone());
        assert_eq!(rows(&editor, cx), [row], "{args}");
        // The reveal that follows keeps a line below the cursor (Monaco
        // `_computeScrollTopToRevealRange`), so only the bottom scrolls.
        if args["to"] != "viewPortBottom" {
            assert_eq!(scroll_top(&editor, cx), top, "{args}");
        }
    }

    // Monaco `findPositionInViewportIfOutside`: a cursor on a partly
    // visible line moves onto the nearest line in full view (the last of
    // which Monaco counts as outside), keeping its column.
    for (from, to) in [
        (10, first),
        (last + 1, last - 1),
        (last, last - 1),
        (12, 12),
    ] {
        set_scroll_top(&editor, cx, top);
        select(&editor, cx, &[Selection::collapsed(p(from, 2))]);
        cursor_move(&editor, cx, json!({ "to": "viewPortIfOutside" }));
        assert_eq!(
            editor.read_with(cx, |e, _| e.selections()),
            [Selection::collapsed(p(to, 2))],
            "from {from}"
        );
        assert_eq!(scroll_top(&editor, cx), top, "from {from}");
    }
}
