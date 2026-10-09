//! Ported from Monaco's `contrib/multicursor/test/browser/multicursor.test.ts`
//! (MIT). Selections are Monaco 1-based `(startLine, startCol, endLine,
//! endCol)` with the anchor at the start.
//!
//! Skipped: word-wrap and paste-order tests (covered by the cursor core),
//! the find-widget option matrix (partially covered below), and the
//! "previous" variants (Monaco's add/move-to-previous commands aren't
//! bound by default and aren't implemented).

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
