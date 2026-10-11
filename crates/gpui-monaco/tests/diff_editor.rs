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
    let signals = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let collected = signals.clone();
    cx.update(|_, cx| {
        cx.subscribe(&diff, move |_, event: &gpui_monaco::DiffEditorEvent, _| {
            if let gpui_monaco::DiffEditorEvent::AccessibilitySignal(signal) = event {
                collected.borrow_mut().push(*signal);
            }
        })
        .detach()
    });
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
    // Monaco's audio cues for the deleted and the added line.
    assert_eq!(
        *signals.borrow(),
        vec![
            gpui_monaco::AccessibilitySignal::DiffLineDeleted,
            gpui_monaco::AccessibilitySignal::DiffLineInserted
        ]
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

#[gpui::test]
fn moved_blocks(cx: &mut TestAppContext) {
    let block: String = (0..6)
        .map(|i| format!("fn moved_{i}() {{ body_{i}(); }}\n"))
        .collect();
    let filler: String = (0..10).map(|i| format!("let x{i} = {i};\n")).collect();
    let original = format!("{block}{filler}");
    let modified = format!("{filler}{block}");
    let (diff, cx) = setup(&original, &modified, cx);
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.show_moves = true;
        d.update_options(options, cx);
    });
    paint(cx);
    let moves = diff.read_with(cx, |d, _| d.diff().unwrap().moves.len());
    assert_eq!(moves, 1);
    // A caption on each side, besides the alignment padding.
    let (original_zones, modified_zones) = diff.read_with(cx, |d, _| d.alignment_zones_for_tests());
    assert!(original_zones >= 1 && modified_zones >= 1);
    // The cursor in the moved block makes it active.
    let modified_editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    modified_editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                12, 0,
            ))],
            cx,
        )
    });
    assert_eq!(
        diff.read_with(cx, |d, _| d.active_move_for_tests()),
        Some(0)
    );
    modified_editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                2, 0,
            ))],
            cx,
        )
    });
    assert_eq!(diff.read_with(cx, |d, _| d.active_move_for_tests()), None);

    // Comparing the move: the block's two places scroll together. It's at
    // line 1 on the left and line 11 on the right.
    let original_editor = diff.read_with(cx, |d, _| d.original_editor().clone());
    diff.update(cx, |d, cx| d.set_move_to_compare(Some(0), cx));
    paint(cx);
    original_editor.update(cx, |e, cx| e.set_scroll_position(point(px(0.), px(0.)), cx));
    paint(cx);
    let (original_top, modified_top) = (
        original_editor.read_with(cx, |e, _| e.scroll_position().y),
        modified_editor.read_with(cx, |e, _| e.scroll_position().y),
    );
    assert!(
        modified_top > original_top,
        "{original_top:?} {modified_top:?}"
    );
    // Escape in an editor stops comparing.
    cx.update(|window, cx| window.focus(&modified_editor.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(diff.read_with(cx, |d, _| d.move_to_compare()), None);
}

#[gpui::test]
fn dragging_hidden_lines_bars(cx: &mut TestAppContext) {
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 50\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    hide_unchanged(&diff, cx);
    let line_height = diff.read_with(cx, |d, cx| {
        d.modified_editor()
            .read(cx)
            .options()
            .resolved_line_height()
    });
    // Dragging the second bar's top edge down three lines shows three.
    diff.update(cx, |d, cx| {
        d.drag_unchanged_for_tests(1, true, px(100.), px(100.) + line_height * 3., cx)
    });
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![1..48, 58..102]
    );
    // Dragging the bottom edge up shows lines above the end.
    diff.update(cx, |d, cx| {
        d.drag_unchanged_for_tests(1, false, px(100.), px(100.) - line_height * 2., cx)
    });
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![1..48, 58..100]
    );
    // A click without moving shows 20 more.
    diff.update(cx, |d, cx| {
        d.drag_unchanged_for_tests(0, false, px(5.), px(6.), cx)
    });
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![1..28, 58..100]
    );
}

#[gpui::test]
fn margin_revert_arrows(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd\ne", "a\nB\nc\nd\ne", cx);
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.render_gutter_menu = false;
        d.update_options(options, cx);
    });
    paint(cx);
    // One arrow, beside the changed line, in the glyph margin.
    let arrows = diff.read_with(cx, |d, cx| d.revert_arrow_bounds_for_tests(cx));
    assert_eq!(arrows.len(), 1);
    // Clicking it reverts the change.
    cx.simulate_click(arrows[0].center(), gpui::Modifiers::none());
    assert_eq!(modified_text(&diff, cx), "a\nb\nc\nd\ne");
    // With the gutter menu, there are none.
    recompute(cx);
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.render_gutter_menu = true;
        d.update_options(options, cx);
    });
    paint(cx);
    assert!(
        diff.read_with(cx, |d, cx| d.revert_arrow_bounds_for_tests(cx))
            .is_empty()
    );
}

#[gpui::test]
fn the_gutter_menu_follows_scrolling_without_a_frame_of_lag(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (diff, cx) = setup(&text, &text.replace("line 150\n", "changed\n"), cx);
    let before = diff.read_with(cx, |d, cx| d.gutter_spans_for_tests(cx));
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    modified.update(cx, |e, cx| {
        e.set_scroll_position(point(px(0.), px(100.)), cx)
    });
    // Before the next frame, the gutter already places items for the new
    // scroll position.
    let after = diff.read_with(cx, |d, cx| d.gutter_spans_for_tests(cx));
    assert_eq!(after[0].0, before[0].0 - px(100.));
}

#[gpui::test]
fn diff_editor_commands_by_monaco_id(cx: &mut TestAppContext) {
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 50\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    let trigger = |name: &'static str, cx: &mut VisualTestContext| {
        diff.update_in(cx, |d, window, cx| d.trigger(name, window, cx).unwrap());
        paint(cx);
    };
    // Other ids go to the modified editor.
    trigger("cursorBottom", cx);
    let modified_editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    assert_eq!(
        modified_editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(100, 0)
    );
    trigger("diffEditor.toggleCollapseUnchangedRegions", cx);
    assert!(diff.read_with(cx, |d, _| d.options().hide_unchanged_regions.enabled));
    assert!(!diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
    trigger("diffEditor.showAllUnchangedRegions", cx);
    assert!(diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
    trigger("diffEditor.collapseAllUnchangedRegions", cx);
    assert!(!diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
    assert!(diff.update_in(cx, |d, window, cx| d.trigger("nope", window, cx).is_err()));
}

#[gpui::test]
fn switch_side_maps_the_selection(cx: &mut TestAppContext) {
    use gpui_monaco::cursor::Selection;
    let (diff, cx) = setup("a\nb\nc\nd", "a\nx\ny\nb\nc\nd", cx);
    let (original, modified) = diff.read_with(cx, |d, _| {
        (d.original_editor().clone(), d.modified_editor().clone())
    });
    // From the modified side's `c` (after two inserted lines)...
    cx.update(|window, cx| window.focus(&modified.focus_handle(cx), cx));
    modified.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(4, 1))], cx)
    });
    diff.update_in(cx, |d, window, cx| {
        d.trigger("diffEditor.switchSide", window, cx).unwrap()
    });
    paint(cx);
    // ...to the original's `c`, now focused.
    assert_eq!(
        original.read_with(cx, |e, _| e.selections()[0]),
        Selection::collapsed(PointUtf16::new(2, 1))
    );
    assert!(cx.update(|window, cx| original.focus_handle(cx).contains_focused(window, cx)));
    // And back with the action.
    cx.dispatch_action(gpui_monaco::diff_editor::SwitchSide);
    paint(cx);
    assert_eq!(
        modified.read_with(cx, |e, _| e.selections()[0]),
        Selection::collapsed(PointUtf16::new(4, 1))
    );
}

#[gpui::test]
fn reveal_first_diff_waits_for_the_diff(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 60\n", "changed\n");
    let (diff, cx) = cx.add_window_view(|window, cx| {
        let original = cx.new(|_| TextModel::new(&original));
        let modified = cx.new(|_| TextModel::new(&modified));
        let mut diff = DiffEditor::new(original, modified, window, cx);
        // Before the diff is computed.
        diff.reveal_first_diff(cx);
        diff
    });
    paint(cx);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    assert_eq!(
        modified.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(60, 0)
    );
}

#[gpui::test]
fn view_state_round_trips(cx: &mut TestAppContext) {
    use gpui_monaco::cursor::Selection;
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 50\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    hide_unchanged(&diff, cx);
    let modified_editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    modified_editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(50, 2))], cx)
    });
    let state = diff.read_with(cx, |d, cx| d.save_view_state(cx));
    let hidden = diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests());
    assert!(!hidden.is_empty());

    diff.update(cx, |d, cx| d.show_all_unchanged_regions(cx));
    modified_editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(0, 0))], cx)
    });
    diff.update(cx, |d, cx| d.restore_view_state(&state, cx));
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        hidden
    );
    assert_eq!(
        modified_editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(50, 2)
    );
}

#[gpui::test]
fn set_models_emits_model_changed(cx: &mut TestAppContext) {
    use gpui_monaco::DiffEditorEvent;
    let (diff, cx) = setup("a", "b", cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&diff, move |_, event: &DiffEditorEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach()
    });
    let (original, modified) = cx.update(|_, cx| {
        (
            cx.new(|_| TextModel::new("x")),
            cx.new(|_| TextModel::new("y")),
        )
    });
    diff.update(cx, |d, cx| d.set_models(original, modified, cx));
    paint(cx);
    assert_eq!(
        *events.borrow(),
        vec![DiffEditorEvent::ModelChanged, DiffEditorEvent::DiffUpdated]
    );
}

#[gpui::test]
fn max_file_size_skips_the_diff(cx: &mut TestAppContext) {
    // Over 1MB.
    let big = "x".repeat(1024 * 1024 + 1);
    let (diff, cx) = setup(&big, "small", cx);
    assert!(diff.read_with(cx, |d, _| d.diff().is_some()));
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.max_file_size = 1;
        d.update_options(options, cx);
    });
    paint(cx);
    assert!(diff.read_with(cx, |d, _| d.diff().is_none()));
    // 0 is no limit.
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.max_file_size = 0;
        d.update_options(options, cx);
    });
    paint(cx);
    assert!(diff.read_with(cx, |d, _| d.diff().is_some()));
}

#[gpui::test]
fn diff_word_wrap_overrides_both_editors(cx: &mut TestAppContext) {
    use gpui_monaco::options::WordWrapOverride;
    let (diff, cx) = setup("a", "b", cx);
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.diff_word_wrap = WordWrapOverride::On;
        d.update_options(options, cx);
    });
    paint(cx);
    let overrides = |cx: &mut VisualTestContext| {
        diff.read_with(cx, |d, cx| {
            (
                d.original_editor().read(cx).options().word_wrap_override1,
                d.modified_editor().read(cx).options().word_wrap_override1,
            )
        })
    };
    assert_eq!(overrides(cx), (WordWrapOverride::On, WordWrapOverride::On));
    // Inline, the hidden original editor never wraps.
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.render_side_by_side = false;
        d.update_options(options, cx);
    });
    paint(cx);
    assert_eq!(overrides(cx), (WordWrapOverride::Off, WordWrapOverride::On));
}

#[gpui::test]
fn legacy_diff_algorithm(cx: &mut TestAppContext) {
    use gpui_monaco::DiffAlgorithm;
    use gpui_monaco::diff::{DiffOptions, compute_legacy_diff};
    let (original, modified) = ("a\nb\nc\nd", "a\nx\nc\nd\ne");
    let (diff, cx) = setup(original, modified, cx);
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.diff_algorithm = DiffAlgorithm::Legacy;
        d.update_options(options, cx);
    });
    paint(cx);
    let expected = compute_legacy_diff(
        &original.split('\n').collect::<Vec<_>>(),
        &modified.split('\n').collect::<Vec<_>>(),
        &DiffOptions::default(),
    );
    assert_eq!(diff.read_with(cx, |d, _| d.diff().cloned()), Some(expected));
}

#[gpui::test]
fn compact_mode(cx: &mut TestAppContext) {
    // Only insertions: inline in compact mode.
    let (diff, cx) = setup("a\nb", "a\nnew\nb", cx);
    assert!(diff.read_with(cx, |d, _| d.renders_side_by_side()));
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        options.compact_mode = true;
        d.update_options(options, cx);
    });
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.renders_side_by_side()));
}

#[gpui::test]
fn diff_toggle_actions(cx: &mut TestAppContext) {
    use gpui_monaco::diff_editor::{
        ToggleCollapseUnchangedRegions, ToggleShowMovedCodeBlocks,
        ToggleUseInlineViewWhenSpaceIsLimited,
    };
    let (diff, cx) = setup("a", "b", cx);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&modified.focus_handle(cx), cx));
    cx.dispatch_action(ToggleCollapseUnchangedRegions);
    cx.dispatch_action(ToggleShowMovedCodeBlocks);
    cx.dispatch_action(ToggleUseInlineViewWhenSpaceIsLimited);
    paint(cx);
    let options = diff.read_with(cx, |d, _| d.options().clone());
    assert!(options.hide_unchanged_regions.enabled);
    assert!(options.show_moves);
    assert!(!options.use_inline_view_when_space_is_limited);
}
