//! The peek view's frame, results list and keyboard (#252): Monaco's
//! `ReferenceWidget`, `PeekViewWidget` and `ZoneWidget`, and the workbench
//! list behind the references tree.

use gpui::{
    AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, Pixels, ScrollDelta,
    ScrollWheelEvent, TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::mouse_target::EmbeddedKind;
use gpui_monaco::providers::Location;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

const URI: &str = "file:///src/main.rs";

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_model_resolver(cx, |uri, cx| {
            (uri == "file:///src/lib.rs").then(|| {
                cx.new(|_| TextModel::new("pub fn alpha() {}\npub fn beta() {}").with_uri(uri))
            })
        });
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri(URI));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.minimap.enabled = false;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, _| window.activate_window());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn here(row: u32, start: u32, end: u32) -> Location {
    Location {
        uri: None,
        range: p(row, start)..p(row, end),
        ..Default::default()
    }
}

fn elsewhere(row: u32, start: u32, end: u32) -> Location {
    Location {
        uri: Some("file:///src/lib.rs".into()),
        range: p(row, start)..p(row, end),
        ..Default::default()
    }
}

fn peek(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, locations: Vec<Location>) {
    editor.update(cx, |e, cx| e.peek_locations(p(0, 0), locations, None, cx));
    draw(cx);
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
}

/// (row labels, focused row, list scroll offset, rows rendered).
fn list(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> (Vec<String>, usize, Pixels, std::ops::Range<usize>) {
    editor
        .read_with(cx, |e, cx| e.peek_list_for_tests(cx))
        .expect("a peek view")
}

fn numbered(count: usize) -> String {
    (0..count)
        .map(|i| format!("call{i:03}();"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[gpui::test]
fn one_file_lists_its_results_without_a_file_row(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&numbered(400), cx);
    peek(&editor, cx, (0..300).map(|row| here(row, 0, 4)).collect());
    let (labels, focused, _, rendered) = list(&editor, cx);
    // Monaco: with one file, the tree's roots are the results.
    assert_eq!(labels.len(), 300);
    assert!(labels.iter().all(|label| label.starts_with("  ")));
    assert_eq!(labels[0], "  call000();");
    assert_eq!(focused, 0);
    // Virtualized: only the rows in view, 23px each.
    let results = cx.debug_bounds("peek-results").unwrap();
    let in_view = (results.size.height / px(23.)).ceil() as usize;
    assert_eq!(rendered, 0..in_view);
}

#[gpui::test]
fn several_files_get_file_rows_and_tree_keys(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn main() {\n    alpha();\n    beta();\n}", cx);
    peek(
        &editor,
        cx,
        vec![
            elsewhere(0, 7, 12),
            elsewhere(1, 7, 11),
            here(1, 4, 9),
            here(2, 4, 8),
        ],
    );
    let (labels, focused, _, _) = list(&editor, cx);
    assert_eq!(
        labels,
        vec![
            "lib.rs",
            "  pub fn alpha() {}",
            "  pub fn beta() {}",
            "main.rs",
            "  alpha();",
            "  beta();",
        ]
    );
    // Nearest to the cursor in this file.
    assert_eq!(focused, 4);
    let focused = |cx: &mut VisualTestContext| list(&editor, cx).1;
    let rows = |cx: &mut VisualTestContext| list(&editor, cx).0.len();
    // Home focuses the first row, a file (Monaco `list.focusFirst`).
    cx.simulate_keystrokes("home");
    draw(cx);
    assert_eq!(focused(cx), 0);
    // Left on a file collapses it; Right expands it, then goes in.
    cx.simulate_keystrokes("left");
    draw(cx);
    assert_eq!(rows(cx), 4);
    cx.simulate_keystrokes("right");
    draw(cx);
    assert_eq!((rows(cx), focused(cx)), (6, 0));
    cx.simulate_keystrokes("right");
    draw(cx);
    assert_eq!(focused(cx), 1);
    // Left on a result goes to its file without collapsing it.
    cx.simulate_keystrokes("down left");
    draw(cx);
    assert_eq!((rows(cx), focused(cx)), (6, 0));
    // Space and Enter toggle a file.
    cx.simulate_keystrokes("space");
    draw(cx);
    assert_eq!(rows(cx), 4);
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(rows(cx), 6);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
}

#[gpui::test]
fn page_down_and_up_move_by_the_rows_in_view(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&numbered(400), cx);
    peek(&editor, cx, (0..200).map(|row| here(row, 0, 4)).collect());
    let results = cx.debug_bounds("peek-results").unwrap();
    // Rows wholly in view.
    let page = (results.size.height / px(23.)).floor() as usize;
    // Monaco `focusNextPage`: to the last row in view first.
    cx.simulate_keystrokes("pagedown");
    draw(cx);
    let (_, focused, scroll, _) = list(&editor, cx);
    assert_eq!(scroll, px(0.));
    assert!(focused + 1 == page || focused == page, "{focused} {page}");
    // Then a page on, scrolling.
    cx.simulate_keystrokes("pagedown");
    draw(cx);
    let (_, next, scroll, _) = list(&editor, cx);
    assert!(scroll > px(0.));
    assert!(next > focused + page / 2);
    // PageUp: to the first row in view, then a page back.
    cx.simulate_keystrokes("pageup");
    draw(cx);
    let (_, up, up_scroll, _) = list(&editor, cx);
    assert_eq!(up_scroll, scroll);
    assert!(up < next && px(23.) * up as f32 >= scroll - px(23.));
    // The preview follows the focus (`selectionNavigation`).
    let preview = editor.read_with(cx, |e, cx| e.peek_preview(cx)).unwrap();
    assert_eq!(
        preview.read_with(cx, |p, _| p.selections()[0].head),
        p(up as u32, 0)
    );
}

#[gpui::test]
fn typing_moves_to_a_matching_row(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one();\ntwo();\nthree();\nfour();", cx);
    peek(
        &editor,
        cx,
        vec![here(0, 0, 3), here(1, 0, 3), here(2, 0, 5), here(3, 0, 4)],
    );
    // Monaco `TypeNavigationController`: a prefix, from after the focus.
    cx.simulate_keystrokes("t");
    draw(cx);
    assert_eq!(list(&editor, cx).1, 1);
    // More characters narrow it from there.
    cx.simulate_keystrokes("h");
    draw(cx);
    assert_eq!(list(&editor, cx).1, 2);
    let preview = editor.read_with(cx, |e, cx| e.peek_preview(cx)).unwrap();
    assert_eq!(
        preview.read_with(cx, |p, _| p.selections()[0].head),
        p(2, 0)
    );
}

#[gpui::test]
fn the_wheel_scrolls_the_list_and_not_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&numbered(400), cx);
    peek(&editor, cx, (0..100).map(|row| here(row, 0, 4)).collect());
    let editor_top = editor.read_with(cx, |e, _| e.scroll_top());
    let results = cx.debug_bounds("peek-results").unwrap();
    cx.simulate_event(ScrollWheelEvent {
        position: results.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-100.))),
        ..Default::default()
    });
    draw(cx);
    assert_eq!(list(&editor, cx).2, px(100.));
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), editor_top);
    // Scrolled: rows render from further down.
    assert_eq!(list(&editor, cx).3.start, 4);
}

#[gpui::test]
fn the_title_opens_the_result_and_the_close_action_closes(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn main() {\n    alpha();\n    beta();\n}", cx);
    peek(&editor, cx, vec![here(1, 4, 9), here(2, 4, 8)]);
    cx.simulate_keystrokes("down");
    draw(cx);
    // Monaco `supportOnTitleClick`: the title opens the selected result.
    let title = cx.debug_bounds("peek-title").unwrap();
    cx.simulate_click(title.center(), Modifiers::default());
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), p(2, 4));

    peek(&editor, cx, vec![here(1, 4, 9), here(2, 4, 8)]);
    let close = cx.debug_bounds("peek-close").unwrap();
    cx.simulate_click(close.center(), Modifiers::default());
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
}

#[gpui::test]
fn the_frame_follows_the_zone_widget_layout(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&numbered(100), cx);
    peek(&editor, cx, vec![here(1, 0, 4), here(5, 0, 4)]);
    let lh = editor.read_with(cx, |e, _| e.layout_for_tests().unwrap().1);
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let zone = widgets
        .iter()
        .find(|(_, kind)| *kind == EmbeddedKind::ViewZone)
        .unwrap()
        .0;
    let results = cx.debug_bounds("peek-results").unwrap();
    // Monaco: the arrow (a third of a line), the 1px frame, the header
    // (1.2 lines), the body's 1px top border, then the body; an arrow's
    // height is left below the frame.
    let arrow = (lh / 3.).round();
    let head = (lh * 1.2).ceil();
    assert_eq!(results.top(), zone.top() + arrow + px(1.) + head + px(1.));
    let body = (lh * 18. - arrow * 2. - head - px(1.)).round();
    assert_eq!(results.size.height, body);
    // The height sash sits on the frame's bottom.
    let sash = cx.debug_bounds("peek-sash-height").unwrap();
    assert_eq!(sash.center().y, results.bottom() + px(1.));
}

#[gpui::test]
fn the_height_sash_moves_in_whole_lines_from_where_it_started(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&numbered(100), cx);
    peek(&editor, cx, vec![here(1, 0, 4), here(5, 0, 4)]);
    let lh = editor.read_with(cx, |e, _| e.layout_for_tests().unwrap().1);
    let zone_height = |cx: &mut VisualTestContext| {
        let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
        widgets
            .iter()
            .find(|(_, kind)| *kind == EmbeddedKind::ViewZone)
            .unwrap()
            .0
            .size
            .height
    };
    let before = zone_height(cx);
    let sash = cx.debug_bounds("peek-sash-height").unwrap();
    let start = sash.center();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    // The drag starts once the mouse moves.
    cx.simulate_mouse_move(
        start + point(px(0.), px(3.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw(cx);
    // 2.6 lines down: 2 more lines (`floor`).
    cx.simulate_mouse_move(
        start + point(px(0.), lh * 2.6),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw(cx);
    assert_eq!(zone_height(cx), before + lh * 2.);
    // Back to 1.6 lines up from the start: 1 fewer (`ceil`).
    cx.simulate_mouse_move(
        start - point(px(0.), lh * 1.6),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw(cx);
    assert_eq!(zone_height(cx), before - lh);
    cx.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui::test]
fn the_preview_uses_monaco_options(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&numbered(50), cx);
    peek(&editor, cx, vec![here(1, 0, 4), here(5, 0, 4)]);
    let preview = editor.read_with(cx, |e, cx| e.peek_preview(cx)).unwrap();
    let options = preview.read_with(cx, |p, _| p.options().clone());
    // Monaco `ReferenceWidget._fillBody`.
    assert!(!options.scroll_beyond_last_line);
    assert!(!options.minimap.enabled);
    assert_eq!(options.overview_ruler_lanes, 2);
    assert_eq!(options.vertical_scrollbar_size, px(14.));
    assert!(options.scrollbar.always_consume_mouse_wheel);
    // Sticky scroll stays on (`peekViewEditorStickyScroll*` colors).
    let outer = editor.read_with(cx, |e, _| e.options().sticky_scroll.enabled);
    assert_eq!(options.sticky_scroll.enabled, outer);
}

#[gpui::test]
fn f4_only_moves_through_results_while_a_peek_view_is_open(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo\nthree\nfour", cx);
    peek(
        &editor,
        cx,
        vec![here(1, 0, 3), here(2, 0, 5), here(3, 0, 4)],
    );
    let selected =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap().1;
    let start = selected(cx);
    // In the editor (not the list), F4 and Shift+F4 step through the
    // results (Monaco `goToNextReference` with `referenceSearchVisible`).
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    cx.simulate_keystrokes("f4");
    draw(cx);
    assert_eq!(selected(cx), (start + 1) % 3);
    cx.simulate_keystrokes("shift-f4");
    draw(cx);
    assert_eq!(selected(cx), start);
    // With the peek view closed, F4 does nothing.
    editor.update(cx, |e, cx| e.close_peek(cx));
    draw(cx);
    let head = editor.read_with(cx, |e, _| e.selections()[0].head);
    cx.simulate_keystrokes("f4");
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), head);
}
