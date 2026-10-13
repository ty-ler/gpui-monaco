//! Ported from Monaco's `contrib/multicursor/test/browser/multicursor.test.ts`
//! (MIT). Selections are Monaco 1-based `(startLine, startCol, endLine,
//! endCol)` with the anchor at the start.
//!
//! Skipped: word-wrap and paste-order tests (covered by the cursor core).

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::SelectedTextMatchMode;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, FindOptions};

type Sel = (u32, u32, u32, u32);

fn sel(s: Sel) -> Selection {
    Selection {
        anchor: PointUtf16::from_monaco(s.0, s.1),
        head: PointUtf16::from_monaco(s.2, s.3),
    }
}

fn setup<'a>(
    lines: &[&str],
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = lines.join("\n");
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn set(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, selections: &[Sel]) {
    let selections: Vec<Selection> = selections.iter().map(|s| sel(*s)).collect();
    editor.update(cx, |e, cx| e.set_selections(&selections, cx));
}

fn get(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Sel> {
    editor.read_with(cx, |e, _| {
        e.selections()
            .iter()
            .map(|s| {
                let (a, b) = (s.start().to_monaco(), s.end().to_monaco());
                (a.0, a.1, b.0, b.1)
            })
            .collect()
    })
}

fn add_next(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.add_selection_to_next_find_match(cx));
}

fn match_mode(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    mode: SelectedTextMatchMode,
) {
    update_options(editor, cx, |o| o.selected_text_match_mode = mode);
}

#[gpui::test]
fn issue_8817_select_highlights_keeps_primary(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        &["var x = (3 * 5)", "var y = (3 * 5)", "var z = (3 * 5)"],
        cx,
    );
    set(&editor, cx, &[(2, 9, 2, 16)]);
    editor.update(cx, |e, cx| e.select_all_occurrences(cx));
    assert_eq!(
        get(&editor, cx),
        vec![(2, 9, 2, 16), (1, 9, 1, 16), (3, 9, 3, 16)]
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(get(&editor, cx), vec![(2, 9, 2, 16)]);
}

#[gpui::test]
fn issue_5400_select_all_occurrences_uses_find_regex(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["something", "someething", "someeething", "nothing"], cx);
    set(&editor, cx, &[(1, 1, 1, 1)]);
    cx.simulate_keystrokes("secondary-f");
    editor.update(cx, |e, cx| {
        e.set_find_options(
            FindOptions {
                query: "some+thing".into(),
                is_regex: true,
                ..Default::default()
            },
            cx,
        );
    });
    set(&editor, cx, &[(2, 1, 2, 11)]);
    editor.update(cx, |e, cx| e.select_all_occurrences(cx));
    assert_eq!(
        get(&editor, cx),
        vec![(2, 1, 2, 11), (1, 1, 1, 10), (3, 1, 3, 12)]
    );
}

#[gpui::test]
fn issue_6661_touching_ranges(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["abcabc", "abc", "abcabc"], cx);
    set(&editor, cx, &[(1, 1, 1, 4)]);
    add_next(&editor, cx);
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4), (1, 4, 1, 7)]);
    add_next(&editor, cx);
    add_next(&editor, cx);
    add_next(&editor, cx);
    assert_eq!(
        get(&editor, cx),
        vec![
            (1, 1, 1, 4),
            (1, 4, 1, 7),
            (2, 1, 2, 4),
            (3, 1, 3, 4),
            (3, 4, 3, 7)
        ]
    );
    cx.simulate_input("z");
    assert_eq!(
        get(&editor, cx),
        vec![
            (1, 2, 1, 2),
            (1, 3, 1, 3),
            (2, 2, 2, 2),
            (3, 2, 3, 2),
            (3, 3, 3, 3)
        ]
    );
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "zz\nz\nzz"
    );
}

#[gpui::test]
fn add_next_from_single_collapsed_selection(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["abc pizza", "abc house", "abc bar"], cx);
    set(&editor, cx, &[(1, 2, 1, 2)]);
    add_next(&editor, cx);
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4)]);
    add_next(&editor, cx);
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4), (2, 1, 2, 4)]);
    add_next(&editor, cx);
    assert_eq!(
        get(&editor, cx),
        vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]
    );
    add_next(&editor, cx);
    assert_eq!(
        get(&editor, cx),
        vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]
    );
}

#[gpui::test]
fn add_next_from_two_selections_one_collapsed(cx: &mut TestAppContext) {
    for initial in [[(1, 1, 1, 4), (2, 2, 2, 2)], [(1, 2, 1, 2), (2, 1, 2, 4)]] {
        let (editor, vcx) = setup(&["abc pizza", "abc house", "abc bar"], cx);
        set(&editor, vcx, &initial);
        add_next(&editor, vcx);
        assert_eq!(get(&editor, vcx), vec![(1, 1, 1, 4), (2, 1, 2, 4)]);
        add_next(&editor, vcx);
        assert_eq!(
            get(&editor, vcx),
            vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]
        );
        add_next(&editor, vcx);
        assert_eq!(
            get(&editor, vcx),
            vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]
        );
    }
}

#[gpui::test]
fn add_next_from_all_collapsed_selections(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["abc pizza", "abc house", "abc bar"], cx);
    set(&editor, cx, &[(1, 2, 1, 2), (2, 2, 2, 2), (3, 1, 3, 1)]);
    add_next(&editor, cx);
    assert_eq!(
        get(&editor, cx),
        vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]
    );
    add_next(&editor, cx);
    assert_eq!(
        get(&editor, cx),
        vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]
    );
    set(&editor, cx, &[(1, 6, 1, 6), (2, 6, 2, 6), (3, 6, 3, 6)]);
    add_next(&editor, cx);
    assert_eq!(
        get(&editor, cx),
        vec![(1, 5, 1, 10), (2, 5, 2, 10), (3, 5, 3, 8)]
    );
    add_next(&editor, cx);
    assert_eq!(
        get(&editor, cx),
        vec![(1, 5, 1, 10), (2, 5, 2, 10), (3, 5, 3, 8)]
    );
}

#[gpui::test]
fn issue_20651_case_insensitive_with_find_options(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["test", "testte", "Test", "testte", "test"], cx);
    cx.simulate_keystrokes("secondary-f");
    editor.update(cx, |e, cx| {
        e.set_find_options(
            FindOptions {
                query: "test".into(),
                ..Default::default()
            },
            cx,
        );
    });
    set(&editor, cx, &[(1, 1, 1, 5)]);
    add_next(&editor, cx);
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 5), (2, 1, 2, 5)]);
    add_next(&editor, cx);
    assert_eq!(
        get(&editor, cx),
        vec![(1, 1, 1, 5), (2, 1, 2, 5), (3, 1, 3, 5)]
    );
}

#[gpui::test]
fn keybindings(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["foo bar foo", "foo"], cx);
    cx.simulate_keystrokes("secondary-d secondary-d secondary-d");
    assert_eq!(
        get(&editor, cx),
        vec![(1, 1, 1, 4), (1, 9, 1, 12), (2, 1, 2, 4)]
    );
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("secondary-shift-l");
    assert_eq!(get(&editor, cx).len(), 3);
}

/// Opens find with `query` and match case `match_case`, leaving the focus
/// in its find input (Monaco's tests' `hasTextFocus: false`).
fn find_focused(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    query: &str,
    match_case: bool,
) {
    cx.simulate_keystrokes("secondary-f");
    editor.update(cx, |e, cx| {
        e.set_find_options(
            FindOptions {
                query: query.into(),
                match_case,
                ..Default::default()
            },
            cx,
        );
    });
    cx.run_until_parked();
}

/// Monaco: "the default Find session survives returning focus to the
/// editor".
#[gpui::test]
fn find_session_survives_returning_focus_to_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["foo", "bar", "bar"], cx);
    find_focused(&editor, cx, "bar", false);
    set(&editor, cx, &[(1, 1, 1, 4)]);
    cx.simulate_keystrokes("secondary-d");
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.simulate_keystrokes("secondary-d");
    assert_eq!(
        get(&editor, cx),
        vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]
    );
}

/// Monaco: "Find-focused mixed-case initial selections respect Find match
/// case false" and "… true".
#[gpui::test]
fn find_focused_mixed_case_selections_use_find_match_case(cx: &mut TestAppContext) {
    for (match_case, expected) in [
        (false, vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]),
        (true, vec![(1, 1, 1, 4), (2, 1, 2, 4)]),
    ] {
        let (editor, vcx) = setup(&["foo", "FOO", "fooBar"], cx);
        find_focused(&editor, vcx, "foo", match_case);
        set(&editor, vcx, &[(1, 1, 1, 4), (2, 1, 2, 4)]);
        vcx.simulate_keystrokes("secondary-d");
        assert_eq!(get(&editor, vcx), expected, "match case {match_case}");
    }
}

/// Monaco: "switching focus transfers matching rules between selection and
/// Find sessions" (the Find session's part: from the find input, the move
/// searches for the widget's string).
#[gpui::test]
fn move_from_the_find_input_searches_the_find_string(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        &["foo", "FOO", "fooqux", "foobar", "bar", "BAR", "barista"],
        cx,
    );
    match_mode(&editor, cx, SelectedTextMatchMode::CaseSensitive);
    set(&editor, cx, &[(1, 1, 1, 4)]);
    editor.update(cx, |e, cx| e.move_selection_to_next_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(3, 1, 3, 4)]);
    find_focused(&editor, cx, "bar", false);
    editor.update(cx, |e, cx| {
        let mut options = e.find_options().clone();
        options.whole_word = true;
        e.set_find_options(options, cx);
    });
    set(&editor, cx, &[(3, 1, 3, 4)]);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-k cmd-d"
    } else {
        "ctrl-k ctrl-d"
    });
    assert_eq!(get(&editor, cx), vec![(5, 1, 5, 4)]);
}

const OCCURRENCES: [&str; 5] = ["foo", "FOObar", "foobar", "FOO", "foo"];

/// Monaco: addSelectionToPreviousFindMatch: selected text, match case true,
/// Find visible false.
#[gpui::test]
fn add_previous_selected_text_case_sensitive(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&OCCURRENCES, cx);
    match_mode(&editor, cx, SelectedTextMatchMode::CaseSensitive);
    set(&editor, cx, &[(5, 1, 5, 4)]);
    editor.update(cx, |e, cx| e.add_selection_to_previous_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(5, 1, 5, 4), (3, 1, 3, 4)]);
}

/// Monaco: addSelectionToPreviousFindMatch: empty selection always starts
/// whole-word and case-sensitive.
#[gpui::test]
fn add_previous_from_empty_selection(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&OCCURRENCES, cx);
    set(&editor, cx, &[(5, 2, 5, 2)]);
    editor.update(cx, |e, cx| e.add_selection_to_previous_find_match(cx));
    editor.update(cx, |e, cx| e.add_selection_to_previous_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(5, 1, 5, 4), (1, 1, 1, 4)]);
}

/// Monaco: moveSelectionToPreviousFindMatch: selected text and empty
/// selection, case-sensitive.
#[gpui::test]
fn move_previous(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&OCCURRENCES, cx);
    match_mode(&editor, cx, SelectedTextMatchMode::CaseSensitive);
    set(&editor, cx, &[(5, 1, 5, 4)]);
    editor.update(cx, |e, cx| e.move_selection_to_previous_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(3, 1, 3, 4)]);

    set(&editor, cx, &[(5, 2, 5, 2)]);
    editor.update(cx, |e, cx| e.move_selection_to_previous_find_match(cx));
    editor.update(cx, |e, cx| e.move_selection_to_previous_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4)]);
}

#[gpui::test]
fn insert_cursor_at_end_of_each_line_selected(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["one", "two", "three", "four"], cx);
    // The last line counts only if the selection ends inside it.
    set(&editor, cx, &[(1, 2, 3, 3)]);
    cx.simulate_keystrokes("shift-alt-i");
    assert_eq!(
        get(&editor, cx),
        vec![(1, 4, 1, 4), (2, 4, 2, 4), (3, 3, 3, 3)]
    );
    set(&editor, cx, &[(1, 1, 3, 1)]);
    cx.simulate_keystrokes("shift-alt-i");
    assert_eq!(get(&editor, cx), vec![(1, 4, 1, 4), (2, 4, 2, 4)]);
}

#[gpui::test]
fn add_cursors_to_bottom_and_top(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["abcd", "ab", "abcd", "abcd"], cx);
    set(&editor, cx, &[(2, 2, 2, 3)]);
    editor.update(cx, |e, cx| e.add_cursors_to_edge(true, cx));
    assert_eq!(
        get(&editor, cx),
        vec![(2, 2, 2, 3), (3, 2, 3, 3), (4, 2, 4, 3)]
    );
    set(&editor, cx, &[(3, 1, 3, 1)]);
    editor.update(cx, |e, cx| e.add_cursors_to_edge(false, cx));
    assert_eq!(
        get(&editor, cx),
        vec![(3, 1, 3, 1), (2, 1, 2, 1), (1, 1, 1, 1)]
    );
}

#[gpui::test]
fn focus_next_and_previous_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["a", "b", "c"], cx);
    set(&editor, cx, &[(1, 1, 1, 1), (2, 1, 2, 1), (3, 1, 3, 1)]);
    editor.update(cx, |e, cx| e.focus_next_cursor(true, cx));
    assert_eq!(get(&editor, cx)[0], (2, 1, 2, 1));
    editor.update(cx, |e, cx| e.focus_next_cursor(false, cx));
    editor.update(cx, |e, cx| e.focus_next_cursor(false, cx));
    assert_eq!(get(&editor, cx)[0], (3, 1, 3, 1));
}

fn update_options(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut gpui_monaco::EditorOptions),
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        f(&mut options);
        e.update_options(options, cx);
    });
}

fn mouse_at(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    line: u32,
    column: u32,
) -> gpui::Point<gpui::Pixels> {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::from_monaco(line, column))
        })
        .expect("visible")
        + gpui::point(gpui::px(1.), gpui::px(2.))
}

#[gpui::test]
fn multi_cursor_limit(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["a", "b", "c", "d"], cx);
    update_options(&editor, cx, |o| o.multi_cursor_limit = 2);
    set(&editor, cx, &[(1, 1, 1, 1), (2, 1, 2, 1), (3, 1, 3, 1)]);
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 1), (2, 1, 2, 1)]);
}

#[gpui::test]
fn multi_cursor_merge_overlapping_off(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["abcdef"], cx);
    set(&editor, cx, &[(1, 1, 1, 4), (1, 3, 1, 6)]);
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 6)]);
    update_options(&editor, cx, |o| o.multi_cursor_merge_overlapping = false);
    set(&editor, cx, &[(1, 1, 1, 4), (1, 3, 1, 6)]);
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4), (1, 3, 1, 6)]);
}

#[gpui::test]
fn column_selection_mode_with_keys(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["abcd", "abcd", "abcd"], cx);
    update_options(&editor, cx, |o| o.column_selection = true);
    cx.simulate_keystrokes("shift-right shift-right shift-down");
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 3), (2, 1, 2, 3)]);
}

#[gpui::test]
fn multi_cursor_modifier_ctrl_cmd(cx: &mut TestAppContext) {
    use gpui::Modifiers;
    let (editor, cx) = setup(&["abcd", "abcd"], cx);
    // Default: Alt+click adds a cursor.
    let at = mouse_at(&editor, cx, 2, 3);
    cx.simulate_click(at, Modifiers::alt());
    assert_eq!(get(&editor, cx).len(), 2);

    update_options(&editor, cx, |o| {
        o.multi_cursor_modifier = gpui_monaco::options::MultiCursorModifier::CtrlCmd
    });
    set(&editor, cx, &[(1, 1, 1, 1)]);
    let at = mouse_at(&editor, cx, 2, 3);
    cx.simulate_click(at, Modifiers::alt());
    assert_eq!(get(&editor, cx), vec![(2, 3, 2, 3)]);
    let at = mouse_at(&editor, cx, 1, 2);
    cx.simulate_click(at, Modifiers::secondary_key());
    assert_eq!(get(&editor, cx).len(), 2);
}

/// Sets the find state's toggles and search string (Monaco's tests'
/// `findController.getState().change`), with the widget open (`revealed`,
/// the focus back in the text) or closed.
fn find_state(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    revealed: bool,
    options: FindOptions,
) {
    if revealed {
        cx.simulate_keystrokes("secondary-f");
    }
    editor.update(cx, |e, cx| e.change_find_options(options, false, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
}

/// The find state's (match case, whole word, regex).
fn find_toggles(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> [bool; 3] {
    editor.read_with(cx, |e, _| {
        let o = e.find_options();
        [o.match_case, o.whole_word, o.is_regex]
    })
}

type Run = fn(&mut CodeEditor, &mut gpui::Context<CodeEditor>);

/// The actions of Monaco's "Selected text occurrence matching" suite.
const OCCURRENCE_ACTIONS: [(&str, Run); 5] = [
    ("addSelectionToNextFindMatch", |e, cx| {
        e.add_selection_to_next_find_match(cx)
    }),
    ("addSelectionToPreviousFindMatch", |e, cx| {
        e.add_selection_to_previous_find_match(cx)
    }),
    ("moveSelectionToNextFindMatch", |e, cx| {
        e.move_selection_to_next_find_match(cx)
    }),
    ("moveSelectionToPreviousFindMatch", |e, cx| {
        e.move_selection_to_previous_find_match(cx)
    }),
    ("selectHighlights", |e, cx| e.select_all_occurrences(cx)),
];

/// Monaco: "default selected-text matching uses Find match case and whole
/// word options", with find closed.
#[gpui::test]
fn default_selected_text_matching_uses_find_match_case_and_whole_word(cx: &mut TestAppContext) {
    let mut actual = Vec::new();
    for (match_case, whole_word) in [(false, false), (false, true), (true, false), (true, true)] {
        let (editor, vcx) = setup(&OCCURRENCES, cx);
        find_state(
            &editor,
            vcx,
            false,
            FindOptions {
                match_case,
                whole_word,
                ..Default::default()
            },
        );
        set(&editor, vcx, &[(1, 1, 1, 4)]);
        add_next(&editor, vcx);
        actual.push(get(&editor, vcx));
    }
    assert_eq!(
        actual,
        vec![
            vec![(1, 1, 1, 4), (2, 1, 2, 4)],
            vec![(1, 1, 1, 4), (4, 1, 4, 4)],
            vec![(1, 1, 1, 4), (3, 1, 3, 4)],
            vec![(1, 1, 1, 4), (5, 1, 5, 4)],
        ]
    );
}

/// Monaco's "Selected text occurrence matching" matrix
/// (`assertSelectedText`): with `caseSensitive` or `caseInsensitive`,
/// selected text matches substrings whatever the find widget's toggles,
/// open or closed, and leaves them alone.
#[gpui::test]
fn selected_text_matches_by_the_mode_not_the_find_widget(cx: &mut TestAppContext) {
    // (action, start line, expected lines case-insensitive and
    // case-sensitive).
    let expected: [(usize, u32, [&[u32]; 2]); 5] = [
        (0, 1, [&[1, 2], &[1, 3]]),
        (1, 5, [&[5, 4], &[5, 3]]),
        (2, 1, [&[2], &[3]]),
        (3, 5, [&[4], &[3]]),
        (4, 1, [&[1, 2, 3, 4, 5], &[1, 3, 5]]),
    ];
    for (action, start, lines) in expected {
        let (name, run) = OCCURRENCE_ACTIONS[action];
        for (mode, lines) in [
            (SelectedTextMatchMode::CaseInsensitive, lines[0]),
            (SelectedTextMatchMode::CaseSensitive, lines[1]),
        ] {
            let match_case = mode == SelectedTextMatchMode::CaseSensitive;
            for revealed in [false, true] {
                let (editor, vcx) = setup(&OCCURRENCES, cx);
                match_mode(&editor, vcx, mode);
                find_state(
                    &editor,
                    vcx,
                    revealed,
                    FindOptions {
                        query: "FOO.*".into(),
                        match_case: !match_case,
                        whole_word: true,
                        is_regex: true,
                        ..Default::default()
                    },
                );
                set(&editor, vcx, &[(start, 1, start, 4)]);
                editor.update(vcx, run);
                let expected: Vec<Sel> = lines.iter().map(|&l| (l, 1, l, 4)).collect();
                assert_eq!(
                    (get(&editor, vcx), find_toggles(&editor, vcx)),
                    (expected, [!match_case, true, true]),
                    "{name} {mode:?} revealed {revealed}"
                );
            }
        }
    }
}

/// Monaco's "Selected text occurrence matching" matrix
/// (`assertEmptySelection`): an empty selection always starts a
/// whole-word, case-sensitive session, overriding the find toggles while
/// it lasts.
#[gpui::test]
fn empty_selection_always_starts_whole_word_case_sensitive(cx: &mut TestAppContext) {
    let expected: [(usize, u32, &[u32]); 5] = [
        (0, 1, &[1, 5]),
        (1, 5, &[5, 1]),
        (2, 1, &[5]),
        (3, 5, &[1]),
        (4, 1, &[1, 5]),
    ];
    for (action, start, lines) in expected {
        let (name, run) = OCCURRENCE_ACTIONS[action];
        for mode in [
            SelectedTextMatchMode::CaseInsensitive,
            SelectedTextMatchMode::CaseSensitive,
        ] {
            let (editor, vcx) = setup(&OCCURRENCES, cx);
            match_mode(&editor, vcx, mode);
            find_state(
                &editor,
                vcx,
                false,
                FindOptions {
                    is_regex: true,
                    ..Default::default()
                },
            );
            set(&editor, vcx, &[(start, 2, start, 2)]);
            editor.update(vcx, run);
            editor.update(vcx, run);
            let expected: Vec<Sel> = lines.iter().map(|&l| (l, 1, l, 4)).collect();
            assert_eq!(
                (get(&editor, vcx), find_toggles(&editor, vcx)),
                (expected, [true, true, false]),
                "{name} {mode:?}"
            );
        }
    }
}

/// Monaco: with several selections and an empty one, Cmd/Ctrl+D only
/// expands them to their words; the next one matches with the find
/// state's toggles (match case off), not whole-word and case-sensitive.
#[gpui::test]
fn add_next_from_several_empty_cursors_starts_no_word_session(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["foo", "foo", "Foo"], cx);
    set(&editor, cx, &[(1, 1, 1, 1), (2, 1, 2, 1)]);
    cx.simulate_keystrokes("secondary-d");
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4), (2, 1, 2, 4)]);
    // No word session: the find toggles keep their own values.
    assert_eq!(find_toggles(&editor, cx), [false, false, false]);
    cx.simulate_keystrokes("secondary-d");
    assert_eq!(
        get(&editor, cx),
        vec![(1, 1, 1, 4), (2, 1, 2, 4), (3, 1, 3, 4)]
    );
}

/// Monaco ends a session that uses the find toggles when they change
/// ("changing Find options ends caret-started sessions"): the next
/// Cmd/Ctrl+D compares the selections with the new match case.
#[gpui::test]
fn changing_the_find_toggles_ends_the_session(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["foo", "FOO", "foo", "FOO"], cx);
    set(&editor, cx, &[(1, 1, 1, 4)]);
    cx.simulate_keystrokes("secondary-d");
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4), (2, 1, 2, 4)]);
    editor.update(cx, |e, cx| {
        let mut options = e.find_options().clone();
        options.match_case = true;
        e.change_find_options(options, false, cx);
    });
    // `foo` and `FOO` differ case-sensitively: nothing to add.
    cx.simulate_keystrokes("secondary-d");
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4), (2, 1, 2, 4)]);
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// Monaco's Add Cursor Above/Below push an undo stop
/// (`model.pushStackElement()`): undo after typing with the new cursor
/// removes only that typing.
#[gpui::test]
fn add_cursor_pushes_an_undo_stop(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["", ""], cx);
    cx.simulate_input("foo");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-alt-down"
    } else {
        "ctrl-alt-down"
    });
    cx.simulate_input("x");
    assert_eq!(text(&editor, cx), "foox\nx");
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "foo\n");
}

/// Monaco's `cursorColumnSelect*` push an undo stop too.
#[gpui::test]
fn keyboard_column_select_pushes_an_undo_stop(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["", "ab"], cx);
    cx.simulate_input("foo");
    cx.dispatch_action(gpui_monaco::actions::CursorColumnSelectDown);
    cx.simulate_input("x");
    assert_eq!(text(&editor, cx), "foox\nabx");
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "foo\nab");
}

/// Monaco makes Add Cursor Above/Below and Focus Next/Previous Cursor
/// no-ops in a read-only editor.
#[gpui::test]
fn multi_cursor_commands_do_nothing_when_read_only(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["a", "b", "c"], cx);
    set(&editor, cx, &[(2, 1, 2, 1), (3, 1, 3, 1)]);
    update_options(&editor, cx, |o| o.read_only = true);
    cx.dispatch_action(gpui_monaco::actions::AddCursorBelow::default());
    cx.dispatch_action(gpui_monaco::actions::AddCursorAbove::default());
    editor.update(cx, |e, cx| e.focus_next_cursor(true, cx));
    assert_eq!(get(&editor, cx), vec![(2, 1, 2, 1), (3, 1, 3, 1)]);
}

fn trigger(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, id: &str, payload: &str) {
    let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
    editor
        .update_in(cx, |e, window, cx| {
            e.trigger_with_payload(id, payload, window, cx)
        })
        .unwrap();
}

/// Line 2 wraps after `aaaa bbbb ` (columns 1-10, then 11-20).
fn wrapped(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    let (editor, cx) = setup(&["x", "aaaa bbbb cccc dddd", "y"], cx);
    update_options(&editor, cx, |o| {
        o.word_wrap = gpui_monaco::options::WordWrap::WordWrapColumn;
        o.word_wrap_column = 10;
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    (editor, cx)
}

/// Monaco `insertCursorBelow {logicalLine: false}` goes by view lines: the
/// new cursor lands on the wrapped line's next segment.
#[gpui::test]
fn add_cursor_below_by_view_lines(cx: &mut TestAppContext) {
    let (editor, cx) = wrapped(cx);
    set(&editor, cx, &[(2, 3, 2, 3)]);
    trigger(&editor, cx, "editor.action.insertCursorBelow", "{}");
    assert_eq!(get(&editor, cx), vec![(2, 3, 2, 3), (3, 2, 3, 2)]);
    set(&editor, cx, &[(2, 3, 2, 3)]);
    trigger(
        &editor,
        cx,
        "editor.action.insertCursorBelow",
        r#"{ "logicalLine": false }"#,
    );
    assert_eq!(get(&editor, cx), vec![(2, 3, 2, 3), (2, 13, 2, 13)]);
}

/// Monaco's `cursorColumnSelect*` go by view lines: each segment of a
/// wrapped line gets a cursor.
#[gpui::test]
fn keyboard_column_select_goes_by_wrapped_segments(cx: &mut TestAppContext) {
    let (editor, cx) = wrapped(cx);
    set(&editor, cx, &[(2, 2, 2, 2)]);
    cx.dispatch_action(gpui_monaco::actions::CursorColumnSelectRight);
    cx.dispatch_action(gpui_monaco::actions::CursorColumnSelectDown);
    assert_eq!(get(&editor, cx), vec![(2, 2, 2, 3), (2, 12, 2, 13)]);
    cx.dispatch_action(gpui_monaco::actions::CursorColumnSelectDown);
    assert_eq!(
        get(&editor, cx),
        vec![(2, 2, 2, 3), (2, 12, 2, 13), (3, 2, 3, 2)]
    );
}

/// Monaco's `cursorColumnSelectDown` skips folded lines: the new cursor
/// goes on the line after the fold, not inside it.
#[gpui::test]
fn keyboard_column_select_skips_folded_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["a {", "  b", "  c", "  d", "  e", "}"], cx);
    set(&editor, cx, &[(1, 1, 1, 1)]);
    editor.update(cx, |e, cx| e.fold(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-alt-down"
    } else {
        "ctrl-shift-alt-down"
    });
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 1), (6, 1, 6, 1)]);
}

/// Monaco `deleteInsideWord {onlyWord: true}` leaves the whitespace.
#[gpui::test]
fn delete_inside_word_only_word(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["foo bar baz"], cx);
    set(&editor, cx, &[(1, 6, 1, 6)]);
    trigger(&editor, cx, "deleteInsideWord", r#"{ "onlyWord": true }"#);
    assert_eq!(text(&editor, cx), "foo  baz");
    trigger(&editor, cx, "deleteInsideWord", "{}");
    assert_eq!(text(&editor, cx), "foobaz");
}

/// Monaco `getConfiguredWordAtPosition`: a word is any run of characters
/// that aren't whitespace or word separators, letters or not, so Cmd/Ctrl+D
/// selects (and then adds) an emoji or a symbol run.
#[gpui::test]
fn cmd_d_selects_words_without_letters_or_digits(cx: &mut TestAppContext) {
    let (editor, vcx) = setup(&["a 😀 b 😀"], cx);
    set(&editor, vcx, &[(1, 3, 1, 3)]);
    vcx.simulate_keystrokes("secondary-d");
    assert_eq!(get(&editor, vcx), vec![(1, 3, 1, 5)]);
    vcx.simulate_keystrokes("secondary-d");
    assert_eq!(get(&editor, vcx), vec![(1, 3, 1, 5), (1, 8, 1, 10)]);

    let (editor, vcx) = setup(&["a -- b --"], cx);
    update_options(&editor, vcx, |o| {
        o.word_separators = o.word_separators.replace('-', "").into();
    });
    set(&editor, vcx, &[(1, 4, 1, 4)]);
    vcx.simulate_keystrokes("secondary-d");
    assert_eq!(get(&editor, vcx), vec![(1, 3, 1, 5)]);
    vcx.simulate_keystrokes("secondary-d");
    assert_eq!(get(&editor, vcx), vec![(1, 3, 1, 5), (1, 8, 1, 10)]);
}

/// Monaco `getSelectionSearchString` seeds find with the same words:
/// Cmd/Ctrl+F on a run of arrows searches for it.
#[gpui::test]
fn find_seeds_words_without_letters_or_digits(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["a →→ b"], cx);
    set(&editor, cx, &[(1, 4, 1, 4)]);
    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.find_options().query.clone()),
        "→→"
    );
}

/// Monaco `SEARCH_STRING_MAX_LENGTH` counts UTF-16 units: a selection of
/// 200,000 CJK characters (600,000 bytes) still seeds the search string.
#[gpui::test]
fn find_seed_limit_counts_utf16_units(cx: &mut TestAppContext) {
    let line = "中".repeat(200_000);
    let (editor, cx) = setup(&[&line], cx);
    set(&editor, cx, &[(1, 1, 1, 200_001)]);
    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.find_options().query.chars().count()),
        200_000
    );
}

/// Monaco `_beginSessionIfNeeded` ends a Cmd/Ctrl+D session on
/// `onDidBlurEditorText`; `_endSession` drops the word session's whole
/// word and match case overrides.
#[gpui::test]
fn blurring_the_text_ends_the_session(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["foo Foo foo"], cx);
    draw(cx);
    set(&editor, cx, &[(1, 2, 1, 2)]);
    cx.simulate_keystrokes("secondary-d");
    assert_eq!(find_toggles(&editor, cx), [true, true, false]);
    // Focus leaves the editor.
    cx.update(|window, cx| window.blur(cx));
    draw(cx);
    assert_eq!(find_toggles(&editor, cx), [false, false, false]);
    // Back in the text, a selection starts a session with the selected
    // text's matching (find's own toggles: whole word and match case off).
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    set(&editor, cx, &[(1, 1, 1, 4)]);
    cx.simulate_keystrokes("secondary-d");
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4), (1, 5, 1, 8)]);
}

/// Draws the window, active, which delivers its focus changes.
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.activate_window();
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

/// Monaco: "switching focus transfers matching rules between selection and
/// Find sessions": back in the text (with `selectedTextMatchMode` not
/// `findOptions`), the find widget's session ends and the selection's
/// matching applies again.
#[gpui::test]
fn focusing_the_text_ends_a_find_session(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        &["foo", "FOO", "fooqux", "foobar", "bar", "BAR", "barista"],
        cx,
    );
    match_mode(&editor, cx, SelectedTextMatchMode::CaseSensitive);
    draw(cx);
    find_focused(&editor, cx, "bar", false);
    draw(cx);
    editor.update(cx, |e, cx| {
        let mut options = e.find_options().clone();
        options.whole_word = true;
        e.set_find_options(options, cx);
    });
    set(&editor, cx, &[(3, 1, 3, 4)]);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-k cmd-d"
    } else {
        "ctrl-k ctrl-d"
    });
    assert_eq!(get(&editor, cx), vec![(5, 1, 5, 4)]);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    editor.update(cx, |e, cx| e.move_selection_to_next_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(7, 1, 7, 4)]);
}

/// Monaco: only Add Selection to Next Find Match grows every empty cursor
/// to its word; the others begin a session whose first result is the
/// primary cursor's word (`currentMatch`), which move puts in place of
/// the last selection and add appends.
#[gpui::test]
fn move_and_add_previous_use_only_the_primary_word(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["foo", "bar", "foo"], cx);
    set(&editor, cx, &[(1, 2, 1, 2), (2, 2, 2, 2)]);
    editor.update(cx, |e, cx| e.move_selection_to_next_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4)]);

    set(&editor, cx, &[(1, 2, 1, 2), (2, 2, 2, 2)]);
    editor.update(cx, |e, cx| e.move_selection_to_previous_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(1, 1, 1, 4)]);

    set(&editor, cx, &[(3, 2, 3, 2), (2, 2, 2, 2)]);
    editor.update(cx, |e, cx| e.add_selection_to_previous_find_match(cx));
    assert_eq!(get(&editor, cx), vec![(3, 1, 3, 4), (2, 2, 2, 2)]);
    editor.update(cx, |e, cx| e.add_selection_to_previous_find_match(cx));
    assert_eq!(
        get(&editor, cx),
        vec![(3, 1, 3, 4), (2, 2, 2, 2), (1, 1, 1, 4)]
    );
}

/// Monaco passes `MAX_SAFE_SMALL_INTEGER` to `findMatches` for Select All
/// Occurrences: only `multiCursorLimit` caps the cursors.
#[gpui::test]
fn select_all_occurrences_is_not_capped_by_the_find_limit(cx: &mut TestAppContext) {
    let line = vec!["x"; 30_000].join(" ");
    let (editor, cx) = setup(&[&line], cx);
    update_options(&editor, cx, |o| o.multi_cursor_limit = 50_000);
    set(&editor, cx, &[(1, 1, 1, 1)]);
    editor.update(cx, |e, cx| e.select_all_occurrences(cx));
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 30_000);
}
