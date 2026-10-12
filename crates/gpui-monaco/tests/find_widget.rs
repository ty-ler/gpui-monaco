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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn selections(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Vec<(u32, u32, u32, u32)> {
    editor.read_with(cx, |e, _| {
        e.selections()
            .iter()
            .map(|s| (s.start().row, s.start().column, s.end().row, s.end().column))
            .collect()
    })
}

fn editor_focused(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
}

/// Monaco `editor.action.selectAllMatches`: Alt+Enter works from the
/// editor while find shows, keeps the widget and focuses the editor.
#[gpui::test]
fn alt_enter_selects_all_matches_from_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar foo\nfoo", cx);
    // Without the widget (its precondition), nothing happens.
    cx.simulate_keystrokes("alt-enter");
    assert_eq!(selections(&editor, cx), vec![(0, 0, 0, 0)]);

    cx.simulate_keystrokes("secondary-f");
    type_query("foo", cx);
    // From the find input.
    cx.simulate_keystrokes("alt-enter");
    cx.run_until_parked();
    assert_eq!(selections(&editor, cx).len(), 3);
    assert!(editor.read_with(cx, |e, _| e.is_find_visible()));
    assert!(editor_focused(&editor, cx));
    // And from the editor.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(
                text::PointUtf16::new(0, 5),
            )],
            cx,
        )
    });
    cx.simulate_keystrokes("alt-enter");
    assert_eq!(selections(&editor, cx).len(), 3);
    assert!(editor.read_with(cx, |e, _| e.is_find_visible()));
}

/// Monaco's multicursor commands have `editorFocus`: from the find input,
/// Cmd/Ctrl+D adds the next match of the widget's string in the editor.
#[gpui::test]
fn cmd_d_in_the_find_input_acts_on_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a.c abc a.c\nxyz", cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(
                text::PointUtf16::new(1, 0),
            )],
            cx,
        )
    });
    cx.simulate_keystrokes("secondary-f");
    type_query("a.c", cx);
    let before = selections(&editor, cx);
    cx.simulate_keystrokes("secondary-d");
    cx.run_until_parked();
    let after = selections(&editor, cx);
    assert_eq!(after.len(), before.len() + 1, "{after:?}");
    // The input keeps its text and the focus.
    assert_eq!(query(&editor, cx), "a.c");
    assert!(!editor_focused(&editor, cx));

    // Cmd/Ctrl+Shift+L: a non-regex search string is matched literally,
    // not as a regex (`abc` isn't selected).
    cx.simulate_keystrokes("secondary-shift-l");
    cx.run_until_parked();
    assert_eq!(selections(&editor, cx), vec![(0, 0, 0, 3), (0, 8, 0, 11)]);
}

/// Monaco `selectAll`: the find widget's string is used only as a regex;
/// otherwise the selection is what's searched for.
#[gpui::test]
fn select_all_occurrences_uses_the_selection_unless_find_is_a_regex(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar foo bar", cx);
    cx.simulate_keystrokes("secondary-f");
    type_query("foo", cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection {
                anchor: text::PointUtf16::new(0, 4),
                head: text::PointUtf16::new(0, 7),
            }],
            cx,
        );
        e.select_all_occurrences(cx);
    });
    assert_eq!(selections(&editor, cx), vec![(0, 4, 0, 7), (0, 12, 0, 15)]);
}

/// Monaco `_beginSessionIfNeeded`: Cmd/Ctrl+D makes the word the search
/// string (whole word and match case while the session lasts), so F3
/// finds it; another selection ends the overrides.
#[gpui::test]
fn cmd_d_seeds_the_find_widget(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo foobar Foo foo", cx);
    cx.simulate_keystrokes("secondary-d");
    let options = editor.read_with(cx, |e, _| e.find_options().clone());
    assert_eq!(options.query, "foo");
    assert!(options.whole_word && options.match_case && !options.is_regex);
    // Another selection ends the session: the toggles are their own again.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(
                text::PointUtf16::new(0, 1),
            )],
            cx,
        )
    });
    let options = editor.read_with(cx, |e, _| e.find_options().clone());
    assert_eq!(options.query, "foo");
    assert!(!options.whole_word && !options.match_case);
    // F3 finds the new search string (opening find resets the overrides,
    // as in Monaco).
    cx.simulate_keystrokes("secondary-d f3");
    cx.run_until_parked();
    assert_eq!(selections(&editor, cx), vec![(0, 4, 0, 7)]);
    assert_eq!(query(&editor, cx), "foo");
}

/// Monaco's `.find-widget` slides in from `translateY(calc(-100% - 10px))`
/// over 200ms, linearly, to 4px (`margin-top`) below the editor's top.
#[gpui::test]
fn the_find_widget_slides_in_to_4px_below_the_top(cx: &mut TestAppContext) {
    let (_editor, cx) = setup("a\nb", cx);
    cx.simulate_keystrokes("secondary-f");
    draw(cx);
    let top = |cx: &mut VisualTestContext| {
        cx.debug_bounds("find-widget")
            .expect("the find widget")
            .origin
            .y
    };
    // 4px, less the 33px widget and its 10px shadow.
    assert_eq!(top(cx), gpui::px(-39.));
    cx.executor().advance_clock(Duration::from_millis(100));
    draw(cx);
    assert_eq!(top(cx), gpui::px(-17.5));
    cx.executor().advance_clock(Duration::from_millis(100));
    draw(cx);
    assert_eq!(top(cx), gpui::px(4.));
    cx.executor().advance_clock(Duration::from_millis(100));
    draw(cx);
    assert_eq!(top(cx), gpui::px(4.));
}

/// Monaco's `.monaco-reduce-motion` makes the transition 0ms: the widget
/// shows in place at once.
#[gpui::test]
fn reduced_motion_shows_the_find_widget_in_place(cx: &mut TestAppContext) {
    let (_editor, cx) = setup("a\nb", cx);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    cx.simulate_keystrokes("secondary-f");
    draw(cx);
    let bounds = cx.debug_bounds("find-widget").expect("the find widget");
    assert_eq!(bounds.origin.y, gpui::px(4.));
}

/// Monaco `_getHeight`: the view zone is the widget's height as laid out
/// and measured (33px, 62px with replace).
#[gpui::test]
fn the_view_zone_is_the_measured_widget_height(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb", cx);
    cx.simulate_keystrokes("secondary-f");
    draw(cx);
    let zone =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.find_zone_height_for_tests());
    assert_eq!(zone(cx), Some(gpui::px(33.)));
    editor.update(cx, |e, cx| e.set_find_replace_visible(true, cx));
    draw(cx);
    assert_eq!(zone(cx), Some(gpui::px(62.)));
    // A line break in the find input grows it by a 19px line.
    cx.simulate_keystrokes("secondary-f ctrl-enter");
    draw(cx);
    assert_eq!(zone(cx), Some(gpui::px(81.)));
}

/// Monaco `_tryUpdateWidgetWidth`: 419px, then reduced, narrow (257px) and
/// collapsed (170px) as the editor narrows; never past its left edge.
#[gpui::test]
fn narrow_reduced_and_collapsed_layouts(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a", cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.minimap.enabled = false;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("secondary-f");
    let layout = |width: f32, cx: &mut VisualTestContext| {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(400.)));
        draw(cx);
        editor
            .read_with(cx, |e, cx| e.find_widget_layout_for_tests(cx))
            .unwrap()
    };
    let (width, _, reduced, narrow, collapsed) = layout(1000., cx);
    assert_eq!(
        (width, reduced, narrow, collapsed),
        (gpui::px(419.), false, false, false)
    );
    let (_, _, reduced, narrow, collapsed) = layout(440., cx);
    assert!(reduced && !narrow && !collapsed);
    let (width, _, reduced, narrow, collapsed) = layout(370., cx);
    assert!(reduced && narrow && !collapsed);
    assert_eq!(width, gpui::px(257.));
    let (width, input, _, _, collapsed) = layout(300., cx);
    assert!(collapsed);
    assert_eq!(width, gpui::px(170.));
    assert!(input > gpui::px(0.));
    // Narrower than the widget: it shrinks to fit.
    let (width, _, _, _, _) = layout(150., cx);
    assert!(width <= gpui::px(150. - 28.), "{width:?}");
}

/// Monaco's titles with keybindings, and `_updateButtons`' disabled
/// states.
#[gpui::test]
fn titles_and_disabled_states(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo foo", cx);
    cx.simulate_keystrokes("secondary-f");
    draw(cx);
    let title = |id: &str, cx: &mut VisualTestContext| {
        cx.update(|window, cx| editor.read(cx).find_widget_title_for_tests(id, window, cx))
            .unwrap()
    };
    assert!(
        title("match-case", cx).starts_with("Match Case ("),
        "{}",
        title("match-case", cx)
    );
    assert!(title("next", cx).starts_with("Next Match ("));
    assert_eq!(title("toggle-replace", cx), "Toggle Replace");
    let enabled = |id: &str, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.find_widget_control_enabled_for_tests(id, cx))
    };
    cx.simulate_keystrokes("secondary-a backspace");
    cx.run_until_parked();
    // No search string: no navigation, no find in selection without a
    // selection.
    assert!(!enabled("previous", cx) && !enabled("next", cx));
    assert!(!enabled("in-selection", cx));
    type_query("foo", cx);
    assert!(enabled("previous", cx) && enabled("next", cx));
    // Replace buttons need the replace input and a search string.
    assert!(!enabled("replace", cx));
    editor.update(cx, |e, cx| e.set_find_replace_visible(true, cx));
    cx.simulate_keystrokes("secondary-f");
    draw(cx);
    assert!(enabled("replace", cx) && enabled("replace-all", cx));
}

#[gpui::test]
fn read_only_hides_the_replace_toggle(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo", cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("secondary-f");
    draw(cx);
    assert!(!editor.read_with(cx, |e, cx| {
        e.find_widget_control_enabled_for_tests("toggle-replace", cx)
    }));
}

/// Monaco's Tab order: find input → match case (or the replace input),
/// through the toggles and buttons; Shift+Tab back.
#[gpui::test]
fn tab_order(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo foo", cx);
    cx.simulate_keystrokes("secondary-f");
    type_query("foo", cx);
    draw(cx);
    let focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            editor
                .read(cx)
                .find_widget_focused_control_for_tests(window, cx)
        })
    };
    cx.simulate_keystrokes("tab");
    assert_eq!(focused(cx), Some("match-case"));
    cx.simulate_keystrokes("tab tab tab");
    assert_eq!(focused(cx), Some("previous"));
    // Space presses the focused button (from the first match, previous
    // wraps to the last).
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert_eq!(selections(&editor, cx), vec![(0, 4, 0, 7)]);
    // Shift+Tab walks back to the find input.
    cx.simulate_keystrokes("shift-tab shift-tab shift-tab shift-tab");
    assert_eq!(focused(cx), None);
    assert!(cx.update(|window, cx| editor.read(cx).find_widget_focus_for_tests(window, cx).0));

    // With replace: find input → replace input → match case.
    cx.update(|window, cx| editor.update(cx, |e, cx| e.open_find(true, window, cx)));
    cx.update(|window, cx| editor.update(cx, |e, cx| e.open_find(false, window, cx)));
    draw(cx);
    cx.simulate_keystrokes("tab");
    assert!(cx.update(|window, cx| editor.read(cx).find_widget_focus_for_tests(window, cx).1));
    cx.simulate_keystrokes("tab");
    assert_eq!(focused(cx), Some("match-case"));
}

/// Monaco's inputs: Cmd/Ctrl+Down focuses the editor; Shift+Enter is
/// Find Previous in the find input only.
#[gpui::test]
fn input_keys(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo foo foo", cx);
    cx.simulate_keystrokes("secondary-f");
    type_query("foo", cx);
    cx.simulate_keystrokes("enter enter");
    assert_eq!(selections(&editor, cx), vec![(0, 8, 0, 11)]);
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(selections(&editor, cx), vec![(0, 4, 0, 7)]);
    cx.update(|window, cx| editor.update(cx, |e, cx| e.open_find(true, window, cx)));
    cx.run_until_parked();
    // In the replace input it's no command: the textarea's line break.
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    assert_eq!(selections(&editor, cx), vec![(0, 4, 0, 7)]);
    assert_eq!(
        editor.read_with(cx, |e, _| e.find_options().replace.clone()),
        "\n"
    );
    cx.simulate_keystrokes("secondary-down");
    assert!(editor_focused(&editor, cx));
    assert!(editor.read_with(cx, |e, _| e.is_find_visible()));
}

/// Monaco's find widget reads each new count to screen readers.
#[gpui::test]
fn screen_reader_status(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo\nbar foo", cx);
    cx.simulate_keystrokes("secondary-f");
    type_query("foo", cx);
    let last = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| e.last_announcement_for_tests())
            .map(|s| s.to_string())
    };
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        last(cx).as_deref(),
        Some("bar foo, 2 of 2 found for 'foo', at 2:5")
    );
    type_query("zzz", cx);
    assert_eq!(last(cx).as_deref(), Some("No results found for 'zzz'"));
}

/// Monaco captures the pointer on the sash: `ew-resize` over the whole
/// window, off the sash and over the text, until the release; Escape
/// closing the widget mid-drag gives the pointers back.
#[gpui::test]
fn dragging_the_sash_keeps_its_pointer_everywhere(cx: &mut TestAppContext) {
    use gpui::{CursorStyle, Modifiers, MouseButton, point, px};
    let (editor, cx) = setup("alpha beta\n", cx);
    let paint = |cx: &mut VisualTestContext| {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    };
    let captured =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.captured_cursor_for_tests(cx));
    cx.simulate_keystrokes("secondary-f");
    // After the slide-in.
    cx.executor().advance_clock(Duration::from_millis(200));
    paint(cx);
    let sash = cx.debug_bounds("find-sash").expect("the sash");
    let from = sash.center();
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeLeftRight));
    for to in [
        from - point(px(80.), px(0.)),
        point(px(20.), px(200.)),
        point(px(-30.), px(-30.)),
    ] {
        cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        paint(cx);
        assert_eq!(captured(cx), Some(CursorStyle::ResizeLeftRight));
    }
    cx.simulate_mouse_up(
        point(px(20.), px(200.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), None);

    let sash = cx.debug_bounds("find-sash").expect("the sash");
    cx.simulate_mouse_move(sash.center(), None, Modifiers::default());
    cx.simulate_mouse_down(sash.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(px(20.), px(200.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeLeftRight));
    cx.simulate_keystrokes("escape");
    paint(cx);
    assert_eq!(captured(cx), None);
}
