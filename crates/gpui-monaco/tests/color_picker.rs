//! The color picker (Monaco `colorPicker`): in the hover over a color and
//! standalone.

use std::time::Duration;

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::actions::*;
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("css"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    (editor, cx)
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(1100));
        cx.run_until_parked();
    }
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn state(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Option<(String, Option<String>, std::ops::Range<PointUtf16>)> {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.color_picker_state_for_tests(cx))
}

fn picker(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Entity<gpui_monaco::ColorPickerWidget> {
    editor
        .read_with(cx, |e, cx| e.color_picker_for_tests(cx))
        .expect("a color picker")
}

fn put_cursor(editor: &Entity<CodeEditor>, row: u32, column: u32, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(row, column))], cx)
    });
}

#[gpui::test]
fn hover_picker_rewrites_the_color(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: #ff0000; }", cx);
    put_cursor(&editor, 0, 13, cx);
    cx.dispatch_action(ShowHover);
    let range = PointUtf16::new(0, 11)..PointUtf16::new(0, 18);
    // The default presentations are rgb, hsl and hex; the one written
    // like the text is chosen.
    assert_eq!(
        state(&editor, cx),
        Some(("#ff0000ff".to_string(), Some("#ff0000".to_string()), range))
    );

    // Letting go of a new color writes it in the chosen presentation, and
    // the hover stays.
    let widget = picker(&editor, cx);
    widget.update(cx, |w, cx| w.pick_css_for_tests("#00ff00", cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a { color: #00ff00; }");
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));

    // Clicking the header switches to the next presentation.
    widget.update(cx, |w, cx| w.click_header_for_tests(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a { color: rgb(0, 255, 0); }");
    let (_, label, range) = state(&editor, cx).unwrap();
    assert_eq!(label.as_deref(), Some("rgb(0, 255, 0)"));
    assert_eq!(range, PointUtf16::new(0, 11)..PointUtf16::new(0, 25));

    // One undo step per change.
    cx.dispatch_action(Undo);
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a { color: #00ff00; }");
}

#[gpui::test]
fn hover_without_color_has_no_picker(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: red; }", cx);
    put_cursor(&editor, 0, 1, cx);
    cx.dispatch_action(ShowHover);
    assert_eq!(state(&editor, cx), None);
}

#[gpui::test]
fn standalone_picker_inserts_and_replaces(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: ; }\nb { color: #00f; }", cx);
    put_cursor(&editor, 0, 11, cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
    // Not on a color: black, inserted at the cursor.
    let (color, _, _) = state(&editor, cx).unwrap();
    assert_eq!(color, "#000000ff");
    let widget = picker(&editor, cx);
    widget.update(cx, |w, cx| w.pick_css_for_tests("#ff0000", cx));
    widget.update(cx, |w, cx| w.insert_for_tests(cx));
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
    assert_eq!(
        text(&editor, cx),
        "a { color: rgb(255, 0, 0); }\nb { color: #00f; }"
    );

    // On a color: it's selected and replaced. `#0000ff` doesn't start
    // with `#00f`, so Monaco's guess falls back to the first presentation.
    settle(cx);
    put_cursor(&editor, 1, 12, cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    cx.run_until_parked();
    let (color, label, range) = state(&editor, cx).unwrap();
    assert_eq!(color, "#0000ffff");
    assert_eq!(label.as_deref(), Some("rgb(0, 0, 255)"));
    assert_eq!(range, PointUtf16::new(1, 11)..PointUtf16::new(1, 15));
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!(
        (selection.start(), selection.end()),
        (range.start, range.end)
    );
    let widget = picker(&editor, cx);
    widget.update(cx, |w, cx| w.pick_css_for_tests("#ffffff", cx));
    widget.update(cx, |w, cx| w.insert_for_tests(cx));
    cx.run_until_parked();
    assert_eq!(
        text(&editor, cx),
        "a { color: rgb(255, 0, 0); }\nb { color: rgb(255, 255, 255); }"
    );
}

#[gpui::test]
fn escape_hides_the_standalone_picker(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: ; }", cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
}
