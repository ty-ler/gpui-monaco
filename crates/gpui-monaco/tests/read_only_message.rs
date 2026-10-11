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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn widgets(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, _| e.widget_bounds_for_tests().1.len())
}

/// Monaco's `.anchor`: above its line the arrow is at the bottom; on the
/// first line, with no room above, the message goes below with the arrow
/// on top. Closing fades it out over 100ms.
#[gpui::test]
fn the_message_points_at_its_line_and_fades_out(cx: &mut TestAppContext) {
    use gpui_monaco::text::PointUtf16;
    let (editor, cx) = setup(&"x\n".repeat(40), cx);
    editor.update(cx, |e, cx| {
        e.show_message("Above", PointUtf16::new(10, 0), cx)
    });
    draw(cx);
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.message_below_for_tests(cx)),
        Some(false)
    );
    editor.update(cx, |e, cx| {
        e.show_message("Below", PointUtf16::new(0, 0), cx)
    });
    draw(cx);
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.message_below_for_tests(cx)),
        Some(true)
    );
    let shown = widgets(&editor, cx);
    editor.update(cx, |e, cx| e.close_message(cx));
    assert!(!editor.read_with(cx, |e, _| e.is_message_visible()));
    draw(cx);
    // Still fading out.
    assert_eq!(widgets(&editor, cx), shown);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(150));
    draw(cx);
    assert_eq!(widgets(&editor, cx), shown - 1);
}

/// Monaco `revealLinesInCenterIfOutsideViewport`: a message out of view
/// scrolls to it.
#[gpui::test]
fn the_message_reveals_its_line(cx: &mut TestAppContext) {
    use gpui_monaco::text::PointUtf16;
    let (editor, cx) = setup(&"x\n".repeat(400), cx);
    editor.update(cx, |e, cx| {
        e.show_message("Here", PointUtf16::new(300, 0), cx)
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.scroll_position().y) > gpui::px(1000.));
}

/// Moves the focus out of the editor.
fn blur(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.blur(cx));
    draw(cx);
}

/// Monaco keeps the message open when the editor loses focus with the
/// mouse over the message.
#[gpui::test]
fn the_message_stays_on_blur_while_the_mouse_is_over_it(cx: &mut TestAppContext) {
    use gpui_monaco::text::PointUtf16;
    let (editor, cx) = setup("abc", cx);
    // Focus events need an active window.
    cx.update(|window, cx| {
        window.activate_window();
        window.focus(&editor.focus_handle(cx), cx);
    });
    draw(cx);
    editor.update(cx, |e, cx| e.show_message("Hi", PointUtf16::new(0, 0), cx));
    editor.update(cx, |e, _| e.set_message_mouse_over_for_tests(true));
    blur(cx);
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    editor.update(cx, |e, _| e.set_message_mouse_over_for_tests(false));
    blur(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_message_visible()));
}
