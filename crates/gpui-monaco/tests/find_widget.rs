//! The find widget (Monaco `findWidget`): history, the extra space on top,
//! and its states.

use std::time::Duration;

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::{CodeEditor, TextModel};

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

fn query(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    cx.run_until_parked();
    editor.read_with(cx, |e, _| e.find_options().query.clone())
}

/// Types `text` over the find input's content and lets it join the history.
fn type_query(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(text);
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
}

#[gpui::test]
fn up_and_down_walk_the_history(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar baz", cx);
    cx.simulate_keystrokes("secondary-f");
    type_query("foo", cx);
    type_query("bar", cx);
    type_query("baz", cx);
    cx.simulate_keystrokes("up");
    assert_eq!(query(&editor, cx), "bar");
    cx.simulate_keystrokes("up");
    assert_eq!(query(&editor, cx), "foo");
    // The oldest stays.
    cx.simulate_keystrokes("up");
    assert_eq!(query(&editor, cx), "foo");
    cx.simulate_keystrokes("down down");
    assert_eq!(query(&editor, cx), "baz");
    // Past the newest: empty.
    cx.simulate_keystrokes("down");
    assert_eq!(query(&editor, cx), "");

    // The history outlives the widget (Monaco's is global).
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("secondary-f");
    cx.simulate_keystrokes("secondary-a backspace up");
    assert_eq!(query(&editor, cx), "baz");
}

#[gpui::test]
fn extra_space_on_top_keeps_the_text_in_place(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb\nc", cx);
    let scroll = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |e, _| e.scroll_position().y)
    };
    assert_eq!(scroll(&editor, cx), gpui::px(0.));
    cx.simulate_keystrokes("secondary-f");
    // A zone above line 1, scrolled past.
    let shown = scroll(&editor, cx);
    assert!(shown > gpui::px(0.), "{shown:?}");
    cx.simulate_keystrokes("escape");
    assert_eq!(scroll(&editor, cx), gpui::px(0.));

    // Off: no space.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.find.add_extra_space_on_top = false;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("secondary-f");
    assert_eq!(scroll(&editor, cx), gpui::px(0.));
}

#[gpui::test]
fn no_results_and_invalid_regex(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", cx);
    cx.simulate_keystrokes("secondary-f");
    type_query("zzz", cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.find_status()),
        Some("No results".into())
    );
    editor.update(cx, |e, cx| {
        let mut options = e.find_options().clone();
        options.is_regex = true;
        options.query = "(".into();
        e.set_find_options(options, cx);
    });
    assert!(editor.read_with(cx, |e, _| e.is_find_query_invalid()));
}
