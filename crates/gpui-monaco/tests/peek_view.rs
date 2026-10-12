//! The peek view's frame, results list and keyboard (#252): Monaco's
//! `ReferenceWidget`, `PeekViewWidget` and `ZoneWidget`, and the workbench
//! list behind the references tree.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, ScrollDelta, ScrollWheelEvent, Task, TestAppContext, VisualTestContext,
    point, px,
};
use gpui_monaco::actions::FindReferences;
use gpui_monaco::cursor::Selection;
use gpui_monaco::mouse_target::EmbeddedKind;
use gpui_monaco::options::GoToLocation;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, TextModel};

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
    // Monaco `openReference` selects the whole result.
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::new(p(2, 4), p(2, 8))
    );

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
fn the_frame_uses_the_zoomed_line_height(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&numbered(100), cx);
    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(1., cx));
    draw(cx);
    peek(&editor, cx, vec![here(1, 0, 4), here(5, 0, 4)]);
    let lh = editor.read_with(cx, |e, _| e.layout_for_tests().unwrap().1);
    let unzoomed = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    assert!(lh > unzoomed);
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let zone = widgets
        .iter()
        .find(|(_, kind)| *kind == EmbeddedKind::ViewZone)
        .unwrap()
        .0;
    let results = cx.debug_bounds("peek-results").unwrap();
    // Monaco's peek view lays out from `EditorOption.lineHeight`, which
    // includes the font zoom.
    let arrow = (lh / 3.).round();
    let head = (lh * 1.2).ceil();
    // Within the pixel snapping of the fractional zoomed zone top.
    let expected = zone.top() + arrow + px(1.) + head + px(1.);
    assert!((results.top() - expected).abs() < px(0.5));
    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(0., cx));
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

/// Monaco `goToNextOrPreviousReference`: F4 and Shift+F4 go from the
/// result nearest the view to the next or previous one, moving the cursor
/// and the view there, and give the focus back to where it was.
#[gpui::test]
fn f4_moves_the_cursor_and_the_view_while_a_peek_view_is_open(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo\nthree\nfour", cx);
    peek(
        &editor,
        cx,
        vec![here(1, 0, 3), here(2, 0, 5), here(3, 0, 4)],
    );
    let selected =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap().1;
    let head = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.selections()[0].head);
    let editor_focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
    };
    assert_eq!(selected(cx), 0);
    // In the editor (not the list), F4 and Shift+F4 step through the
    // results (with `referenceSearchVisible`), and the editor keeps focus.
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    cx.simulate_keystrokes("f4");
    draw(cx);
    assert_eq!(selected(cx), 1);
    assert_eq!(head(cx), p(2, 0));
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert!(editor_focused(cx));
    cx.simulate_keystrokes("f4");
    draw(cx);
    cx.simulate_keystrokes("f4");
    draw(cx);
    // Wrapping.
    assert_eq!(selected(cx), 0);
    assert_eq!(head(cx), p(1, 0));
    cx.simulate_keystrokes("shift-f4");
    draw(cx);
    assert_eq!(selected(cx), 2);
    assert_eq!(head(cx), p(3, 0));
    assert!(editor_focused(cx));
    // The view moved: its zone is under the new line.
    let zone_top = |cx: &mut VisualTestContext| cx.debug_bounds("peek-title").unwrap().top();
    let at_last = zone_top(cx);
    cx.simulate_keystrokes("shift-f4");
    draw(cx);
    assert!(zone_top(cx) < at_last);
    // From the list, the list keeps focus. (From the editor, the toggle
    // focuses the preview, then the list.)
    cx.update(|window, cx| editor.update(cx, |e, cx| e.toggle_peek_widget_focus(window, cx)));
    draw(cx);
    cx.update(|window, cx| editor.update(cx, |e, cx| e.toggle_peek_widget_focus(window, cx)));
    draw(cx);
    assert!(!editor_focused(cx));
    cx.simulate_keystrokes("shift-f4");
    draw(cx);
    assert_eq!(head(cx), p(1, 0));
    assert!(!editor_focused(cx));
    assert!(!cx.update(|window, cx| editor.read(cx).is_peek_preview_focused(window, cx)));
    // With the peek view closed, F4 does nothing.
    editor.update(cx, |e, cx| e.close_peek(cx));
    draw(cx);
    let before = head(cx);
    cx.simulate_keystrokes("f4");
    draw(cx);
    assert_eq!(head(cx), before);
}

/// F4 to a result in another model closes the view and asks the host to
/// open it (standalone Monaco's `openCodeEditor` opens no other model).
#[gpui::test]
fn f4_to_another_model_closes_the_view(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo", cx);
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::OpenLocation(location) = event {
                *sink.borrow_mut() = Some(location.clone());
            }
        })
        .detach();
    });
    peek(&editor, cx, vec![here(1, 0, 3), elsewhere(1, 7, 11)]);
    // From the result in this model (the nearest), Shift+F4 wraps to
    // lib.rs's.
    cx.simulate_keystrokes("shift-f4");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    let location = opened.borrow().clone().unwrap();
    assert_eq!(location.uri.as_deref(), Some("file:///src/lib.rs"));
    assert_eq!(location.range.start, p(1, 7));
}

/// Monaco `peekMode`: Enter or a double click on a result in this model
/// moves the view there after a Peek command, but closes it and selects
/// the result after a "Go to" command that showed a peek view.
#[gpui::test]
fn enter_moves_the_view_only_in_peek_mode(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo\nthree\nfour", cx);
    let results = vec![here(1, 0, 3), here(2, 0, 5)];
    // `peekLocations` is a Peek command.
    peek(&editor, cx, results.clone());
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::collapsed(p(2, 0))
    );
    // A double click on a result too (the tree's `goto`).
    let rows = cx.debug_bounds("peek-results").unwrap();
    let first = point(rows.left() + px(40.), rows.top() + px(11.));
    cx.simulate_event(MouseDownEvent {
        position: first,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: first,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::collapsed(p(1, 0))
    );
    editor.update(cx, |e, cx| e.close_peek(cx));
    draw(cx);

    // `goToLocations` with `peek` isn't.
    editor.update(cx, |e, cx| {
        e.go_to_locations(p(0, 0), results, Some(GoToLocation::Peek), None, cx)
    });
    draw(cx);
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::new(p(2, 0), p(2, 5))
    );
}

/// Monaco `ZoneWidget._showImpl`: a peek view on the last line puts the
/// line near the top, so the view shows below it.
#[gpui::test]
fn a_peek_view_on_the_last_line_is_revealed(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&numbered(200), cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(199, 0))], cx);
        e.reveal_lines(199, 199, cx);
    });
    draw(cx);
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let before = editor.read_with(cx, |e, _| e.scroll_position().y);
    editor.update(cx, |e, cx| {
        e.peek_locations(p(199, 0), vec![here(0, 0, 4), here(199, 0, 4)], None, cx)
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    let after = editor.read_with(cx, |e, _| e.scroll_position().y);
    // Near the top: Monaco `revealLineNearTop` leaves a fifth of the
    // viewport above, and the scroll-off lines (the line was at the
    // bottom).
    let viewport = line_height * 200. - before;
    assert!(after > before, "{after:?} > {before:?}");
    assert!(
        line_height * 199. - after < viewport * 0.4,
        "{after:?} (viewport {viewport:?})"
    );
}

/// Monaco `editor.action.findReferences`: the peek view opens at once,
/// "Loading..." until the references come, then "No results" without any;
/// run again at the view's position, it closes it.
#[gpui::test]
fn find_references_always_opens_a_peek_view(cx: &mut TestAppContext) {
    struct Slow(Arc<Mutex<Vec<Location>>>);
    impl ReferenceProvider for Slow {
        fn provide_references(
            &self,
            _: &ProviderContext,
            _: PointUtf16,
            _: ReferenceContext,
            cx: &mut App,
        ) -> Task<anyhow::Result<Vec<Location>>> {
            let found = self.0.lock().unwrap().clone();
            let timer = cx.background_executor().timer(Duration::from_millis(100));
            cx.background_executor().spawn(async move {
                timer.await;
                Ok(found)
            })
        }
    }
    let (editor, cx) = setup("fn alpha() {}\nalpha();", cx);
    let found = Arc::new(Mutex::new(Vec::new()));
    let provider = Slow(found.clone());
    cx.update(|_, cx| register_reference_provider(cx, LanguageSelector::All, provider));
    let find = |cx: &mut VisualTestContext| {
        cx.dispatch_action(FindReferences {
            uri: None,
            position: None,
        });
        draw(cx);
    };
    let title =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.peek_title_for_tests(cx));
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    find(cx);
    assert_eq!(title(cx), Some(("Loading...".into(), None, None)));
    cx.executor().advance_clock(Duration::from_millis(100));
    draw(cx);
    assert_eq!(title(cx), Some((String::new(), None, Some("No results"))));
    // The editor keeps focus; the title opens nothing.
    assert!(cx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
    let bounds = cx.debug_bounds("peek-title").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    // Again at the same place: Monaco `toggleWidget` closes it.
    find(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));

    // With references: the list, not in peek mode, so Enter closes.
    *found.lock().unwrap() = vec![here(0, 3, 8), here(1, 0, 5)];
    find(cx);
    cx.executor().advance_clock(Duration::from_millis(100));
    draw(cx);
    assert_eq!(
        title(cx),
        Some(("main.rs".into(), Some("References (2)".into()), None))
    );
    assert_eq!(list(&editor, cx).1, 0);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::new(p(1, 0), p(1, 5))
    );
}

/// Monaco `toggleWidget`: a peek command again where the open view is
/// closes it; elsewhere it opens a new one.
#[gpui::test]
fn peeking_again_at_the_view_closes_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo\nthree", cx);
    let results = vec![here(1, 0, 3), here(2, 0, 5)];
    peek(&editor, cx, results.clone());
    editor.update(cx, |e, cx| {
        e.peek_locations(p(0, 0), results.clone(), None, cx)
    });
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    peek(&editor, cx, results.clone());
    editor.update(cx, |e, cx| e.peek_locations(p(0, 2), results, None, cx));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
}

/// Monaco's preview double click opens the clicked place, not the result
/// (`_onNewModel`'s `onMouseDown` passes the mouse target's range).
#[gpui::test]
fn a_double_click_in_the_preview_opens_the_clicked_place(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn main() {\n    alpha();\n    beta();\n}", cx);
    peek(&editor, cx, vec![here(1, 4, 9), here(2, 4, 8)]);
    let preview = editor.read_with(cx, |e, cx| e.peek_preview(cx)).unwrap();
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let position = preview
        .read_with(cx, |preview, _| preview.scrolled_visible_position(p(2, 6)))
        .unwrap();
    let position = point(position.x + px(1.), position.y + line_height / 2.);
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::collapsed(p(2, 6))
    );
}

/// Monaco captures the pointer on a sash: the height sash's `ns-resize`
/// and the split sash's `ew-resize` stay over the whole window, over the
/// text too, until the release (GPUI's drags keep the dragged element's
/// cursor).
#[gpui::test]
fn dragging_a_sash_keeps_its_pointer_everywhere(cx: &mut TestAppContext) {
    use gpui::CursorStyle;
    let (editor, cx) = setup(&numbered(100), cx);
    peek(&editor, cx, vec![here(1, 0, 4), here(5, 0, 4)]);
    let captured =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.captured_cursor_for_tests(cx));
    assert_eq!(captured(cx), None);
    for (id, cursor) in [
        ("peek-sash-height", CursorStyle::ResizeRow),
        ("peek-sash-split", CursorStyle::ResizeColumn),
    ] {
        let start = cx.debug_bounds(id).unwrap().center();
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        // The drag starts once the mouse moves; then off the sash, over
        // the text above the peek view and outside the editor.
        for to in [
            start + point(px(3.), px(3.)),
            point(px(80.), px(5.)),
            point(px(-20.), px(-20.)),
        ] {
            cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
            draw(cx);
            assert_eq!(captured(cx), Some(cursor), "{id}");
        }
        cx.simulate_mouse_up(
            point(px(80.), px(5.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        draw(cx);
        assert_eq!(captured(cx), None, "{id}");
    }
}

/// Monaco `Sash`: `sash.hoverBorder` shows once the mouse has rested on a
/// sash for 300ms (`hoverDelay`), and goes as soon as it leaves.
#[gpui::test]
fn sashes_show_their_hover_border_after_a_delay(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (editor, cx) = setup(&numbered(100), cx);
    peek(&editor, cx, vec![here(1, 0, 4), here(5, 0, 4)]);
    let shown = |id: &'static str, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.peek_sash_hover_for_tests(id, cx))
    };
    for id in ["peek-sash-height", "peek-sash-split"] {
        let at = cx.debug_bounds(id).unwrap().center();
        cx.simulate_mouse_move(at, None, Modifiers::default());
        draw(cx);
        assert!(!shown(id, cx), "{id}");
        cx.executor().advance_clock(Duration::from_millis(299));
        draw(cx);
        assert!(!shown(id, cx), "{id}");
        cx.executor().advance_clock(Duration::from_millis(1));
        draw(cx);
        assert!(shown(id, cx), "{id}");
        // Leaving hides it at once.
        cx.simulate_mouse_move(point(px(80.), px(5.)), None, Modifiers::default());
        draw(cx);
        assert!(!shown(id, cx), "{id}");
        // Leaving before the delay cancels it.
        cx.simulate_mouse_move(at, None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.simulate_mouse_move(point(px(80.), px(5.)), None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(200));
        draw(cx);
        assert!(!shown(id, cx), "{id}");
    }
}
