//! The side-by-side diff editor (Monaco `DiffEditorWidget`).

use gpui::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext, point, px};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{DiffEditor, TextModel};

fn setup<'a>(
    original: &str,
    modified: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<DiffEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let (original, modified) = (original.to_string(), modified.to_string());
    let (diff, cx) = cx.add_window_view(|window, cx| {
        let original = cx.new(|_| TextModel::new(&original));
        let modified = cx.new(|_| TextModel::new(&modified));
        DiffEditor::new(original, modified, window, cx)
    });
    paint(cx);
    (diff, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
fn computes_and_aligns_changes(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd", "a\nx\ny\nb\nc", cx);
    let changes = diff.read_with(cx, |d, _| {
        d.diff()
            .unwrap()
            .changes
            .iter()
            .map(|c| (c.original.to_string(), c.modified.to_string()))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        changes,
        vec![
            ("[2,2)".to_string(), "[2,4)".to_string()),
            ("[4,5)".to_string(), "[6,6)".to_string()),
        ]
    );
    // Padding on the original side for the insertion, on the modified side
    // for the deletion.
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (1, 1)
    );
}

#[gpui::test]
fn edits_update_the_diff(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nb", cx);
    assert!(diff.read_with(cx, |d, _| d.diff().unwrap().changes.is_empty()));
    let model = diff.read_with(cx, |d, cx| d.modified_editor().read(cx).model().clone());
    model.update(cx, |m, cx| {
        m.edit([(1..1, "z")], cx).unwrap();
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.diff().unwrap().changes.len()),
        1
    );
}

#[gpui::test]
fn scrolling_stays_in_sync(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (diff, cx) = setup(&text, &text, cx);
    let (original, modified) = diff.read_with(cx, |d, _| {
        (d.original_editor().clone(), d.modified_editor().clone())
    });
    modified.update(cx, |e, cx| {
        e.set_scroll_position(point(px(0.), px(300.)), cx)
    });
    paint(cx);
    assert_eq!(
        original.read_with(cx, |e, _| e.scroll_position().y),
        px(300.)
    );
    original.update(cx, |e, cx| {
        e.set_scroll_position(point(px(0.), px(120.)), cx)
    });
    paint(cx);
    assert_eq!(
        modified.read_with(cx, |e, _| e.scroll_position().y),
        px(120.)
    );
}

#[gpui::test]
fn go_to_next_and_previous_diff(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd\ne\nf", "a\nB\nc\nd\nE\nf", cx);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    let row = |cx: &mut VisualTestContext| modified.read_with(cx, |e, _| e.selections()[0].head);
    diff.update_in(cx, |d, window, cx| d.go_to_diff(true, window, cx));
    assert_eq!(row(cx), PointUtf16::new(1, 0));
    diff.update_in(cx, |d, window, cx| d.go_to_diff(true, window, cx));
    assert_eq!(row(cx), PointUtf16::new(4, 0));
    // Wraps around.
    diff.update_in(cx, |d, window, cx| d.go_to_diff(true, window, cx));
    assert_eq!(row(cx), PointUtf16::new(1, 0));
    diff.update_in(cx, |d, window, cx| d.go_to_diff(false, window, cx));
    assert_eq!(row(cx), PointUtf16::new(4, 0));
}

#[gpui::test]
fn inline_view_shows_deleted_lines_in_the_modified_editor(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd", "a\nx\ny\nb\nc", cx);
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.render_side_by_side = false;
        d.update_options(options, cx);
    });
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.renders_side_by_side()));
    // One zone for the deleted `d`; the insertion has nothing deleted.
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (0, 1)
    );
    // Back to side by side.
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.render_side_by_side = true;
        d.update_options(options, cx);
    });
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (1, 1)
    );
}

#[gpui::test]
fn narrow_diff_editors_show_inline(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    cx.simulate_resize(gpui::size(px(1200.), px(600.)));
    paint(cx);
    assert!(diff.read_with(cx, |d, _| d.renders_side_by_side()));
    cx.simulate_resize(gpui::size(px(600.), px(600.)));
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.renders_side_by_side()));
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (0, 1)
    );
    // Unless that's turned off.
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.use_inline_view_when_space_is_limited = false;
        d.update_options(options, cx);
    });
    paint(cx);
    assert!(diff.read_with(cx, |d, _| d.renders_side_by_side()));
}

fn hide_unchanged(diff: &Entity<DiffEditor>, cx: &mut VisualTestContext) {
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.hide_unchanged_regions.enabled = true;
        d.update_options(options, cx);
    });
    paint(cx);
}

#[gpui::test]
fn unchanged_regions_hide(cx: &mut TestAppContext) {
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 50\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    hide_unchanged(&diff, cx);
    // Line 51 changed: 3 lines of context around it.
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![1..48, 55..102]
    );
    let modified_editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    assert_eq!(
        modified_editor.read_with(cx, |e, _| e.hidden_areas().to_vec()),
        vec![0..47, 54..101]
    );
    // A bar's top edge reveals 20 lines above.
    diff.update(cx, |d, cx| d.reveal_unchanged_for_tests(1, true, cx));
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![1..48, 75..102]
    );
    // Moving the cursor into hidden lines reveals them.
    modified_editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                5, 0,
            ))],
            cx,
        )
    });
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![7..48, 75..102]
    );
    diff.update(cx, |d, cx| d.show_all_unchanged_regions(cx));
    assert!(diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
    diff.update(cx, |d, cx| d.collapse_all_unchanged_regions(cx));
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![1..48, 55..102]
    );
}

#[gpui::test]
fn revealed_lines_stay_revealed_after_edits(cx: &mut TestAppContext) {
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 50\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    hide_unchanged(&diff, cx);
    diff.update(cx, |d, cx| d.show_all_unchanged_regions(cx));
    // An edit far below adds a change; what was shown before stays shown.
    let model = diff.read_with(cx, |d, cx| d.modified_editor().read(cx).model().clone());
    model.update(cx, |m, cx| {
        let offset = m.text().find("line 90").unwrap();
        m.edit([(offset..offset, "new\n")], cx).unwrap();
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    paint(cx);
    let hidden = diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests());
    assert!(hidden.iter().all(|r| r.start > 51), "{hidden:?}");
}

#[gpui::test]
fn identical_models_hide_nothing(cx: &mut TestAppContext) {
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let (diff, cx) = setup(&text, &text, cx);
    hide_unchanged(&diff, cx);
    assert!(diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
}

fn modified_text(diff: &Entity<DiffEditor>, cx: &mut VisualTestContext) -> String {
    diff.read_with(cx, |d, cx| {
        d.modified_editor().read(cx).model().read(cx).text()
    })
}

fn recompute(cx: &mut VisualTestContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    paint(cx);
}

#[gpui::test]
fn reverting_blocks(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd", "a\nx\ny\nb\nc", cx);
    // Revert the insertion of x and y.
    let first = diff.read_with(cx, |d, _| d.diff().unwrap().changes[0].line_range_mapping());
    diff.update(cx, |d, cx| d.revert(&first, cx));
    assert_eq!(modified_text(&diff, cx), "a\nb\nc");
    // Reverting with a stale diff does nothing.
    let deletion = diff.read_with(cx, |d, _| d.diff().unwrap().changes[1].line_range_mapping());
    diff.update(cx, |d, cx| d.revert(&deletion, cx));
    assert_eq!(modified_text(&diff, cx), "a\nb\nc");
    recompute(cx);
    // Then the deletion of `d` at the end.
    let deletion = diff.read_with(cx, |d, _| d.diff().unwrap().changes[0].line_range_mapping());
    diff.update(cx, |d, cx| d.revert(&deletion, cx));
    assert_eq!(modified_text(&diff, cx), "a\nb\nc\nd");
    // And an insertion at the end.
    model_edit_end(&diff, "\nz", cx);
    recompute(cx);
    let insertion = diff.read_with(cx, |d, _| d.diff().unwrap().changes[0].line_range_mapping());
    diff.update(cx, |d, cx| d.revert(&insertion, cx));
    assert_eq!(modified_text(&diff, cx), "a\nb\nc\nd");
    // One undo step each.
    let model = diff.read_with(cx, |d, cx| d.modified_editor().read(cx).model().clone());
    model.update(cx, |m, cx| m.undo(None, cx));
    assert_eq!(modified_text(&diff, cx), "a\nb\nc\nd\nz");
}

fn model_edit_end(diff: &Entity<DiffEditor>, text: &str, cx: &mut VisualTestContext) {
    let model = diff.read_with(cx, |d, cx| d.modified_editor().read(cx).model().clone());
    model.update(cx, |m, cx| {
        let len = m.text().len();
        m.edit([(len..len, text)], cx).unwrap();
    });
}

#[gpui::test]
fn reverting_the_selection(cx: &mut TestAppContext) {
    let (diff, cx) = setup("one two three four\n", "one TWO three FOUR\n", cx);
    let inner = diff.read_with(cx, |d, _| {
        d.diff().unwrap().changes[0].inner_changes.clone()
    });
    assert_eq!(inner.len(), 2);
    // Only the first character change.
    diff.update(cx, |d, cx| d.revert_range_mappings(&inner[..1], cx));
    assert_eq!(modified_text(&diff, cx), "one two three FOUR\n");
}

#[gpui::test]
fn the_revert_action_reverts_changes_at_the_cursor(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd\ne", "a\nB\nc\nD\ne", cx);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&modified.read(cx).focus_handle(cx), cx));
    modified.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                3, 0,
            ))],
            cx,
        )
    });
    cx.dispatch_action(gpui_monaco::diff_editor::Revert);
    assert_eq!(modified_text(&diff, cx), "a\nB\nc\nd\ne");
}

#[gpui::test]
fn the_accessible_diff_viewer(cx: &mut TestAppContext) {
    let original: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let modified = original
        .replace("line 5\n", "five\n")
        .replace("line 30\n", "");
    let (diff, cx) = setup(&original, &modified, cx);
    let editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    // F7 in the editor opens it at the first change.
    cx.simulate_keystrokes("f7");
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_state_for_tests(cx)),
        Some((2, 0, 0))
    );
    // Down moves through the group: three lines of context, then the
    // change.
    cx.simulate_keystrokes("down down down down");
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_current_for_tests(cx)),
        Some("deleted")
    );
    // The modified cursor follows (line 6, the change's start).
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(5, 0)
    );
    cx.simulate_keystrokes("down");
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_current_for_tests(cx)),
        Some("added")
    );
    // F7 again: the next change; then it cycles back.
    cx.simulate_keystrokes("f7");
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_state_for_tests(cx)),
        Some((2, 1, 0))
    );
    cx.simulate_keystrokes("f7");
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_state_for_tests(cx)),
        Some((2, 0, 0))
    );
    // Escape closes it and focuses the modified editor.
    cx.simulate_keystrokes("escape");
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.is_accessible_diff_viewer_visible()));
    assert!(cx.update(|window, cx| editor.read(cx).focus_handle(cx).is_focused(window)));
    // Shift+F7 opens it at the previous change.
    cx.simulate_keystrokes("shift-f7");
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_state_for_tests(cx)),
        Some((2, 1, 0))
    );
    // Enter on a line reveals it in the editor.
    cx.simulate_keystrokes("down down down down");
    cx.simulate_keystrokes("enter");
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.is_accessible_diff_viewer_visible()));
    let original_editor = diff.read_with(cx, |d, _| d.original_editor().clone());
    assert!(cx.update(|window, cx| original_editor.read(cx).focus_handle(cx).is_focused(window)));
    assert_eq!(
        original_editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(30, 0)
    );
}

#[gpui::test]
fn only_showing_the_accessible_diff_viewer(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.only_show_accessible_diff_viewer = true;
        d.update_options(options, cx);
    });
    paint(cx);
    cx.simulate_keystrokes("f7");
    cx.simulate_keystrokes("escape");
    paint(cx);
    // It can't be closed.
    assert!(diff.read_with(cx, |d, _| d.is_accessible_diff_viewer_visible()));
}
