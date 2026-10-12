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
    // One zone for the deleted `d`; the insertion has nothing deleted. The
    // original column pads after each change (Monaco's `gutter-delete`
    // zones).
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (2, 1)
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
        (1, 1)
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
    // (Scrolling to where it is reports nothing, as in Monaco: move first.)
    original_editor.update(cx, |e, cx| e.set_scroll_position(point(px(0.), px(1.)), cx));
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

fn update_options(
    diff: &Entity<DiffEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut gpui_monaco::DiffEditorOptions),
) {
    diff.update(cx, |d, cx| {
        let mut options = d.options().clone();
        f(&mut options);
        d.update_options(options, cx);
    });
    paint(cx);
}

/// The diff editor's decorations on one side: (Monaco 1-based range,
/// options).
fn side_decorations(
    diff: &Entity<DiffEditor>,
    modified: bool,
    cx: &mut VisualTestContext,
) -> Vec<((u32, u32, u32, u32), gpui_monaco::DecorationOptions)> {
    let editor = diff.read_with(cx, |d, _| {
        if modified {
            d.modified_editor().clone()
        } else {
            d.original_editor().clone()
        }
    });
    let id = editor.entity_id();
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |model, _| {
        let buffer = model.buffer().clone();
        model
            .all_decorations(Some(id))
            .into_iter()
            .filter(|d| d.owner == Some(id))
            .map(|d| {
                let a = buffer.offset_to_point_utf16(d.range.start).to_monaco();
                let b = buffer.offset_to_point_utf16(d.range.end).to_monaco();
                ((a.0, a.1, b.0, b.1), (*d.options).clone())
            })
            .collect()
    })
}

#[gpui::test]
fn diff_code_lens(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    let code_lens = |diff: &Entity<DiffEditor>, cx: &mut VisualTestContext| {
        diff.read_with(cx, |d, cx| {
            (
                d.original_editor().read(cx).options().code_lens,
                d.modified_editor().read(cx).options().code_lens,
            )
        })
    };
    // Monaco `diffCodeLens` is off by default, for both editors.
    assert_eq!(code_lens(&diff, cx), (false, false));
    update_options(&diff, cx, |o| o.diff_code_lens = true);
    assert_eq!(code_lens(&diff, cx), (true, true));
}

#[gpui::test]
fn accessibility_verbose(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    let labels = |diff: &Entity<DiffEditor>, cx: &mut VisualTestContext| {
        diff.read_with(cx, |d, cx| {
            (
                d.original_editor()
                    .read(cx)
                    .options()
                    .aria_label
                    .to_string(),
                d.modified_editor()
                    .read(cx)
                    .options()
                    .aria_label
                    .to_string(),
            )
        })
    };
    let plain = labels(&diff, cx);
    assert_eq!(plain.1, "Editor content");
    // Monaco `_updateAriaLabel`: a navigation tip, added once and removed
    // again.
    update_options(&diff, cx, |o| o.accessibility_verbose = true);
    let verbose = labels(&diff, cx);
    assert!(verbose.0.starts_with("Editor content use "));
    assert!(verbose.1.ends_with("accessible diff viewer."));
    update_options(&diff, cx, |o| o.diff_code_lens = true);
    assert_eq!(labels(&diff, cx), verbose);
    update_options(&diff, cx, |o| o.accessibility_verbose = false);
    assert_eq!(labels(&diff, cx), plain);
}

#[gpui::test]
fn show_empty_decorations(cx: &mut TestAppContext) {
    // `x` inserted: an empty range on the original side.
    let (diff, cx) = setup("abc\nd", "abxc\nd", cx);
    let empty = |diff: &Entity<DiffEditor>, cx: &mut VisualTestContext| {
        side_decorations(diff, false, cx)
            .into_iter()
            .filter_map(|(range, options)| Some((range, options.left_border?)))
            .collect::<Vec<_>>()
    };
    // Monaco `diffDeleteDecorationEmpty`: a 3px bar a pixel left of the
    // insertion point.
    let marks = empty(&diff, cx);
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].0, (1, 3, 1, 3));
    assert_eq!(marks[0].1.width, px(3.));
    assert_eq!(marks[0].1.offset, px(-1.));
    update_options(&diff, cx, |o| o.show_empty_decorations = false);
    assert!(empty(&diff, cx).is_empty());
}

#[gpui::test]
fn true_inline_view(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nfoo bar\nc", "a\nfoo baz\nc", cx);
    let deleted_text = |diff: &Entity<DiffEditor>, cx: &mut VisualTestContext| {
        side_decorations(diff, true, cx)
            .into_iter()
            .filter_map(|(range, options)| {
                let before = options.before?;
                assert!(before.style.strikethrough);
                Some((range, before.content.to_string()))
            })
            .collect::<Vec<_>>()
    };
    update_options(&diff, cx, |o| o.render_side_by_side = false);
    // The deleted line shows above, the original column pads for the
    // modified line.
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (1, 1)
    );
    assert!(deleted_text(&diff, cx).is_empty());
    // Monaco `experimental.useTrueInlineView`: the deleted `r` struck
    // through before the inserted `z`, and no zone.
    update_options(&diff, cx, |o| o.use_true_inline_view = true);
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (0, 0)
    );
    assert_eq!(
        deleted_text(&diff, cx),
        vec![((2, 7, 2, 7), "r".to_string())]
    );
    // Only in the inline view.
    update_options(&diff, cx, |o| o.render_side_by_side = true);
    assert!(deleted_text(&diff, cx).is_empty());
}

#[gpui::test]
fn true_inline_changes_count_as_simple_in_compact_mode(cx: &mut TestAppContext) {
    let (diff, cx) = setup("foo bar", "foo baz", cx);
    update_options(&diff, cx, |o| o.compact_mode = true);
    assert!(diff.read_with(cx, |d, _| d.renders_side_by_side()));
    update_options(&diff, cx, |o| o.use_true_inline_view = true);
    assert!(!diff.read_with(cx, |d, _| d.renders_side_by_side()));
}

#[gpui::test]
fn screen_readers_keep_narrow_diffs_side_by_side(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    cx.simulate_resize(gpui::size(px(600.), px(600.)));
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.renders_side_by_side()));
    // Monaco's `_screenReaderMode` ignores `useInlineViewWhenSpaceIsLimited`.
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    modified.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.accessibility_support = gpui_monaco::options::AccessibilitySupport::On;
        e.update_options(options, cx);
    });
    paint(cx);
    paint(cx);
    assert!(diff.read_with(cx, |d, _| d.renders_side_by_side()));
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

// ----- Diff editor fidelity (#257) -----------------------------------------

fn editors(
    diff: &Entity<DiffEditor>,
    cx: &mut VisualTestContext,
) -> (
    Entity<gpui_monaco::CodeEditor>,
    Entity<gpui_monaco::CodeEditor>,
) {
    diff.read_with(cx, |d, _| {
        (d.original_editor().clone(), d.modified_editor().clone())
    })
}

fn set_editor_options(
    editor: &Entity<gpui_monaco::CodeEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut gpui_monaco::EditorOptions),
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        f(&mut options);
        e.update_options(options, cx);
    });
    paint(cx);
}

/// Monaco `_adjustOptionsForSubEditor`: no minimap or folding, no sticky
/// scroll with hidden unchanged regions, and the hidden original editor of
/// the inline view without sticky scroll, unicode highlighting or glyph
/// margin.
#[gpui::test]
fn sub_editors_turn_off_minimap_folding_and_sticky_scroll(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    let (original, modified) = editors(&diff, cx);
    for editor in [&original, &modified] {
        let options = editor.read_with(cx, |e, _| e.options().clone());
        assert!(!options.minimap.enabled);
        assert!(!options.folding);
        assert!(options.sticky_scroll.enabled);
    }
    // A host turning the minimap back on doesn't win.
    set_editor_options(&modified, cx, |o| o.minimap.enabled = true);
    assert!(!modified.read_with(cx, |e, _| e.options().minimap.enabled));
    update_options(&diff, cx, |o| o.hide_unchanged_regions.enabled = true);
    assert!(!modified.read_with(cx, |e, _| e.options().sticky_scroll.enabled));
    update_options(&diff, cx, |o| o.hide_unchanged_regions.enabled = false);
    assert!(modified.read_with(cx, |e, _| e.options().sticky_scroll.enabled));
    // Inline, the original editor only shows its line numbers.
    update_options(&diff, cx, |o| o.render_side_by_side = false);
    let options = original.read_with(cx, |e, _| e.options().clone());
    assert!(!options.sticky_scroll.enabled);
    assert!(!options.glyph_margin);
    assert!(!options.unicode_highlight.invisible_characters);
    assert!(!options.unicode_highlight.ambiguous_characters);
    update_options(&diff, cx, |o| o.render_side_by_side = true);
    let options = original.read_with(cx, |e, _| e.options().clone());
    assert!(options.sticky_scroll.enabled);
    assert!(options.glyph_margin);
    assert!(options.unicode_highlight.invisible_characters);
}

/// Monaco `originalAriaLabel`/`modifiedAriaLabel`.
#[gpui::test]
fn aria_label_options(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    update_options(&diff, cx, |o| {
        o.original_aria_label = Some("Before".into());
        o.modified_aria_label = Some("After".into());
    });
    let (original, modified) = editors(&diff, cx);
    assert_eq!(
        original.read_with(cx, |e, _| e.options().aria_label.clone()),
        "Before"
    );
    assert_eq!(
        modified.read_with(cx, |e, _| e.options().aria_label.clone()),
        "After"
    );
}

/// Monaco `getAdditionalLineHeights`: other view zones count, and
/// alignment follows their changes.
#[gpui::test]
fn alignment_counts_other_view_zones(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc", "a\nb\nc", cx);
    let (_, modified) = editors(&diff, cx);
    assert!(
        diff.read_with(cx, |d, _| d.alignment_heights_for_tests())
            .is_empty()
    );
    let view = cx.update(|_, cx| cx.new(|_| gpui::Empty));
    let zone = modified.update(cx, |e, cx| {
        e.add_view_zone(
            gpui_monaco::widgets::ViewZone::new(
                1,
                gpui_monaco::widgets::ZoneHeight::Pixels(px(40.)),
                view.into(),
            ),
            cx,
        )
    });
    paint(cx);
    // The original pads after line 1 by as much.
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_heights_for_tests()),
        vec![(true, 1, 40.)]
    );
    modified.update(cx, |e, cx| e.remove_widget(zone, cx));
    paint(cx);
    assert!(
        diff.read_with(cx, |d, _| d.alignment_heights_for_tests())
            .is_empty()
    );
}

/// Monaco realigns on `lineHeight` changes (e.g. the font zoom).
#[gpui::test]
fn alignment_follows_the_line_height(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nx\nb", cx);
    let (original, modified) = editors(&diff, cx);
    let height =
        |cx: &mut VisualTestContext| diff.read_with(cx, |d, _| d.alignment_heights_for_tests());
    let line_height = f32::from(modified.read_with(cx, |e, _| e.options().resolved_line_height()));
    assert_eq!(height(cx), vec![(true, 1, line_height)]);
    for editor in [&original, &modified] {
        set_editor_options(editor, cx, |o| o.line_height = Some(px(30.)));
    }
    paint(cx);
    assert_eq!(height(cx), vec![(true, 1, 30.)]);
}

/// Monaco `renderLines` with the modified editor's line breaks and
/// `minWidthInPx`.
#[gpui::test]
fn inline_deleted_lines_wrap_and_widen_the_scroll_range(cx: &mut TestAppContext) {
    let long = "x".repeat(400);
    let (diff, cx) = setup(&format!("a\n{long}\nb"), "a\nb", cx);
    update_options(&diff, cx, |o| o.render_side_by_side = false);
    let layout = diff.read_with(cx, |d, cx| d.deleted_code_layout_for_tests(cx));
    assert_eq!(layout.len(), 1);
    let (lines, min_width) = layout[0];
    assert_eq!(lines, 1);
    // The modified text is short, but the deleted line can be scrolled to.
    let (_, modified) = editors(&diff, cx);
    let width = modified.read_with(cx, |e, _| e.content_width().unwrap());
    assert!(min_width > px(400. * 5.), "{min_width:?}");
    assert!(width >= min_width, "{width:?} {min_width:?}");
    // With word wrap the deleted line wraps, and the original column pads
    // for its extra rows.
    update_options(&diff, cx, |o| {
        o.diff_word_wrap = gpui_monaco::options::WordWrapOverride::On
    });
    paint(cx);
    let (lines, _) = diff.read_with(cx, |d, cx| d.deleted_code_layout_for_tests(cx))[0];
    assert!(lines > 1, "{lines}");
    let zones = diff.read_with(cx, |d, _| d.alignment_heights_for_tests());
    let line_height = f32::from(modified.read_with(cx, |e, _| e.options().resolved_line_height()));
    assert!(
        zones.contains(&(false, 1, lines as f32 * line_height)),
        "{zones:?}"
    );
    assert!(
        zones.contains(&(true, 2, (lines - 1) as f32 * line_height)),
        "{zones:?}"
    );
}

/// Monaco hides the editors behind the accessible diff viewer: clicks and
/// the wheel don't reach them.
#[gpui::test]
fn the_accessible_diff_viewer_blocks_the_editors(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (diff, cx) = setup(&text, &text.replace("line 5\n", "five\n"), cx);
    let (_, modified) = editors(&diff, cx);
    diff.update_in(cx, |d, window, cx| {
        d.accessible_diff_viewer_next(window, cx)
    });
    paint(cx);
    let position = point(px(700.), px(300.));
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-200.))),
        ..Default::default()
    });
    paint(cx);
    assert_eq!(modified.read_with(cx, |e, _| e.scroll_position().y), px(0.));
    let before = modified.read_with(cx, |e, _| e.selections()[0].head);
    cx.simulate_click(position, gpui::Modifiers::none());
    paint(cx);
    assert_eq!(
        modified.read_with(cx, |e, _| e.selections()[0].head),
        before
    );
    assert!(diff.read_with(cx, |d, _| d.is_accessible_diff_viewer_visible()));
}

/// Monaco's inline view keeps the original editor as a column of its line
/// numbers, unless `hideOriginalLineNumbers` (or `compactMode`).
#[gpui::test]
fn inline_view_shows_original_line_numbers(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd", "a\nc\nd", cx);
    update_options(&diff, cx, |o| o.render_side_by_side = false);
    paint(cx);
    let width = diff.read_with(cx, |d, cx| d.inline_original_width_for_tests(cx));
    assert!(width > px(5.), "{width:?}");
    // The deleted line's number shows in it.
    let (original, _) = editors(&diff, cx);
    let labels = original.read_with(cx, |e, _| e.line_number_labels_for_tests());
    assert!(labels.contains(&(1, "2".to_string())), "{labels:?}");
    update_options(&diff, cx, |o| o.hide_original_line_numbers = true);
    assert_eq!(
        diff.read_with(cx, |d, cx| d.inline_original_width_for_tests(cx)),
        px(0.)
    );
}

/// Monaco `InlineDiffDeletedCodeMargin` and `copySelection`.
#[gpui::test]
fn inline_deleted_lines_copy_and_revert(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nold one\nold two\nz", "a\nz", cx);
    update_options(&diff, cx, |o| o.render_side_by_side = false);
    assert_eq!(
        diff.read_with(cx, |d, cx| d.deleted_menu_for_tests(0, 2, cx)),
        vec![
            "Copy deleted lines".to_string(),
            "Copy deleted line (3)".to_string(),
            "Revert this change".to_string(),
        ]
    );
    diff.update(cx, |d, cx| d.run_deleted_menu_item_for_tests(0, 2, 0, cx));
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some("old one\nold two\n".to_string())
    );
    diff.update(cx, |d, cx| d.run_deleted_menu_item_for_tests(0, 2, 1, cx));
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some("old two".to_string())
    );
    // A selection in the deleted text copies.
    assert!(diff.update(cx, |d, cx| {
        d.copy_deleted_text_for_tests(0, PointUtf16::new(1, 4)..PointUtf16::new(2, 3), cx)
    }));
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some("one\nold".to_string())
    );
    // Revert this change.
    diff.update(cx, |d, cx| d.run_deleted_menu_item_for_tests(0, 1, 2, cx));
    paint(cx);
    assert_eq!(modified_text(&diff, cx), "a\nold one\nold two\nz");
}

/// Monaco `OverviewRulerFeature`: the diff's marks are in its own two
/// columns, not in the editors' rulers.
#[gpui::test]
fn the_diff_overview_ruler(cx: &mut TestAppContext) {
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let (diff, cx) = setup(&text, &text.replace("line 50\n", "changed\n"), cx);
    let (original_marks, modified_marks) =
        diff.read_with(cx, |d, cx| d.overview_zones_for_tests(cx));
    assert_eq!((original_marks.len(), modified_marks.len()), (1, 1));
    let (top, bottom) = modified_marks[0];
    assert!(top > 0.2 && bottom < 0.8 && top < bottom, "{top} {bottom}");
    assert!(
        side_decorations(&diff, true, cx)
            .iter()
            .all(|(_, o)| o.overview_ruler.is_none())
    );
    // The modified editor leaves room for the 30px column.
    let (_, modified) = editors(&diff, cx);
    let width = modified.read_with(cx, |e, _| e.layout_info().unwrap().width);
    let window = cx.update(|window, _| window.viewport_size().width);
    let sash = diff.read_with(cx, |d, _| d.sash_left_for_tests());
    assert_eq!(width, window - sash - px(30.));
}

/// Monaco `DiffEditorSash`: 100px minimum per side, double click resets.
#[gpui::test]
fn the_sash(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    paint(cx);
    // The width less the overview ruler, split in half.
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(600.));
    diff.update(cx, |d, cx| d.drag_sash_for_tests(px(600.), px(10.), cx));
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(100.));
    diff.update(cx, |d, cx| d.drag_sash_for_tests(px(100.), px(5000.), cx));
    assert_eq!(
        diff.read_with(cx, |d, _| d.sash_left_for_tests()),
        px(1100.)
    );
    diff.update(cx, |d, cx| d.reset_sash(cx));
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(600.));
    // A double click on it resets it too.
    diff.update(cx, |d, cx| d.drag_sash_for_tests(px(600.), px(400.), cx));
    paint(cx);
    let at = point(px(400.), px(300.));
    cx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position: at,
        modifiers: gpui::Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position: at,
        modifiers: gpui::Modifiers::none(),
        click_count: 2,
    });
    paint(cx);
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(600.));
}

/// Monaco captures the pointer on the sash: its resize cursor stays over
/// the whole window, over both editors' text, until the release (or a move
/// without the button).
#[gpui::test]
fn dragging_the_sash_keeps_its_pointer_everywhere(cx: &mut TestAppContext) {
    use gpui::{CursorStyle, Modifiers, MouseButton};
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    paint(cx);
    let captured =
        |cx: &mut VisualTestContext| diff.read_with(cx, |d, _| d.captured_cursor_for_tests());
    assert_eq!(captured(cx), None);
    let from = point(px(600.), px(300.));
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeColumn));
    for to in [
        point(px(400.), px(20.)),
        point(px(900.), px(20.)),
        point(px(-10.), px(700.)),
    ] {
        cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        paint(cx);
        assert_eq!(captured(cx), Some(CursorStyle::ResizeColumn));
    }
    cx.simulate_mouse_move(
        point(px(500.), px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(500.), px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), None);
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(500.));

    cx.simulate_mouse_move(point(px(500.), px(300.)), None, Modifiers::default());
    cx.simulate_mouse_down(
        point(px(500.), px(300.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(300.), px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeColumn));
    cx.simulate_mouse_move(point(px(300.), px(20.)), None, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), None);
}

/// Monaco: a click on the gutter focuses the modified editor.
#[gpui::test]
fn clicking_the_gutter_focuses_the_modified_editor(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    paint(cx);
    let (original, modified) = editors(&diff, cx);
    cx.update(|window, cx| window.focus(&original.focus_handle(cx), cx));
    let sash = diff.read_with(cx, |d, _| d.sash_left_for_tests());
    // The gutter is the 35px left of the sash.
    cx.simulate_click(point(sash - px(20.), px(300.)), gpui::Modifiers::none());
    paint(cx);
    assert!(cx.update(|window, cx| modified.focus_handle(cx).contains_focused(window, cx)));
}

/// Monaco `shouldFillLineOnLineBreak` on the changed characters.
#[gpui::test]
fn char_changes_fill_to_the_line_end(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nfoo\nb", "a\nfoo bar\nbaz\nb", cx);
    let chars: Vec<_> = side_decorations(&diff, true, cx)
        .into_iter()
        .filter(|(_, o)| !o.is_whole_line && o.background.is_some())
        .collect();
    assert!(!chars.is_empty());
    assert!(chars.iter().all(|(_, o)| o.should_fill_line_on_line_break));
}

/// Monaco's "Fold Unchanged Region" glyph and `_runWithLowerScrollAnchor`.
#[gpui::test]
fn hidden_region_fold_glyph_and_bottom_drag(cx: &mut TestAppContext) {
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 50\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    hide_unchanged(&diff, cx);
    assert_eq!(diff.read_with(cx, |d, _| d.fold_glyphs_for_tests()), (0, 0));
    diff.update(cx, |d, cx| d.show_all_unchanged_regions(cx));
    paint(cx);
    // Fully shown regions get the glyph on both sides.
    assert_eq!(diff.read_with(cx, |d, _| d.fold_glyphs_for_tests()), (2, 2));
    diff.update(cx, |d, cx| d.fold_unchanged_for_tests(1, cx));
    paint(cx);
    assert_eq!(diff.read_with(cx, |d, _| d.fold_glyphs_for_tests()), (1, 1));
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![55..102]
    );
    // Dragging the first region's bottom edge up shows lines above the
    // change and scrolls by as much, so what's below stays put.
    diff.update(cx, |d, cx| d.collapse_all_unchanged_regions(cx));
    paint(cx);
    let (_, modified_editor) = editors(&diff, cx);
    let line_height = modified_editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let before = modified_editor.read_with(cx, |e, _| e.scroll_position().y);
    diff.update(cx, |d, cx| {
        d.drag_unchanged_for_tests(0, false, px(100.), px(100.) - line_height * 2., cx)
    });
    let after = modified_editor.read_with(cx, |e, _| e.scroll_position().y);
    assert_eq!(after - before, line_height * 2.);
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests())[0],
        1..46
    );
}

struct Symbols;
impl gpui_monaco::providers::DocumentSymbolProvider for Symbols {
    fn provide_document_symbols(
        &self,
        _: &gpui_monaco::providers::ProviderContext,
        _: &mut gpui::App,
    ) -> gpui::Task<anyhow::Result<Vec<gpui_monaco::providers::DocumentSymbol>>> {
        use gpui_monaco::providers::{DocumentSymbol, SymbolKind};
        let symbol = |name: &str, start: u32, end: u32, children| DocumentSymbol {
            name: name.to_string().into(),
            kind: SymbolKind::Function,
            range: PointUtf16::new(start, 0)..PointUtf16::new(end, 0),
            children,
            ..Default::default()
        };
        gpui::Task::ready(Ok(vec![symbol(
            "outer",
            10,
            80,
            vec![
                symbol("inner", 20, 75, Vec::new()),
                symbol("small", 5, 8, Vec::new()),
            ],
        )]))
    }
}

/// Monaco `diffEditorBreadcrumbs`: the symbols the hidden lines start that
/// end after them, outermost first; clicking one reveals it.
#[gpui::test]
fn hidden_region_breadcrumbs(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::providers::register_document_symbol_provider(
            cx,
            gpui_monaco::providers::LanguageSelector::All,
            Symbols,
        );
    });
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 70\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    hide_unchanged(&diff, cx);
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.breadcrumbs_for_tests(0)),
        vec!["outer".to_string(), "inner".to_string()]
    );
    diff.update(cx, |d, cx| d.click_breadcrumb_for_tests(0, 1, cx));
    let hidden = diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests());
    assert!(hidden[0].end <= 21, "{hidden:?}");
}

/// Monaco hides the margin revert arrows while comparing a moved block and
/// keeps the other changes' arrows with a selection.
#[gpui::test]
fn margin_arrows_with_selection_and_compare(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd\ne", "a\nB\nc\nD\ne", cx);
    update_options(&diff, cx, |o| o.render_gutter_menu = false);
    assert_eq!(
        diff.read_with(cx, |d, cx| d.revert_arrow_bounds_for_tests(cx))
            .len(),
        2
    );
    let (_, modified) = editors(&diff, cx);
    modified.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection {
                anchor: PointUtf16::new(1, 0),
                head: PointUtf16::new(1, 1),
            }],
            cx,
        )
    });
    paint(cx);
    // The selection's arrow and the other change's.
    assert_eq!(
        diff.read_with(cx, |d, cx| d.revert_arrow_bounds_for_tests(cx))
            .len(),
        2
    );
}

/// Monaco's extensible `DiffEditorHunkToolbar`.
#[gpui::test]
fn host_hunk_actions(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc", "a\nB\nc", cx);
    let (_, modified) = editors(&diff, cx);
    set_editor_options(&modified, cx, |o| o.read_only = true);
    // Read-only: no built-in revert, so no gutter.
    assert!(!diff.read_with(cx, |d, cx| d.shows_gutter_for_tests(cx)));
    let staged = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = staged.clone();
    diff.update(cx, |d, cx| {
        d.add_hunk_action(
            gpui_monaco::DiffHunkAction {
                id: "stage".into(),
                label: "Stage".into(),
                icon: "+".into(),
                toolbar: gpui_monaco::DiffHunkToolbar::Hunk,
                run: std::rc::Rc::new(move |context, _, _| {
                    *sink.borrow_mut() = Some(context.original_with_modified_changes.clone());
                }),
            },
            cx,
        )
    });
    assert!(diff.read_with(cx, |d, cx| d.shows_gutter_for_tests(cx)));
    assert!(diff.update_in(cx, |d, window, cx| {
        d.run_hunk_action_for_tests(0, "stage", window, cx)
    }));
    assert_eq!(staged.borrow().as_deref(), Some("a\nB\nc"));
    assert!(diff.update(cx, |d, cx| d.remove_hunk_action("stage", cx)));
    assert!(!diff.read_with(cx, |d, cx| d.shows_gutter_for_tests(cx)));
}

/// Monaco `getLineChanges`, `identical`, `setModel(null)` and
/// `waitForDiff`.
#[gpui::test]
fn diff_editor_api(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd", "a\nB\nc", cx);
    let changes = diff.read_with(cx, |d, _| d.line_changes().unwrap());
    assert_eq!(changes.len(), 2);
    assert_eq!(
        (
            changes[0].original_start_line_number,
            changes[0].original_end_line_number,
            changes[0].modified_start_line_number,
            changes[0].modified_end_line_number,
        ),
        (2, 2, 2, 2)
    );
    assert_eq!(changes[0].char_changes.as_ref().unwrap().len(), 1);
    // A deletion: the modified side is the line before, with end 0.
    assert_eq!(
        (
            changes[1].modified_start_line_number,
            changes[1].modified_end_line_number
        ),
        (3, 0)
    );
    assert!(changes[1].char_changes.is_none());
    assert_eq!(diff.read_with(cx, |d, _| d.identical()), Some(false));

    // Trimmed whitespace: no changes, but not identical.
    let model = |text: &str, cx: &mut VisualTestContext| {
        let text = text.to_string();
        cx.update(|_, cx| cx.new(|_| TextModel::new(&text)))
    };
    let (original, modified) = (model("a", cx), model("a  ", cx));
    diff.update(cx, |d, cx| d.set_models(original, modified, cx));
    let wait = diff.update(cx, |d, _| d.wait_for_diff());
    paint(cx);
    futures::executor::block_on(wait);
    assert!(diff.read_with(cx, |d, _| d.diff().unwrap().changes.is_empty()));
    assert_eq!(diff.read_with(cx, |d, _| d.identical()), Some(false));
    let (original, modified) = (model("x", cx), model("x", cx));
    diff.update(cx, |d, cx| d.set_models(original, modified, cx));
    paint(cx);
    assert_eq!(diff.read_with(cx, |d, _| d.identical()), Some(true));

    // `setModel(null)`.
    diff.update(cx, |d, cx| d.set_model(None, cx));
    paint(cx);
    assert!(diff.read_with(cx, |d, cx| d.model(cx).is_none()));
    assert!(diff.read_with(cx, |d, _| d.diff().is_none()));
    assert_eq!(modified_text(&diff, cx), "");
}

/// Monaco `IStandaloneDiffEditor.addAction` goes to the modified editor.
#[gpui::test]
fn diff_editor_add_action(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    let _action = diff.update(cx, |d, cx| {
        d.add_action(
            gpui_monaco::EditorActionDescriptor::new("my.action", "My Action", |_, _, _| {}),
            cx,
        )
    });
    let (_, modified) = editors(&diff, cx);
    assert!(modified.read_with(cx, |e, cx| {
        e.supported_actions(cx)
            .iter()
            .any(|a| a.id.as_ref() == "my.action")
    }));
}

/// Monaco's signals from `goToDiff` and cursor moves onto changes.
#[gpui::test]
fn accessibility_signals_for_changes(cx: &mut TestAppContext) {
    use gpui_monaco::{AccessibilitySignal, DiffEditorEvent};
    let (diff, cx) = setup("a\nb\nc\nd\ne", "a\nB\nc\nnew\nd\ne", cx);
    let signals = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = signals.clone();
    cx.update(|_, cx| {
        cx.subscribe(&diff, move |_, event: &DiffEditorEvent, _| {
            if let DiffEditorEvent::AccessibilitySignal(signal) = event {
                sink.borrow_mut().push(*signal);
            }
        })
        .detach()
    });
    diff.update_in(cx, |d, window, cx| d.go_to_diff(true, window, cx));
    diff.update_in(cx, |d, window, cx| d.go_to_diff(true, window, cx));
    paint(cx);
    assert_eq!(
        signals.borrow().as_slice(),
        &[
            AccessibilitySignal::DiffLineModified,
            AccessibilitySignal::DiffLineInserted
        ]
    );
}

/// Monaco `applyEditToLineRange`: an edit moves the shown diff until it's
/// computed again.
#[gpui::test]
fn edits_move_the_diff_while_it_recomputes(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc", "a\nb\nX\nc", cx);
    let change = |cx: &mut VisualTestContext| {
        diff.read_with(cx, |d, _| d.diff().unwrap().changes[0].modified.to_string())
    };
    assert_eq!(change(cx), "[3,4)");
    let model = diff.read_with(cx, |d, cx| d.modified_editor().read(cx).model().clone());
    model.update(cx, |m, cx| {
        m.edit([(0..0, "new\n")], cx).unwrap();
    });
    // Before the diff is computed again.
    assert_eq!(change(cx), "[4,5)");
    recompute(cx);
    let changes = diff.read_with(cx, |d, _| d.diff().unwrap().changes.len());
    assert_eq!(changes, 2);
}

/// Monaco `couldShowInlineViewBecauseOfSize` is `width <= breakpoint`.
#[gpui::test]
fn the_inline_breakpoint_is_inclusive(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    cx.simulate_resize(gpui::size(px(900.), px(600.)));
    paint(cx);
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.renders_side_by_side()));
}
