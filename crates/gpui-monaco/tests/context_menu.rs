//! The context menu (Monaco `ContextMenuController`): Shift+F10 and
//! right-click, groups, keyboard navigation and host actions.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::{CodeEditor, EditorActionDescriptor, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.context_menu_rows_for_tests(cx))
}

#[gpui::test]
fn shift_f10_shows_the_default_menu(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar foo", cx);
    cx.simulate_keystrokes("shift-f10");
    // No language providers: no go to definition, rename or formatting,
    // and no symbols to go to (Monaco's `hasDocumentSymbolProvider`).
    assert_eq!(
        rows(&editor, cx),
        vec![
            "Change All Occurrences",
            "---",
            "Cut",
            "Copy",
            "Paste",
            "---",
            "Command Palette",
        ]
    );
    // Keyboard: down to "Change All Occurrences", Enter runs it.
    cx.simulate_keystrokes("down");
    assert_eq!(rows(&editor, cx)[0], "> Change All Occurrences");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 2);
}

#[gpui::test]
fn escape_and_options(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo", cx);
    cx.simulate_keystrokes("shift-f10");
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));

    // Read-only: no editing items.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("shift-f10");
    assert_eq!(rows(&editor, cx), vec!["Copy", "---", "Command Palette"]);
    cx.simulate_keystrokes("escape");

    // Off: nothing.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.context_menu = false;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("shift-f10");
    assert!(rows(&editor, cx).is_empty());
}

#[gpui::test]
fn host_actions_join_their_group(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", cx);
    editor.update(cx, |e, cx| {
        let _ = e.add_action(
            EditorActionDescriptor::new("my.upper", "Make Upper", |editor, _, cx| {
                editor.model().update(cx, |m, cx| m.set_value("ABC", cx));
            })
            .context_menu("1_modification", 0.5),
            cx,
        );
        let _ = e.add_action(
            EditorActionDescriptor::new("my.hidden", "Not In Menu", |_, _, _| {}),
            cx,
        );
    });
    cx.simulate_keystrokes("shift-f10");
    let menu = rows(&editor, cx);
    assert_eq!(menu[0], "Make Upper");
    assert!(!menu.iter().any(|r| r.contains("Not In Menu")));
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "ABC"
    );
}
