//! Ported from Monaco's `contrib/multicursor/test/browser/multicursor.test.ts`
//! (MIT). Selections are Monaco 1-based `(startLine, startCol, endLine,
//! endCol)` with the anchor at the start.
//!
//! Skipped: word-wrap and paste-order tests (covered by the cursor core),
//! and most of the find-widget option matrix (`selectedTextMatchMode` isn't
//! an option here; selected text matches case-sensitively, Monaco's
//! `caseSensitive` mode).

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
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

const OCCURRENCES: [&str; 5] = ["foo", "FOObar", "foobar", "FOO", "foo"];

/// Monaco: addSelectionToPreviousFindMatch: selected text, match case true,
/// Find visible false.
#[gpui::test]
fn add_previous_selected_text_case_sensitive(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&OCCURRENCES, cx);
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
