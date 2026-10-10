//! Unusual line terminators (Monaco `UnusualLineTerminatorsDetector`).

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::CodeEditor;
use gpui_monaco::options::UnusualLineTerminators;

fn setup<'a>(
    text: &str,
    mode: UnusualLineTerminators,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = CodeEditor::with_text(&text, window, cx);
        let mut options = editor.options().clone();
        options.unusual_line_terminators = mode;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

#[gpui::test]
fn auto_removes_them(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\u{2028}b\u{2029}c", UnusualLineTerminators::Auto, cx);
    assert_eq!(text(&editor, cx), "abc");
    // Typed (or pasted) ones too.
    cx.simulate_input("\u{2028}");
    paint(cx);
    assert_eq!(text(&editor, cx), "abc");
}

#[gpui::test]
fn off_keeps_them(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\u{2028}b", UnusualLineTerminators::Off, cx);
    assert_eq!(text(&editor, cx), "a\u{2028}b");
}

#[gpui::test]
fn prompt_asks_first(cx: &mut TestAppContext) {
    // Switching the option checks again.
    let (editor, cx) = setup("a\u{2028}b", UnusualLineTerminators::Off, cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.unusual_line_terminators = UnusualLineTerminators::Prompt;
        e.update_options(options, cx);
    });
    paint(cx);
    let rows = editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx));
    assert_eq!(rows, vec!["> Remove Unusual Line Terminators", "Ignore"]);
    cx.simulate_keystrokes("enter");
    paint(cx);
    assert_eq!(text(&editor, cx), "ab");
    assert!(editor.read_with(cx, |e, _| !e.is_quick_access_open()));

    // Ignoring keeps them, and doesn't ask again for this model.
    cx.simulate_input("\u{2029}");
    paint(cx);
    cx.simulate_keystrokes("down enter");
    paint(cx);
    assert_eq!(text(&editor, cx), "\u{2029}ab");
    cx.simulate_input("\u{2029}");
    paint(cx);
    assert!(editor.read_with(cx, |e, _| !e.is_quick_access_open()));
}
