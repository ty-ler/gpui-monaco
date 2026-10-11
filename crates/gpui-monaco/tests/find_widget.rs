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

fn selection(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (u32, u32, u32, u32) {
    editor.read_with(cx, |e, _| {
        let s = &e.selections()[0];
        (s.start().row, s.start().column, s.end().row, s.end().column)
    })
}

#[gpui::test]
fn find_on_type_off_searches_on_enter(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar foo", cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.find.find_on_type = false;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    // Seeded from the word at the cursor.
    assert_eq!(query(&editor, cx), "foo");
    type_query("bar", cx);
    // Not searched while typing (Monaco `find.findOnType: false`).
    assert_eq!(query(&editor, cx), "foo");
    cx.simulate_keystrokes("enter");
    assert_eq!(query(&editor, cx), "bar");
    assert_eq!(selection(&editor, cx), (0, 4, 0, 7));

    // On (the default), typing searches.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.find.find_on_type = true;
        e.update_options(options, cx);
    });
    type_query("fo", cx);
    assert_eq!(query(&editor, cx), "fo");
}

#[gpui::test]
fn find_with_args(cx: &mut TestAppContext) {
    use gpui_monaco::actions::FindWithArgs;
    let (editor, cx) = setup("one Two two\nthree", cx);
    // Move to the second line's end, off any word that would seed it.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(
                text::PointUtf16::new(1, 5),
            )],
            cx,
        )
    });
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger_with_payload(
                "editor.actions.findWithArgs",
                serde_json::json!({
                    "searchString": "",
                    "replaceString": "2",
                    "isCaseSensitive": true,
                    "matchWholeWord": true,
                }),
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    // An empty search string is seeded from the word at the cursor first,
    // as in Monaco.
    let options = editor.read_with(cx, |e, _| e.find_options().clone());
    assert_eq!(options.query, "three");
    assert_eq!(options.replace, "2");
    assert!(options.match_case && options.whole_word && !options.is_regex);
    assert!(editor.read_with(cx, |e, _| e.is_find_visible()));
    cx.simulate_keystrokes("escape");

    // Otherwise the given search string is used, and the replace input
    // shows only with a replace string.
    let editor_handle = editor.clone();
    cx.update(|window, cx| {
        editor_handle.update(cx, |e, cx| {
            e.find_with_args(
                &FindWithArgs {
                    search_string: Some("two".into()),
                    is_case_sensitive: Some(false),
                    ..Default::default()
                },
                window,
                cx,
            )
        })
    });
    cx.run_until_parked();
    let options = editor.read_with(cx, |e, _| e.find_options().clone());
    assert_eq!(options.query, "two");
    assert_eq!(options.replace, "2");
    assert!(!options.match_case && options.whole_word);
    assert_eq!(editor.read_with(cx, |e, _| e.find_matches_count()), 2);

    // A bad payload fails.
    let result = cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger_with_payload(
                "editor.actions.findWithArgs",
                serde_json::json!({ "isRegex": "yes" }),
                window,
                cx,
            )
        })
    });
    assert!(result.is_err());
}

#[gpui::test]
fn go_to_match(cx: &mut TestAppContext) {
    let (editor, cx) = setup("ab ab\nab", cx);
    let run = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.run_action("editor.action.goToMatchFindAction", window, cx)
            })
        })
    };
    // Needs the find widget.
    assert!(!run(&editor, cx));
    cx.simulate_keystrokes("secondary-f");
    type_query("ab", cx);
    assert!(run(&editor, cx));
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_quick_access_open()));
    let rows = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx))
    };
    assert_eq!(
        rows(cx),
        ["> Type a number to go to a specific match (between 1 and 3)"]
    );
    // Typing a number selects that match; negative counts from the end.
    cx.simulate_input("2");
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), (0, 3, 0, 5));
    cx.simulate_keystrokes("backspace");
    cx.simulate_input("-1");
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), (1, 0, 1, 2));
    cx.simulate_keystrokes("backspace backspace");
    cx.simulate_input("9");
    cx.run_until_parked();
    assert_eq!(rows(cx), ["> Please type a number between 1 and 3"]);
    cx.simulate_keystrokes("backspace");
    cx.simulate_input("1");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_quick_access_open()));
    assert_eq!(selection(&editor, cx), (0, 0, 0, 2));
}
