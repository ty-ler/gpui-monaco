//! gpui-monaco inside gpui-component's `Root`, with both libraries'
//! keymaps loaded: typing, editor keybindings and focus still work.

use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use gpui_component::Root;
use gpui_monaco::{CodeEditor, TextModel};

fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_monaco::init(cx);
    });
    let mut editor = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("fn main() {}\n"));
        let e = cx.new(|cx| CodeEditor::new(model, window, cx));
        window.focus(&e.read(cx).focus_handle(cx), cx);
        editor = Some(e.clone());
        Root::new(e, window, cx)
    });
    cx.run_until_parked();
    (editor.unwrap(), cx)
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

#[gpui::test]
fn typing_and_editor_keys_work_inside_root(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    assert!(cx.update(|window, cx| editor.read(cx).focus_handle(cx).is_focused(window)));
    // Text input goes through gpui-component's Root to the editor.
    cx.simulate_input("hello");
    assert_eq!(text(&editor, cx), "hellofn main() {}\n");
    // The editor's own keybindings still win: undo, select all.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&editor, cx), "fn main() {}\n");
    cx.simulate_keystrokes("cmd-a backspace");
    assert_eq!(text(&editor, cx), "");
}
