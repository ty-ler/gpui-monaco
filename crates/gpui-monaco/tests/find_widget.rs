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

#[gpui::test]
fn multi_line_search_and_replace_inputs(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo\nbar foo\nbaz", cx);
    cx.simulate_keystrokes("secondary-f");
    type_query("foo", cx);
    let zone = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| e.find_zone_height_for_tests())
            .unwrap()
    };
    let one_line = zone(cx);
    // Monaco: Ctrl+Enter inserts a line break in the find input.
    cx.simulate_keystrokes("ctrl-enter");
    cx.simulate_input("bar");
    assert_eq!(query(&editor, cx), "foo\nbar");
    assert_eq!(editor.read_with(cx, |e, _| e.find_matches_count()), 1);
    // The input grows, and the space above the text with it.
    assert!(zone(cx) > one_line);

    // Up moves in the input (Monaco `stopPropagationForMultiLineUpwards`)
    // until it's at the start; then it walks the history.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.simulate_keystrokes("up up");
    assert_eq!(query(&editor, cx), "foo\nbar");
    cx.simulate_keystrokes("up");
    assert_eq!(query(&editor, cx), "foo");
    cx.simulate_keystrokes("down");
    assert_eq!(query(&editor, cx), "foo\nbar");

    // The replace input too.
    cx.update(|window, cx| editor.update(cx, |e, cx| e.open_find(true, window, cx)));
    cx.simulate_input("x");
    cx.simulate_keystrokes("ctrl-enter");
    cx.simulate_input("y");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.find_options().replace.clone()),
        "x\ny"
    );
    editor.update(cx, |e, cx| e.replace_all(cx));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "x\ny foo\nbaz"
    );
}

#[gpui::test]
fn pasting_line_breaks_into_the_find_input(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb a", cx);
    cx.simulate_keystrokes("secondary-f");
    cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("a\r\nb".into())));
    cx.simulate_keystrokes("secondary-a secondary-v");
    // Kept, as in Monaco's textarea (line breaks normalized).
    assert_eq!(query(&editor, cx), "a\nb");
    assert_eq!(editor.read_with(cx, |e, _| e.find_matches_count()), 1);
    // Text without line breaks pastes as usual.
    cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("b".into())));
    cx.simulate_keystrokes("secondary-a secondary-v");
    assert_eq!(query(&editor, cx), "b");
}

/// Moves the focus out of `editor` and back (focus events need an active
/// window and a frame).
fn refocus(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    let draw = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        cx.run_until_parked();
    };
    cx.update(|window, _| window.activate_window());
    draw(cx);
    cx.update(|window, cx| window.blur(cx));
    draw(cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
}

#[gpui::test]
fn global_find_clipboard(cx: &mut TestAppContext) {
    let (editor, cx) = setup("alpha beta\n", cx);
    let set_global = |editor: &Entity<CodeEditor>, on: bool, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            let mut options = e.options().clone();
            options.find.global_find_clipboard = on;
            e.update_options(options, cx);
        })
    };
    set_global(&editor, true, cx);
    // Another editor sharing the find clipboard.
    let other = cx.update(|window, cx| {
        cx.new(|cx| {
            let model = cx.new(|_| TextModel::new("x"));
            CodeEditor::new(model, window, cx)
        })
    });
    set_global(&other, true, cx);
    let set_query = |editor: &Entity<CodeEditor>, query: &str, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            let mut options = e.find_options().clone();
            options.query = query.into();
            e.set_find_options(options, cx);
        })
    };
    // Finding writes the search string to the find clipboard.
    set_query(&other, "beta", cx);
    #[cfg(target_os = "macos")]
    assert_eq!(
        cx.update(|_, cx| cx.read_from_find_pasteboard())
            .and_then(|item| item.text())
            .as_deref(),
        Some("beta")
    );
    // Opening find with nothing to seed from takes it.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(
                text::PointUtf16::new(1, 0),
            )],
            cx,
        )
    });
    cx.simulate_keystrokes("secondary-f");
    assert_eq!(query(&editor, cx), "beta");

    // Focusing the editor picks up a search string written meanwhile.
    set_query(&other, "alpha", cx);
    refocus(&editor, cx);
    assert_eq!(query(&editor, cx), "alpha");

    // Off: neither read nor written.
    set_global(&editor, false, cx);
    set_query(&other, "gamma", cx);
    refocus(&editor, cx);
    assert_eq!(query(&editor, cx), "alpha");
}
