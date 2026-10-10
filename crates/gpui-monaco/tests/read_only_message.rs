//! The placeholder (Monaco `placeholderText`) and the read-only message
//! (Monaco `readOnlyMessage`, `MessageController`).

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::{CodeEditor, CodeEditorEvent};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn set_options(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut gpui_monaco::options::EditorOptions),
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        f(&mut options);
        e.update_options(options, cx);
    });
}

#[gpui::test]
fn placeholder_shows_while_empty(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    editor.read_with(cx, |e, _| assert!(!e.is_placeholder_visible()));
    set_options(&editor, cx, |o| o.placeholder = Some("Type here".into()));
    editor.read_with(cx, |e, _| assert!(e.is_placeholder_visible()));
    cx.simulate_input("a");
    editor.read_with(cx, |e, _| assert!(!e.is_placeholder_visible()));
}

#[gpui::test]
fn typing_into_read_only_shows_the_message(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello", cx);
    let attempts = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = attempts.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if matches!(event, CodeEditorEvent::ReadOnlyEditAttempt) {
                counter.set(counter.get() + 1);
            }
        })
    });
    set_options(&editor, cx, |o| o.read_only = true);
    cx.simulate_input("x");
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| {
        assert_eq!(e.model().read(cx).text(), "hello");
        assert_eq!(
            e.message_text().as_deref(),
            Some("Cannot edit in read-only editor")
        );
    });
    assert_eq!(attempts.get(), 1);

    // Editing commands too, with a custom markdown message.
    set_options(&editor, cx, |o| {
        o.read_only_message = Some("**Locked** file".into())
    });
    cx.simulate_keystrokes("backspace");
    editor.read_with(cx, |e, _| {
        assert_eq!(e.message_text().as_deref(), Some("**Locked** file"))
    });
    assert_eq!(attempts.get(), 2);

    // Escape closes it.
    cx.simulate_keystrokes("escape");
    editor.read_with(cx, |e, _| assert!(!e.is_message_visible()));
}
