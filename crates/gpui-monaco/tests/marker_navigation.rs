//! F8 marker navigation (Monaco `MarkerList`, `MarkerController` and
//! `MarkerNavigationWidget`): which markers are visited and in what order,
//! the zone following marker changes, revealing it and its accessibility.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, Marker, MarkerSeverity, MarkerZoneState, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///main.rs"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn marker(severity: MarkerSeverity, message: &str, row: u32, columns: (u32, u32)) -> Marker {
    Marker {
        severity,
        message: message.to_string().into(),
        range: PointUtf16::new(row, columns.0)..PointUtf16::new(row, columns.1),
        ..Default::default()
    }
}

fn set_markers(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, markers: Vec<Marker>) {
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_markers("test", markers, cx))
    });
    draw(cx);
}

fn move_to(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32, column: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(row, column))], cx)
    });
}

fn go(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, next: bool) -> MarkerZoneState {
    editor.update(cx, |e, cx| e.go_to_marker(next, cx));
    draw(cx);
    zone(editor, cx)
}

fn zone(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> MarkerZoneState {
    editor
        .read_with(cx, |e, cx| e.marker_zone_state_for_tests(cx))
        .expect("marker zone")
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

/// Monaco `MarkerList` reads errors, warnings and infos only: hints aren't
/// visited or counted. The playground's demo markers: Alt+F8 on line 1
/// goes to the error, "1 of 2".
#[gpui::test]
fn hints_are_not_problems(cx: &mut TestAppContext) {
    let text: String = (0..12).map(|i| format!("line {i}\n")).collect();
    let (editor, cx) = setup(&text, cx);
    set_markers(
        &editor,
        cx,
        vec![
            marker(MarkerSeverity::Hint, "unused import", 0, (0, 4)),
            marker(MarkerSeverity::Warning, "unused variable", 4, (0, 4)),
            marker(MarkerSeverity::Hint, "could be const", 7, (0, 4)),
            marker(MarkerSeverity::Error, "mismatched types", 10, (0, 4)),
        ],
    );
    cx.simulate_keystrokes("alt-f8");
    draw(cx);
    let state = zone(&editor, cx);
    assert_eq!(
        (state.message.as_ref(), state.index, state.total),
        ("mismatched types", 1, 2)
    );
    let state = go(&editor, cx, true);
    assert_eq!(
        (state.message.as_ref(), state.index, state.total),
        ("unused variable", 2, 2)
    );
    let state = go(&editor, cx, true);
    assert_eq!(state.message.as_ref(), "mismatched types");
    // Only hints: nothing to go to.
    editor.update(cx, |e, cx| e.close_marker_zone(cx));
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Hint, "unused import", 0, (0, 4))],
    );
    editor.update(cx, |e, cx| e.go_to_marker(true, cx));
    assert!(
        editor
            .read_with(cx, |e, cx| e.marker_zone_state_for_tests(cx))
            .is_none()
    );
}

/// Most severe first, then by position (standalone Monaco has no
/// `problems.sortOrder`), wrapping around.
#[gpui::test]
fn markers_go_by_severity_then_position(cx: &mut TestAppContext) {
    let text: String = (0..10).map(|i| format!("line {i}\n")).collect();
    let (editor, cx) = setup(&text, cx);
    set_markers(
        &editor,
        cx,
        vec![
            marker(MarkerSeverity::Info, "info", 0, (0, 4)),
            marker(MarkerSeverity::Warning, "warning", 1, (0, 4)),
            marker(MarkerSeverity::Error, "late error", 5, (0, 4)),
            marker(MarkerSeverity::Error, "early error", 3, (0, 4)),
        ],
    );
    move_to(&editor, cx, 0, 0);
    // The first move: the first marker, in that order, at or after the
    // cursor.
    let visits: Vec<(String, usize)> = (0..5)
        .map(|_| {
            let state = go(&editor, cx, true);
            (state.message.to_string(), state.index)
        })
        .collect();
    assert_eq!(
        visits,
        [
            ("early error".to_string(), 1),
            ("late error".to_string(), 2),
            ("warning".to_string(), 3),
            ("info".to_string(), 4),
            ("early error".to_string(), 1),
        ]
    );
    assert_eq!(go(&editor, cx, false).message.as_ref(), "info");
    // Backwards from after every marker: the last one.
    editor.update(cx, |e, cx| e.close_marker_zone(cx));
    move_to(&editor, cx, 9, 0);
    assert_eq!(go(&editor, cx, false).message.as_ref(), "info");
}

/// The first move takes the marker containing the cursor, and the zone
/// shows at the cursor; later moves step, so markers starting at the same
/// place are each visited.
#[gpui::test]
fn the_first_move_takes_the_marker_at_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let value = compute(1, 2);\nlet other = 3;\n", cx);
    set_markers(
        &editor,
        cx,
        vec![
            marker(MarkerSeverity::Error, "first", 0, (12, 25)),
            marker(MarkerSeverity::Error, "second", 0, (12, 19)),
            marker(MarkerSeverity::Error, "other", 1, (4, 9)),
        ],
    );
    move_to(&editor, cx, 0, 15);
    let state = go(&editor, cx, true);
    // Same start, so by end: "second" (12..19) sorts first.
    assert_eq!((state.message.as_ref(), state.index), ("second", 1));
    assert_eq!(head(&editor, cx), PointUtf16::new(0, 15));
    let state = go(&editor, cx, true);
    assert_eq!((state.message.as_ref(), state.index), ("first", 2));
    let state = go(&editor, cx, true);
    assert_eq!((state.message.as_ref(), state.index), ("other", 3));
    assert_eq!(head(&editor, cx), PointUtf16::new(1, 4));
    // Backwards from inside a marker: that marker too.
    editor.update(cx, |e, cx| e.close_marker_zone(cx));
    move_to(&editor, cx, 1, 6);
    assert_eq!(go(&editor, cx, false).message.as_ref(), "other");
    // Moving the cursor off the shown marker starts from the cursor again.
    move_to(&editor, cx, 0, 13);
    assert_eq!(go(&editor, cx, true).message.as_ref(), "second");
}

/// An empty marker contains the word at its start (Monaco `_initIdx`).
#[gpui::test]
fn an_empty_marker_covers_its_word(cx: &mut TestAppContext) {
    let (editor, cx) = setup("call missing here\n", cx);
    set_markers(
        &editor,
        cx,
        vec![
            marker(MarkerSeverity::Warning, "empty", 0, (5, 5)),
            marker(MarkerSeverity::Warning, "later", 0, (13, 17)),
        ],
    );
    move_to(&editor, cx, 0, 9);
    assert_eq!(go(&editor, cx, true).message.as_ref(), "empty");
    // Shown at its start: the cursor isn't in the empty range.
    assert_eq!(head(&editor, cx), PointUtf16::new(0, 5));
}

/// Monaco `MarkerController`: when the markers change, the zone shows the
/// marker now at its position, or turns stale.
#[gpui::test]
fn the_zone_follows_marker_changes(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let a = 1;\nlet b = 2;\n", cx);
    set_markers(
        &editor,
        cx,
        vec![
            marker(MarkerSeverity::Error, "old message", 0, (4, 5)),
            marker(MarkerSeverity::Warning, "elsewhere", 1, (4, 5)),
        ],
    );
    move_to(&editor, cx, 0, 0);
    let state = go(&editor, cx, true);
    assert_eq!(state.message.as_ref(), "old message");
    set_markers(
        &editor,
        cx,
        vec![
            marker(MarkerSeverity::Error, "new message", 0, (4, 5)),
            marker(MarkerSeverity::Warning, "elsewhere", 1, (4, 5)),
        ],
    );
    let state = zone(&editor, cx);
    assert_eq!(
        (state.message.as_ref(), state.stale),
        ("new message", false)
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.last_announcement_for_tests()),
        Some("let a = 1;, Error: new message at 1:5. ".into())
    );
    // Monaco `aria.status`: polite.
    assert!(editor.read_with(cx, |e, _| e.last_announcement_is_status_for_tests()));
    // The host clears them: the zone stays, stale.
    set_markers(&editor, cx, Vec::new());
    let state = zone(&editor, cx);
    assert_eq!((state.message.as_ref(), state.stale), ("new message", true));
    // A marker at the position again: fresh.
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, "back", 0, (4, 5))],
    );
    let state = zone(&editor, cx);
    assert_eq!((state.message.as_ref(), state.stale), ("back", false));
    // The list was reset: the next move starts from the cursor.
    let state = go(&editor, cx, true);
    assert_eq!(
        (state.message.as_ref(), state.index, state.total),
        ("back", 1, 1)
    );
}

/// `ZoneWidget.show` reveals the marker's line and the zone under it, and
/// `showAtMarker` the position.
#[gpui::test]
fn showing_the_zone_reveals_it(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (editor, cx) = setup(&text, cx);
    let (viewport, line_height) = editor.read_with(cx, |e, _| {
        (
            e.layout_info().expect("laid out").height,
            e.options().resolved_line_height(),
        )
    });
    let visible_rows = (viewport / line_height).floor() as u32;
    // On the last fully visible line: the zone would open below the
    // viewport.
    let row = visible_rows - 1;
    set_markers(
        &editor,
        cx,
        vec![
            // Ten lines: a zone taller than the cursor's own reveal makes
            // room for.
            marker(
                MarkerSeverity::Error,
                &(0..10)
                    .map(|i| format!("note {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                row,
                (0, 4),
            ),
            marker(MarkerSeverity::Error, "far below", 150, (0, 4)),
        ],
    );
    move_to(&editor, cx, 0, 0);
    go(&editor, cx, true);
    draw(cx);
    let window_bottom = editor.read_with(cx, |e, _| e.layout_info().expect("laid out").height);
    let (bounds, _, _) = editor
        .read_with(cx, |e, cx| e.marker_zone_for_tests(cx))
        .expect("zone painted");
    let top = editor.read_with(cx, |e, _| e.scroll_top());
    assert!(top > gpui::px(0.), "scrolled to show the zone");
    assert!(
        bounds.bottom() <= window_bottom + gpui::px(1.),
        "{bounds:?} in {window_bottom:?}"
    );
    // Far below: the line comes into view, with its zone.
    go(&editor, cx, true);
    draw(cx);
    let top = editor.read_with(cx, |e, _| e.scroll_top());
    let line_top = line_height * 150.;
    assert!(top <= line_top && line_top < top + viewport, "{top:?}");
    let (bounds, _, _) = editor
        .read_with(cx, |e, cx| e.marker_zone_for_tests(cx))
        .expect("zone painted");
    assert!(bounds.bottom() <= window_bottom + gpui::px(1.));
}

/// After the line and zone, `revealPositionInCenterIfOutsideViewport`
/// scrolls horizontally to a marker far along its line.
#[gpui::test]
fn showing_the_zone_reveals_the_position(cx: &mut TestAppContext) {
    let text = format!("short\n{}\nshort\n", "x".repeat(600));
    let (editor, cx) = setup(&text, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, "far right", 1, (500, 510))],
    );
    move_to(&editor, cx, 0, 0);
    go(&editor, cx, true);
    draw(cx);
    let position = editor.read_with(cx, |e, _| {
        e.scrolled_visible_position(PointUtf16::new(1, 500))
    });
    let width = editor.read_with(cx, |e, _| e.layout_info().expect("laid out").width);
    let position = position.expect("the marker's start is laid out");
    assert!(editor.read_with(cx, |e, _| e.scroll_left()) > gpui::px(0.));
    assert!(
        position.x > gpui::px(0.) && position.x < width,
        "{position:?} in {width:?}"
    );
}

/// Monaco's message is an assertive `role="alert"` labelled (and read
/// with `aria.status`) "<line>, <severity>: <message> at <line>:<column>.";
/// the body is focusable, `role="tooltip"`.
#[gpui::test]
fn the_zone_is_accessible(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn main() {\n    let x: u8 = 300;\n}\n", cx);
    set_markers(
        &editor,
        cx,
        vec![marker(
            MarkerSeverity::Warning,
            "literal out of range",
            1,
            (16, 19),
        )],
    );
    move_to(&editor, cx, 0, 0);
    let state = go(&editor, cx, true);
    let expected = "    let x: u8 = 300;, Warning: literal out of range at 2:17. ";
    assert_eq!(state.aria_label.as_ref(), expected);
    assert_eq!(
        editor.read_with(cx, |e, _| e.last_announcement_for_tests()),
        Some(expected.into())
    );
    // Clicking the body focuses it.
    let (bounds, _, _) = editor
        .read_with(cx, |e, cx| e.marker_zone_for_tests(cx))
        .expect("zone painted");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    draw(cx);
    let focused = cx.update(|window, _| state.focus_handle.is_focused(window));
    assert!(focused, "the body takes focus");
    // Closing the zone gives the editor its focus back.
    editor.update(cx, |e, cx| e.close_marker_zone(cx));
    draw(cx);
    let focused = cx.update(|window, cx| editor.focus_handle(cx).is_focused(window));
    assert!(focused, "the editor has focus again");
}
