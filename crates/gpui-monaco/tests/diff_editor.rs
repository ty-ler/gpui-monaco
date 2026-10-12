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

/// Gives the modified editor a glyph margin (off by default, as in
/// standalone Monaco), where the margin revert arrows go.
fn with_glyph_margin(diff: &Entity<DiffEditor>, cx: &mut VisualTestContext) {
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    modified.update(cx, |e, cx| {
        e.update_options_with(|o| o.glyph_margin = true, cx)
    });
    paint(cx);
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
    diff.update(cx, |d, cx| d.go_to_diff(true, cx));
    assert_eq!(row(cx), PointUtf16::new(1, 0));
    diff.update(cx, |d, cx| d.go_to_diff(true, cx));
    assert_eq!(row(cx), PointUtf16::new(4, 0));
    // Wraps around.
    diff.update(cx, |d, cx| d.go_to_diff(true, cx));
    assert_eq!(row(cx), PointUtf16::new(1, 0));
    diff.update(cx, |d, cx| d.go_to_diff(false, cx));
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

/// Two changes far apart: line 6 replaced, line 31 deleted.
fn two_change_texts() -> (String, String) {
    let original: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let modified = original
        .replace("line 5\n", "five\n")
        .replace("line 30\n", "");
    (original, modified)
}

/// Monaco focuses the viewer's current row, so screen readers read it
/// (audit 4 §E.1): the focus is on the line list, which has a role, and the
/// current row is its active descendant.
#[gpui::test]
fn screen_readers_read_the_accessible_diff_viewer(cx: &mut TestAppContext) {
    use gpui_monaco::diff_editor::AccessibleDiffViewerAria;
    let (original, modified) = two_change_texts();
    let (diff, cx) = setup(&original, &modified, cx);
    let editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("f7");
    paint(cx);
    let aria = |cx: &mut VisualTestContext| -> AccessibleDiffViewerAria {
        diff.read_with(cx, |d, cx| d.accessible_viewer_aria_for_tests(cx))
            .expect("the viewer shows")
    };
    let state = aria(cx);
    assert_eq!(
        state.focused,
        (
            gpui::Role::List,
            "Accessible Diff Viewer. Use arrow up and down to navigate.".into()
        )
    );
    assert_eq!(
        state.active_descendant.as_deref(),
        Some("Difference 1 of 2: original line 6, 1 line changed, modified line 6, 1 line changed")
    );
    assert!(
        state
            .rows
            .iter()
            .all(|(role, _, _)| *role == gpui::Role::ListItem)
    );
    assert_eq!(
        state
            .rows
            .iter()
            .filter(|(_, _, selected)| *selected)
            .count(),
        1
    );
    assert!(state.rows[0].2);
    // Each Down makes the new row the one read.
    let expected = [
        "line 2 unchanged line 3",
        "line 3 unchanged line 4",
        "line 4 unchanged line 5",
        "- line 5 original line 6",
        "+ five modified line 6",
        "line 6 unchanged line 7",
    ];
    for (ix, label) in expected.into_iter().enumerate() {
        cx.simulate_keystrokes("down");
        let state = aria(cx);
        assert_eq!(state.active_descendant.as_deref(), Some(label));
        let selected: Vec<_> = state.rows.iter().map(|(_, _, s)| *s).collect();
        assert!(selected[ix + 1] && selected.iter().filter(|s| **s).count() == 1);
    }
    // F7: the next group's header.
    cx.simulate_keystrokes("f7");
    assert_eq!(
        aria(cx).active_descendant.as_deref(),
        Some(
            "Difference 2 of 2: original line 31, 1 line changed, modified line 31, no lines changed"
        )
    );
}

/// Monaco rebuilds the viewer's groups only for a new diff, and keeps the
/// current line (audit 4 §E.2).
#[gpui::test]
fn the_accessible_diff_viewer_keeps_its_place_across_re_applies(cx: &mut TestAppContext) {
    let (original, modified) = two_change_texts();
    let (diff, cx) = setup(&original, &modified, cx);
    cx.simulate_resize(gpui::size(px(1200.), px(600.)));
    paint(cx);
    let editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("f7");
    // The first group's last line, after the change: the modified cursor
    // follows it to line 9, past the change.
    cx.simulate_keystrokes("down down down down down down down down");
    let state = |cx: &mut VisualTestContext| {
        diff.read_with(cx, |d, cx| d.accessible_viewer_state_for_tests(cx))
    };
    assert_eq!(state(cx), Some((2, 0, 8)));
    // A theme change, an options change and a layout switch re-apply the
    // same diff.
    cx.update(|_, cx| gpui_monaco::set_theme(cx, "hc-black"));
    paint(cx);
    assert_eq!(state(cx), Some((2, 0, 8)));
    update_options(&diff, cx, |o| o.render_indicators = false);
    assert_eq!(state(cx), Some((2, 0, 8)));
    cx.simulate_resize(gpui::size(px(600.), px(600.)));
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.renders_side_by_side()));
    assert_eq!(state(cx), Some((2, 0, 8)));
}

/// Monaco's `setModel`: new models start with no groups, and no models
/// close the viewer (audit 4 §E.2).
#[gpui::test]
fn the_accessible_diff_viewer_follows_the_models(cx: &mut TestAppContext) {
    let (original, modified) = two_change_texts();
    let (diff, cx) = setup(&original, &modified, cx);
    let editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("f7");
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_state_for_tests(cx)),
        Some((2, 0, 0))
    );
    // Other models: the old groups go at once, the new diff's come.
    let groups = diff.update(cx, |d, cx| {
        let original = cx.new(|_| TextModel::new("a\nb"));
        let modified = cx.new(|_| TextModel::new("a\nc"));
        d.set_models(original, modified, cx);
        d.accessible_viewer_state_for_tests(cx)
    });
    assert_eq!(groups.map(|(groups, _, _)| groups), Some(0));
    paint(cx);
    assert!(diff.read_with(cx, |d, _| d.is_accessible_diff_viewer_visible()));
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_state_for_tests(cx)),
        Some((1, 0, 0))
    );
    assert_eq!(
        diff.read_with(cx, |d, cx| d.accessible_viewer_aria_for_tests(cx))
            .and_then(|aria| aria.active_descendant)
            .as_deref(),
        Some("Difference 1 of 1: original line 2, 1 line changed, modified line 2, 1 line changed")
    );
    // No models: it closes.
    diff.update(cx, |d, cx| d.set_model(None, cx));
    paint(cx);
    assert!(!diff.read_with(cx, |d, _| d.is_accessible_diff_viewer_visible()));
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
    with_glyph_margin(&diff, cx);
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
    let mut expected = compute_legacy_diff(
        &original.split('\n').collect::<Vec<_>>(),
        &modified.split('\n').collect::<Vec<_>>(),
        &DiffOptions::default(),
    );
    // Normalized (Monaco `normalizeDocumentDiff`): `b` -> `x` is the whole
    // line, so it takes the line break.
    let inner = &mut expected.changes[0].inner_changes[0];
    assert_eq!(inner.original_range.to_string(), "[2,1 -> 2,2]");
    for range in [&mut inner.original_range, &mut inner.modified_range] {
        range.end_line_number = 3;
        range.end_column = 1;
    }
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
    // Monaco `renderSideBySideInAutomaticMode` is true with a screen reader
    // at any width (audit 4 §E.3).
    let keys = diff.read_with(cx, |d, cx| d.context_keys(cx));
    assert!(
        keys.contains("diffEditorAutomaticRenderSideBySide"),
        "{keys:?}"
    );
    assert!(keys.contains("diffEditorRenderSideBySideInlineBreakpointReached"));
    assert!(!keys.contains("diffEditorInlineMode"));
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

/// Monaco's monitor stops when the capture is lost: the window going
/// inactive mid-drag ends the sash drag and gives the pointer back. A drag
/// in a window that isn't active (a click-through) goes on.
#[gpui::test]
fn the_window_going_inactive_ends_a_sash_drag(cx: &mut TestAppContext) {
    use gpui::{CursorStyle, Modifiers, MouseButton};
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    paint(cx);
    let captured =
        |cx: &mut VisualTestContext| diff.read_with(cx, |d, _| d.captured_cursor_for_tests());
    let start_drag = |cx: &mut VisualTestContext| {
        let sash = diff.read_with(cx, |d, _| d.sash_left_for_tests());
        let from = point(sash, px(300.));
        cx.simulate_mouse_move(from, None, Modifiers::default());
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            point(sash - px(50.), px(300.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        paint(cx);
    };
    // The test window isn't active: the drag goes on.
    start_drag(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeColumn));
    let sash = diff.read_with(cx, |d, _| d.sash_left_for_tests());
    cx.simulate_mouse_up(
        point(sash, px(300.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), None);
    // Active, then deactivated mid-drag: the drag ends.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    start_drag(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeColumn));
    cx.deactivate_window();
    paint(cx);
    assert_eq!(captured(cx), None);
}

/// Monaco captures the pointer on the overview ruler's slider: the arrow
/// over the whole window while it's dragged, given back on release.
#[gpui::test]
fn dragging_the_overview_slider_keeps_the_arrow_everywhere(cx: &mut TestAppContext) {
    use gpui::{CursorStyle, Modifiers, MouseButton};
    let text: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let (diff, cx) = setup(&text, &text.replace("line 50\n", "changed\n"), cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    paint(cx);
    let captured =
        |cx: &mut VisualTestContext| diff.read_with(cx, |d, _| d.captured_cursor_for_tests());
    let (_, modified) = editors(&diff, cx);
    let scroll = |cx: &mut VisualTestContext| modified.read_with(cx, |e, _| e.scroll_position().y);
    let ruler = cx
        .debug_bounds("diff-overview")
        .expect("the overview ruler");
    let from = point(ruler.center().x, ruler.top() + px(5.));
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::Arrow));
    cx.simulate_mouse_move(
        point(px(20.), ruler.center().y),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::Arrow));
    assert!(scroll(cx) > px(0.));
    cx.simulate_mouse_up(
        point(px(20.), px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), None);
}

/// Monaco captures the pointer on a hidden-lines bar's edge: its resize
/// cursor over the whole window while it's dragged.
#[gpui::test]
fn dragging_a_hidden_lines_edge_keeps_its_pointer_everywhere(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 50\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    hide_unchanged(&diff, cx);
    paint(cx);
    let captured =
        |cx: &mut VisualTestContext| diff.read_with(cx, |d, _| d.captured_cursor_for_tests());
    assert_eq!(captured(cx), None);
    let edge = cx
        .debug_bounds("unchanged-bottom")
        .expect("a bar's bottom edge");
    cx.simulate_mouse_move(edge.center(), None, Modifiers::default());
    cx.simulate_mouse_down(edge.center(), MouseButton::Left, Modifiers::default());
    paint(cx);
    let during = captured(cx);
    assert!(during.is_some());
    cx.simulate_mouse_move(
        point(px(20.), edge.center().y + px(60.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert!(captured(cx).is_some());
    cx.simulate_mouse_up(
        point(px(20.), px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
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
    with_glyph_margin(&diff, cx);
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

/// Monaco `IStandaloneDiffEditor.addCommand`: the keybinding works in this
/// diff editor, on either side, while `when` holds there, and the command
/// runs by id too, until disposed.
#[gpui::test]
fn diff_editor_add_command(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    let (original, modified) = editors(&diff, cx);
    let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = ran.clone();
    let (id, command) = diff
        .update(cx, |d, cx| {
            d.add_command(
                "ctrl-alt-k",
                move |d, _, cx| {
                    log.borrow_mut()
                        .push(d.modified_editor().read(cx).model().read(cx).text())
                },
                None,
                cx,
            )
        })
        .unwrap();
    assert!(id.starts_with("DYNAMIC_"));
    let focus = |editor: &Entity<gpui_monaco::CodeEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
        paint(cx);
    };
    focus(&modified, cx);
    cx.simulate_keystrokes("ctrl-alt-k");
    focus(&original, cx);
    cx.simulate_keystrokes("ctrl-alt-k");
    assert_eq!(*ran.borrow(), ["b", "b"]);
    diff.update_in(cx, |d, window, cx| d.trigger(&id, window, cx))
        .unwrap();
    assert_eq!(ran.borrow().len(), 3);
    cx.update(|_, cx| command.dispose(cx));
    cx.simulate_keystrokes("ctrl-alt-k");
    assert_eq!(ran.borrow().len(), 3);

    // `when` is tested in the focused side's keys: a key made on the
    // modified editor (Monaco's `createContextKey` delegates there too).
    let key = modified.update(cx, |e, cx| e.create_context_key("myMode", true, cx));
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = count.clone();
    let _command = diff
        .update(cx, |d, cx| {
            d.add_command(
                "ctrl-alt-j",
                move |_, _, _| counter.set(counter.get() + 1),
                Some("myMode"),
                cx,
            )
        })
        .unwrap();
    paint(cx);
    cx.simulate_keystrokes("ctrl-alt-j");
    assert_eq!(count.get(), 0);
    focus(&modified, cx);
    cx.simulate_keystrokes("ctrl-alt-j");
    assert_eq!(count.get(), 1);
    cx.update(|_, cx| key.set(false, cx));
    paint(cx);
    cx.simulate_keystrokes("ctrl-alt-j");
    assert_eq!(count.get(), 1);
    // Bad keystrokes or conditions fail.
    assert!(
        diff.update(cx, |d, cx| d.add_command(
            "ctrl-alt-j",
            |_, _, _| {},
            Some("&&"),
            cx
        ))
        .is_err()
    );
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
    diff.update(cx, |d, cx| d.go_to_diff(true, cx));
    diff.update(cx, |d, cx| d.go_to_diff(true, cx));
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

/// Monaco `Sash`: the diff sash shows `sash.hoverBorder` after the mouse
/// rests on it for 300ms (`hoverDelay`), and at once while dragged.
#[gpui::test]
fn the_sash_hover_border_waits_for_the_hover_delay(cx: &mut TestAppContext) {
    use gpui::Modifiers;
    use std::time::Duration;
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    paint(cx);
    let shown =
        |cx: &mut VisualTestContext| diff.read_with(cx, |d, _| d.sash_hover_for_tests("diff-sash"));
    let on_sash = point(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(300.));
    cx.simulate_mouse_move(on_sash, None, Modifiers::default());
    paint(cx);
    assert!(!shown(cx));
    cx.executor().advance_clock(Duration::from_millis(299));
    paint(cx);
    assert!(!shown(cx));
    cx.executor().advance_clock(Duration::from_millis(1));
    paint(cx);
    assert!(shown(cx));
    cx.simulate_mouse_move(point(px(300.), px(300.)), None, Modifiers::default());
    paint(cx);
    assert!(!shown(cx));
}

fn set_cursor(editor: &Entity<gpui_monaco::CodeEditor>, row: u32, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                row, 0,
            ))],
            cx,
        )
    });
}

/// Monaco `normalizeDocumentDiff`: a whole-line change ends on the next
/// line, so its highlight fills to the right edge and the true inline view
/// shows the deleted line instead of striking it through.
#[gpui::test]
fn whole_line_changes_are_normalized(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nfoo\nb", "a\nbar\nb", cx);
    let inner = diff.read_with(cx, |d, _| {
        d.diff().unwrap().changes[0].inner_changes[0]
            .modified_range
            .to_string()
    });
    assert_eq!(inner, "[2,1 -> 3,1]");
    let fills = side_decorations(&diff, true, cx)
        .into_iter()
        .filter(|(_, options)| options.should_fill_line_on_line_break)
        .map(|(range, _)| range)
        .collect::<Vec<_>>();
    assert_eq!(fills, vec![(2, 1, 3, 1)]);
    update_options(&diff, cx, |o| {
        o.render_side_by_side = false;
        o.use_true_inline_view = true;
    });
    // The deleted line in a zone, nothing struck through.
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (1, 1)
    );
    assert!(
        side_decorations(&diff, true, cx)
            .iter()
            .all(|(_, options)| options.before.is_none())
    );
}

/// Monaco `revertFocusedRangeMappings`: the cursor's line range is empty,
/// and touching line ranges intersect, so the cursor below a deletion or on
/// the line after a change reverts it.
#[gpui::test]
fn the_revert_action_reverts_a_deletion_above_the_cursor(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd\ne\nf", "a\nb\ne\nf", cx);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&modified.read(cx).focus_handle(cx), cx));
    // The cursor on `e`, right below the deleted `c` and `d`.
    set_cursor(&modified, 2, cx);
    cx.dispatch_action(gpui_monaco::diff_editor::Revert);
    assert_eq!(modified_text(&diff, cx), "a\nb\nc\nd\ne\nf");
}

#[gpui::test]
fn the_revert_action_reverts_a_change_ending_at_the_cursor(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd", "a\nX\nc\nd", cx);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&modified.read(cx).focus_handle(cx), cx));
    set_cursor(&modified, 2, cx);
    cx.dispatch_action(gpui_monaco::diff_editor::Revert);
    assert_eq!(modified_text(&diff, cx), "a\nb\nc\nd");
}

/// Monaco `Range.areIntersecting`: a selection that ends where a changed
/// span starts doesn't select it, so the change keeps its own arrow.
#[gpui::test]
fn the_revert_selection_arrow_needs_a_real_intersection(cx: &mut TestAppContext) {
    let (diff, cx) = setup("x\none two\n", "x\none TWO\n", cx);
    with_glyph_margin(&diff, cx);
    update_options(&diff, cx, |o| o.render_gutter_menu = false);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    // Selected backwards from `one |TWO` up to the first line, so the
    // selection's arrow is on line 1 and the change's on line 2.
    let select_to = |column: u32, cx: &mut VisualTestContext| {
        modified.update(cx, |e, cx| {
            e.set_selections(
                &[gpui_monaco::cursor::Selection::new(
                    PointUtf16::new(1, column),
                    PointUtf16::new(0, 0),
                )],
                cx,
            )
        });
        paint(cx);
        let arrows = diff.read_with(cx, |d, cx| d.revert_arrow_bounds_for_tests(cx));
        assert_eq!(arrows.len(), 1);
        arrows[0].top()
    };
    let touching = select_to(4, cx);
    let intersecting = select_to(5, cx);
    assert!(intersecting < touching, "{intersecting:?} {touching:?}");
}

/// Monaco `setActiveMovedTextFromCursor`: moving the cursor off the
/// compared block stops comparing it, and a new diff keeps comparing it
/// only while the cursor is in it.
#[gpui::test]
fn comparing_a_moved_block_follows_the_cursor(cx: &mut TestAppContext) {
    let block: String = (0..6)
        .map(|i| format!("fn moved_{i}() {{ body_{i}(); }}\n"))
        .collect();
    let filler: String = (0..10).map(|i| format!("let x{i} = {i};\n")).collect();
    let (diff, cx) = setup(&format!("{block}{filler}"), &format!("{filler}{block}"), cx);
    update_options(&diff, cx, |o| o.show_moves = true);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    let comparing = |cx: &mut VisualTestContext| diff.read_with(cx, |d, _| d.move_to_compare());
    // In the block (lines 11-16 on the right).
    set_cursor(&modified, 12, cx);
    diff.update(cx, |d, cx| d.set_move_to_compare(Some(0), cx));
    set_cursor(&modified, 13, cx);
    assert_eq!(comparing(cx), Some(0));
    // An edit elsewhere: the new diff still has it, and the cursor is in it.
    model_edit_end(&diff, "// end\n", cx);
    recompute(cx);
    assert_eq!(comparing(cx), Some(0));
    // Off the block.
    set_cursor(&modified, 2, cx);
    assert_eq!(comparing(cx), None);
    // Comparing with the cursor elsewhere lasts until the next diff.
    diff.update(cx, |d, cx| d.set_move_to_compare(Some(0), cx));
    assert_eq!(comparing(cx), Some(0));
    model_edit_end(&diff, "// more\n", cx);
    recompute(cx);
    assert_eq!(comparing(cx), None);
}

/// Monaco `_adjustOptionsForSubEditor`: no variable line heights, which
/// alignment can't account for.
#[gpui::test]
fn sub_editors_have_no_variable_line_heights(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    let allowed = diff.read_with(cx, |d, cx| {
        [d.original_editor(), d.modified_editor()]
            .map(|e| e.read(cx).options().allow_variable_line_heights)
    });
    assert_eq!(allowed, [false, false]);
}

/// Monaco's `StableEditorScrollState` around the modified editor's view
/// zones: padding added above the viewport doesn't move the text.
#[gpui::test]
fn realignment_keeps_the_modified_side_still(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (diff, cx) = setup(&text, &text, cx);
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    modified.update(cx, |e, cx| {
        e.set_scroll_position(point(px(0.), px(1005.)), cx)
    });
    paint(cx);
    let first_visible = |cx: &mut VisualTestContext| {
        modified.read_with(cx, |e, cx| e.visible_ranges(cx)[0].start.row)
    };
    let top_of = |row: u32, cx: &mut VisualTestContext| {
        modified.read_with(cx, |e, _| {
            e.top_for_line_number(row, false).unwrap() - e.scroll_position().y
        })
    };
    let row = first_visible(cx);
    assert!(row > 20, "{row}");
    let top = top_of(row, cx);
    // Five lines more near the top of the original: the modified side
    // gets five lines of padding above the viewport.
    let original = diff.read_with(cx, |d, cx| d.original_editor().read(cx).model().clone());
    original.update(cx, |m, cx| {
        let at = m.text().find("line 5\n").unwrap();
        m.edit([(at..at, "new\n".repeat(5))], cx).unwrap();
    });
    recompute(cx);
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.alignment_zones_for_tests()),
        (0, 1)
    );
    assert_eq!(first_visible(cx), row);
    assert_eq!(top_of(row, cx), top);
}

/// Monaco's "collapse touching unchanged ranges": a region split around
/// lines that were visible merges back once its parts touch, so one bar
/// shows.
#[gpui::test]
fn touching_unchanged_regions_merge(cx: &mut TestAppContext) {
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original
        .replace("line 20\n", "changed\n")
        .replace("line 50\n", "changed too\n");
    let (diff, cx) = setup(&original, &modified, cx);
    hide_unchanged(&diff, cx);
    let hidden = |cx: &mut VisualTestContext| {
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests())
    };
    assert_eq!(hidden(cx), vec![1..18, 25..48, 55..102]);
    // Reverting the first change keeps the lines around it visible: the
    // new region splits around them.
    let model = diff.read_with(cx, |d, cx| d.modified_editor().read(cx).model().clone());
    model.update(cx, |m, cx| {
        let offset = m.text().find("changed\n").unwrap();
        m.edit([(offset..offset + 7, "line 20")], cx).unwrap();
    });
    recompute(cx);
    // (The edit's cursor at line 1 also shows that line.)
    assert_eq!(hidden(cx)[1..], [25..48, 55..102]);
    assert_eq!(
        diff.read_with(cx, |d, _| d.unchanged_bars_for_tests()),
        (3, 3)
    );
    // Collapsing makes the parts touch: they merge, with one bar.
    diff.update(cx, |d, cx| d.collapse_all_unchanged_regions(cx));
    paint(cx);
    assert_eq!(hidden(cx), vec![1..48, 55..102]);
    assert_eq!(
        diff.read_with(cx, |d, _| d.unchanged_bars_for_tests()),
        (2, 2)
    );
}

/// Monaco `shouldHideControls` is false while the bar is dragged: a drag
/// that shows every line keeps the bar until the mouse is released. The
/// edges are 10px bands straddling the bar's top and bottom.
#[gpui::test]
fn a_dragged_bar_stays_until_released(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let original: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 35\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    hide_unchanged(&diff, cx);
    paint(cx);
    let bars = |cx: &mut VisualTestContext| diff.read_with(cx, |d, _| d.unchanged_bars_for_tests());
    assert_eq!(bars(cx), (1, 1));
    let top = cx.debug_bounds("unchanged-top").expect("a bar's top edge");
    let bottom = cx
        .debug_bounds("unchanged-bottom")
        .expect("a bar's bottom edge");
    // Monaco's `.top`/`.bottom`: 6px above the bar's top to 4px below it,
    // and 6px above its bottom to 4px below.
    assert_eq!(top.size.height, px(10.));
    assert_eq!(bottom.size.height, px(10.));
    assert_eq!(bottom.top() - top.top(), px(24.));
    let (_, modified_editor) = editors(&diff, cx);
    let line_height = modified_editor.read_with(cx, |e, _| e.options().resolved_line_height());
    // The bar is at the top: grab its top edge's lower half.
    let start = point(px(300.), top.top() + px(8.));
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    paint(cx);
    cx.simulate_mouse_move(
        point(start.x, start.y + line_height * 40.),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert!(diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
    assert_eq!(bars(cx), (1, 1));
    assert_eq!(diff.read_with(cx, |d, _| d.fold_glyphs_for_tests()), (0, 0));
    cx.simulate_mouse_up(
        point(start.x, start.y + line_height * 40.),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(bars(cx), (0, 0));
    assert_eq!(diff.read_with(cx, |d, _| d.fold_glyphs_for_tests()), (1, 1));
}

/// Monaco `_runWithLowerScrollAnchor` measures the anchor's top, so with
/// word wrap the lines below a bar stay put as wrapped lines show. (Inline,
/// so no padding for the other side's wrapping comes after.)
#[gpui::test]
fn the_lower_scroll_anchor_counts_wrapped_lines(cx: &mut TestAppContext) {
    let long = "x ".repeat(200);
    let original: String = (0..60).map(|i| format!("line {i} {long}\n")).collect();
    let modified = original.replace(&format!("line 50 {long}\n"), "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    update_options(&diff, cx, |o| {
        o.diff_word_wrap = gpui_monaco::options::WordWrapOverride::On;
        o.render_side_by_side = false;
    });
    hide_unchanged(&diff, cx);
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests())[0],
        1..48
    );
    let (_, modified_editor) = editors(&diff, cx);
    let line_height = modified_editor.read_with(cx, |e, _| e.options().resolved_line_height());
    // Line 48, the first after the region.
    let anchor = |cx: &mut VisualTestContext| {
        modified_editor.read_with(cx, |e, _| {
            e.top_for_line_number(47, false).unwrap() - e.scroll_position().y
        })
    };
    let (before, scroll_before) = (
        anchor(cx),
        modified_editor.read_with(cx, |e, _| e.scroll_position().y),
    );
    diff.update(cx, |d, cx| {
        d.drag_unchanged_for_tests(0, false, px(100.), px(100.) - line_height * 2., cx)
    });
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests())[0],
        1..46
    );
    let scrolled = modified_editor.read_with(cx, |e, _| e.scroll_position().y) - scroll_before;
    // The two lines shown wrap.
    assert!(scrolled > line_height * 4., "{scrolled:?}");
    assert_eq!(anchor(cx), before);
}

/// Monaco's `PlaceholderViewZone` has `showInHiddenAreas`: a bar between
/// two changes shows, though the line after it is hidden.
#[gpui::test]
fn bars_between_changes_show(cx: &mut TestAppContext) {
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 50\n", "changed\n");
    let (diff, cx) = setup(&original, &modified, cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    hide_unchanged(&diff, cx);
    paint(cx);
    // The last bar painted is the second region's, after line 54.
    let (_, modified_editor) = editors(&diff, cx);
    let line_height = modified_editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let top = cx.debug_bounds("unchanged-top").expect("a bar's top edge");
    assert!(top.top() > line_height * 7., "{top:?}");
}

/// Monaco's diff editor context keys (`diffEditorWidget.ts`,
/// `DiffEditorEditors`): on the diff editor and inherited by both editors,
/// as from Monaco's scoped context key service.
#[gpui::test]
fn context_keys(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb", "a\nc", cx);
    let (original, modified) = editors(&diff, cx);
    cx.simulate_resize(gpui::size(px(1200.), px(600.)));
    paint(cx);
    for keys in [
        diff.read_with(cx, |d, cx| d.context_keys(cx)),
        original.read_with(cx, |e, cx| e.context_keys(cx)),
        modified.read_with(cx, |e, cx| e.context_keys(cx)),
    ] {
        for key in [
            "isInDiffEditor",
            "inDiffEditor",
            "diffEditorHasChanges",
            "diffEditorModifiedWritable",
            "diffEditorAutomaticRenderSideBySide",
        ] {
            assert!(keys.contains(key), "{key} in {keys:?}");
        }
        for key in [
            "diffEditorOriginalWritable",
            "diffEditorInlineMode",
            "diffEditorRenderSideBySideInlineBreakpointReached",
            "comparingMovedCode",
            "isEmbeddedDiffEditor",
            "inMultiDiffEditor",
        ] {
            assert!(!keys.contains(key), "{key} in {keys:?}");
        }
        assert!(
            keys.get("diffEditorModifiedUri")
                .is_some_and(|uri| uri.starts_with("inmemory://model/")),
            "{keys:?}"
        );
    }
    // The focused side.
    let focus = |editor: &Entity<gpui_monaco::CodeEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
        paint(cx);
    };
    focus(&original, cx);
    let keys = modified.read_with(cx, |e, cx| e.context_keys(cx));
    assert!(keys.contains("isInDiffLeftEditor") && !keys.contains("isInDiffRightEditor"));
    focus(&modified, cx);
    let keys = original.read_with(cx, |e, cx| e.context_keys(cx));
    assert!(!keys.contains("isInDiffLeftEditor") && keys.contains("isInDiffRightEditor"));
    // Options.
    update_options(&diff, cx, |o| {
        o.original_editable = true;
        o.is_in_embedded_editor = true;
    });
    let keys = original.read_with(cx, |e, cx| e.context_keys(cx));
    assert!(keys.contains("diffEditorOriginalWritable") && keys.contains("isEmbeddedDiffEditor"));
    // Narrow: the breakpoint is reached and the diff shows inline.
    cx.simulate_resize(gpui::size(px(600.), px(600.)));
    paint(cx);
    let keys = modified.read_with(cx, |e, cx| e.context_keys(cx));
    assert!(keys.contains("diffEditorRenderSideBySideInlineBreakpointReached"));
    assert!(keys.contains("diffEditorInlineMode"));
    assert!(!keys.contains("diffEditorAutomaticRenderSideBySide"));
    // A read-only modified side; no changes.
    modified.update(cx, |e, cx| {
        e.update_options_with(|o| o.read_only = true, cx)
    });
    let model = modified.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| m.set_value("a\nb", cx));
    recompute(cx);
    let keys = modified.read_with(cx, |e, cx| e.context_keys(cx));
    assert!(!keys.contains("diffEditorModifiedWritable"));
    assert!(!keys.contains("diffEditorHasChanges"));
}

/// The audit's repro: `add_command`'s `when` sees the diff editor's keys.
#[gpui::test]
fn add_command_when_tests_diff_editor_keys(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "a", cx);
    let (_, modified) = editors(&diff, cx);
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = count.clone();
    let _command = diff
        .update(cx, |d, cx| {
            d.add_command(
                "ctrl-alt-j",
                move |_, _, _| counter.set(counter.get() + 1),
                Some("isInDiffEditor && diffEditorHasChanges"),
                cx,
            )
        })
        .unwrap();
    cx.update(|window, cx| window.focus(&modified.focus_handle(cx), cx));
    paint(cx);
    cx.simulate_keystrokes("ctrl-alt-j");
    assert_eq!(count.get(), 0);
    let model = modified.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| m.set_value("b", cx));
    recompute(cx);
    cx.simulate_keystrokes("ctrl-alt-j");
    assert_eq!(count.get(), 1);
}

/// Monaco `_goTo` only moves the modified editor's cursor; Alt+F5 is
/// VS Code's workbench key, not standalone Monaco's.
#[gpui::test]
fn go_to_diff_keeps_the_focus_and_is_unbound(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc", "a\nB\nc", cx);
    let (original, modified) = editors(&diff, cx);
    cx.update(|window, cx| window.focus(&original.focus_handle(cx), cx));
    paint(cx);
    diff.update(cx, |d, cx| d.go_to_diff(true, cx));
    paint(cx);
    assert_eq!(
        modified.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(1, 0)
    );
    assert!(cx.update(|window, cx| original.focus_handle(cx).is_focused(window)));
    // No default keys.
    cx.update(|window, cx| window.focus(&modified.focus_handle(cx), cx));
    paint(cx);
    cx.simulate_keystrokes("alt-f5");
    cx.simulate_keystrokes("shift-alt-f5");
    assert_eq!(
        modified.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(1, 0)
    );
}

/// Monaco `IDiffEditor.onDidContentSizeChange`: the taller side's height,
/// and both widths plus the 30px overview ruler.
#[gpui::test]
fn content_size_changed(cx: &mut TestAppContext) {
    use gpui_monaco::{ContentSizeChange, DiffEditorEvent};
    let (diff, cx) = setup("a", "b", cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::<ContentSizeChange>::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&diff, move |_, event: &DiffEditorEvent, _| {
            if let DiffEditorEvent::ContentSizeChanged(change) = event {
                sink.borrow_mut().push(*change);
            }
        })
        .detach()
    });
    let (original, modified) = editors(&diff, cx);
    let model = modified.read_with(cx, |e, _| e.model().clone());
    let text: String = (0..30).map(|i| format!("line {i}\n")).collect();
    model.update(cx, |m, cx| m.set_value(&text, cx));
    recompute(cx);
    let last = *events.borrow().last().expect("a content size change");
    assert!(events.borrow().iter().any(|e| e.content_height_changed));
    let height = diff.read_with(cx, |d, cx| d.content_height(cx)).unwrap();
    assert_eq!(last.content_height, height);
    assert!(height >= modified.read_with(cx, |e, _| e.content_height()).unwrap());
    let width = |editor: &Entity<gpui_monaco::CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.content_width()).unwrap()
    };
    assert_eq!(
        last.content_width,
        width(&original, cx) + width(&modified, cx) + px(30.)
    );
    assert_eq!(
        diff.read_with(cx, |d, cx| d.content_width(cx)),
        last.content_width
    );
}

/// Monaco's `.line-insert`/`.char-insert` (and `-delete`) borders:
/// `diffEditor.insertedTextBorder`/`removedTextBorder` on whole lines too,
/// dashed in high contrast themes.
#[gpui::test]
fn high_contrast_change_borders(cx: &mut TestAppContext) {
    use gpui::BorderStyle;
    let (diff, cx) = setup("a\nb\nc", "a\nB\nc\nnew", cx);
    let bordered = |modified: bool, cx: &mut VisualTestContext| {
        side_decorations(&diff, modified, cx)
            .into_iter()
            .filter(|(_, o)| o.border.is_some())
            .map(|(range, o)| (range, o.is_whole_line, o.border_style))
            .collect::<Vec<_>>()
    };
    // `vs` has no borders.
    assert!(bordered(true, cx).is_empty());
    cx.update(|_, cx| gpui_monaco::set_theme(cx, "hc-black"));
    paint(cx);
    let modified = bordered(true, cx);
    // The changed and the inserted lines, whole and by character.
    assert!(
        modified
            .iter()
            .any(|(range, whole, _)| *whole && range.0 == 4),
        "{modified:?}"
    );
    assert!(
        modified
            .iter()
            .any(|(range, whole, _)| !*whole && range.0 == 2),
        "{modified:?}"
    );
    assert!(
        modified
            .iter()
            .all(|(_, _, style)| *style == BorderStyle::Dashed)
    );
    let original = bordered(false, cx);
    assert!(
        original
            .iter()
            .any(|(range, whole, _)| *whole && range.0 == 2),
        "{original:?}"
    );
}

/// Monaco `validateDiffEditorOptions`: `clampedFloat(splitViewDefaultRatio,
/// 0.5, 0.1, 0.9)`.
#[gpui::test]
fn split_view_default_ratio_is_clamped(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    for (ratio, clamped) in [(0.98, 0.9), (0.02, 0.1), (0.3, 0.3), (f32::NAN, 0.5)] {
        update_options(&diff, cx, |o| o.split_view_default_ratio = ratio);
        assert_eq!(
            diff.read_with(cx, |d, _| d.options().split_view_default_ratio),
            clamped
        );
    }
}

// ----- Hidden unchanged regions (#404) --------------------------------------

/// `count` lines `line N` (1-based), with `changed N` at `changed`.
fn numbered_lines(count: usize, changed: &[usize]) -> String {
    (1..=count)
        .map(|i| {
            if changed.contains(&i) {
                format!("changed {i}\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect()
}

/// Replaces what `edit` picks in the modified text and recomputes the diff.
fn edit_modified(
    diff: &Entity<DiffEditor>,
    cx: &mut VisualTestContext,
    edit: impl FnOnce(&str) -> (std::ops::Range<usize>, String),
) {
    let (_, modified) = editors(diff, cx);
    let model = modified.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| {
        let (range, text) = edit(&m.text());
        m.edit([(range, text)], cx).unwrap();
    });
    recompute(cx);
}

/// Monaco `filterWithPrevious` and `LineRangeMapping.clip`: an edit that
/// collapses the last two regions onto one line drops the second, rather
/// than building an inverted range (which panicked in debug builds).
#[gpui::test]
fn edits_collapsing_unchanged_regions_onto_one_line(cx: &mut TestAppContext) {
    let (diff, cx) = setup(
        &numbered_lines(100, &[]),
        &numbered_lines(100, &[30, 70]),
        cx,
    );
    hide_unchanged(&diff, cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![1..27, 34..67, 74..102]
    );
    // Line 32 to the end becomes `x`.
    edit_modified(&diff, cx, |text| {
        (text.find("line 32\n").unwrap()..text.len(), "x".into())
    });
    // Only the first region is left (the edit's cursor at line 1 shows that
    // line).
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![2..27]
    );
}

/// Monaco: once a diff showed everything, new regions start revealed, with
/// only the fold glyph; only the first diff's regions start collapsed.
#[gpui::test]
fn new_regions_after_none_start_revealed(cx: &mut TestAppContext) {
    let (diff, cx) = setup(
        &numbered_lines(29, &[]),
        &numbered_lines(29, &[5, 10, 15, 20, 25]),
        cx,
    );
    hide_unchanged(&diff, cx);
    // 30 lines (with the last, empty one): every gap is too short to hide.
    assert!(diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
    for line in [10, 15, 20] {
        edit_modified(&diff, cx, |text| {
            let changed = format!("changed {line}\n");
            let start = text.find(&changed).unwrap();
            (start..start + changed.len(), format!("line {line}\n"))
        });
    }
    // The new region, lines 9 to 21, stays shown.
    assert!(diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
    assert_eq!(diff.read_with(cx, |d, _| d.fold_glyphs_for_tests()), (1, 1));
    diff.update(cx, |d, cx| d.collapse_all_unchanged_regions(cx));
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![9..22]
    );
}

/// Monaco: identical models keep their whole-file region, which the view
/// skips, and its state carries over to the next diff's regions.
#[gpui::test]
fn the_whole_file_region_keeps_its_state(cx: &mut TestAppContext) {
    let text = numbered_lines(100, &[]);
    let (diff, cx) = setup(&text, &text, cx);
    hide_unchanged(&diff, cx);
    assert!(diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests().is_empty()));
    assert_eq!(diff.read_with(cx, |d, _| d.fold_glyphs_for_tests()), (0, 0));
    assert_eq!(
        diff.read_with(cx, |d, _| d.unchanged_bars_for_tests()),
        (0, 0)
    );
    // The cursor reveals line 40 in it.
    let (_, modified) = editors(&diff, cx);
    modified.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                39, 0,
            ))],
            cx,
        )
    });
    // After a change at line 80, line 40 still shows; the rest started
    // hidden.
    edit_modified(&diff, cx, |text| {
        let start = text.find("line 80\n").unwrap();
        (start..start + 4, "changed".into())
    });
    let hidden = diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests());
    assert!(hidden.iter().all(|r| !r.contains(&40)), "{hidden:?}");
    assert_eq!(hidden.last(), Some(&(84..102)), "{hidden:?}");
}

/// Monaco `_hide = !sideBySide`: inline, the original column's bars have no
/// unfold button or label.
#[gpui::test]
fn inline_original_bars_hide_their_controls(cx: &mut TestAppContext) {
    let (diff, cx) = setup(&numbered_lines(100, &[]), &numbered_lines(100, &[50]), cx);
    cx.simulate_resize(gpui::size(px(1230.), px(600.)));
    hide_unchanged(&diff, cx);
    assert!(cx.debug_bounds("unchanged-unfold-original").is_some());
    assert!(cx.debug_bounds("unchanged-unfold-modified").is_some());
    update_options(&diff, cx, |o| o.render_side_by_side = false);
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, _| d.unchanged_bars_for_tests()),
        (2, 2)
    );
    assert!(cx.debug_bounds("unchanged-unfold-original").is_none());
    assert!(cx.debug_bounds("unchanged-unfold-modified").is_some());
}

/// Monaco skips the "Fold Unchanged Region" glyph only in its multi diff
/// `compact` variant, not with `compactMode`.
#[gpui::test]
fn compact_mode_keeps_the_fold_glyph(cx: &mut TestAppContext) {
    let (diff, cx) = setup(&numbered_lines(100, &[]), &numbered_lines(100, &[50]), cx);
    update_options(&diff, cx, |o| o.compact_mode = true);
    hide_unchanged(&diff, cx);
    diff.update(cx, |d, cx| d.show_all_unchanged_regions(cx));
    paint(cx);
    assert_eq!(diff.read_with(cx, |d, _| d.fold_glyphs_for_tests()), (2, 2));
    diff.update(cx, |d, cx| d.fold_unchanged_for_tests(0, cx));
    assert_eq!(
        diff.read_with(cx, |d, _| d.hidden_modified_ranges_for_tests()),
        vec![1..47]
    );
}

/// Monaco `renderLines`: without character decorations (the legacy diff's
/// pure deletions) each deleted view line is a full-width `char-delete`.
#[gpui::test]
fn inline_deleted_lines_without_inner_changes_tint_full_width(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nb\nc\nd\ne", "a\ne", cx);
    update_options(&diff, cx, |o| {
        o.render_side_by_side = false;
        o.diff_algorithm = gpui_monaco::DiffAlgorithm::Legacy;
    });
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, cx| d.deleted_code_full_width_for_tests(cx)),
        vec![true]
    );
}

/// A changed line's inner changes are marked instead of the whole line.
#[gpui::test]
fn inline_deleted_lines_with_inner_changes_mark_them(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a\nfoo bar\nc", "a\nfoo baz\nc", cx);
    update_options(&diff, cx, |o| o.render_side_by_side = false);
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, cx| d.deleted_code_full_width_for_tests(cx)),
        vec![false]
    );
}

/// Monaco `EditorGutter`'s `.scroll-decoration`: shown while the modified
/// editor isn't scrolled to the top.
#[gpui::test]
fn gutter_scroll_decoration_follows_the_modified_scroll_top(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (diff, cx) = setup(&text, &text.replace("line 5\n", "line five\n"), cx);
    assert!(diff.read_with(cx, |d, cx| d.shows_gutter_for_tests(cx)));
    assert!(!diff.read_with(cx, |d, cx| d.gutter_scroll_decoration_for_tests(cx)));
    let (_, modified) = editors(&diff, cx);
    modified.update(cx, |e, cx| {
        e.set_scroll_position(point(px(0.), px(100.)), cx)
    });
    paint(cx);
    assert!(diff.read_with(cx, |d, cx| d.gutter_scroll_decoration_for_tests(cx)));
    modified.update(cx, |e, cx| e.set_scroll_position(point(px(0.), px(0.)), cx));
    paint(cx);
    assert!(!diff.read_with(cx, |d, cx| d.gutter_scroll_decoration_for_tests(cx)));
}

// ----- Diff editor polish (#424) --------------------------------------------

fn line_height(diff: &Entity<DiffEditor>, cx: &mut VisualTestContext) -> gpui::Pixels {
    diff.read_with(cx, |d, cx| {
        d.modified_editor()
            .read(cx)
            .options()
            .resolved_line_height()
    })
}

fn new_model(text: &str, cx: &mut VisualTestContext) -> Entity<TextModel> {
    let text = text.to_string();
    cx.update(|_, cx| cx.new(|_| TextModel::new(&text)))
}

/// Monaco `EditorGutter.render`: an item spans the view zones above its
/// first line and below its last, here the inline view's deleted lines
/// above and the side by side view's padding below.
#[gpui::test]
fn gutter_items_span_the_zones_next_to_a_change(cx: &mut TestAppContext) {
    let original = "1\n2\n3\n4\n5\n6\nsame\nsame2";
    let (diff, cx) = setup(original, "x\nsame\nsame2", cx);
    let lh = line_height(&diff, cx);
    // Side by side: the one modified line and five lines of padding.
    let spans = diff.read_with(cx, |d, cx| d.gutter_spans_for_tests(cx));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].1 - spans[0].0, lh * 6.);
    // Inline: the six deleted lines above and the modified line.
    update_options(&diff, cx, |o| o.render_side_by_side = false);
    let spans = diff.read_with(cx, |d, cx| d.gutter_spans_for_tests(cx));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0], (px(0.), lh * 7.));
}

/// Monaco `computeLineStart`: a moved block's line starts halfway from the
/// top of its "Code moved" caption to the bottom of its last line.
#[gpui::test]
fn moved_block_lines_start_at_the_caption(cx: &mut TestAppContext) {
    let filler: String = (0..10).map(|i| format!("let x{i} = {i};\n")).collect();
    let block: String = (0..3)
        .map(|i| format!("fn moved_{i}() {{ body_{i}(); }}\n"))
        .collect();
    let (diff, cx) = setup(&format!("{block}{filler}"), &format!("{filler}{block}"), cx);
    update_options(&diff, cx, |o| o.show_moves = true);
    let lh = line_height(&diff, cx);
    let (original, modified) = editors(&diff, cx);
    let ys = diff.read_with(cx, |d, cx| d.move_line_ys_for_tests(cx));
    assert_eq!(ys.len(), 1);
    let span = |editor: &Entity<gpui_monaco::CodeEditor>, row: u32, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            let scroll_top = e.scroll_position().y;
            let top = e.top_for_line_number(row, true).unwrap() - scroll_top;
            let bottom = e.top_for_line_number(row, false).unwrap() + lh - scroll_top;
            (top, bottom)
        })
    };
    // The caption is above the line on both sides.
    let (top, _) = span(&original, 0, cx);
    let (_, bottom) = span(&original, 2, cx);
    assert!(bottom - top > lh * 3., "{top:?} {bottom:?}");
    assert_eq!(ys[0].0, (top + bottom) / 2.);
    let (top, _) = span(&modified, 10, cx);
    let (_, bottom) = span(&modified, 12, cx);
    assert!(bottom - top > lh * 3., "{top:?} {bottom:?}");
    assert_eq!(ys[0].1, (top + bottom) / 2.);
}

/// Monaco `RevertButton.getPosition`: the right lane at z-index 10001, so a
/// host's glyph on the changed line doesn't hide the arrow.
#[gpui::test]
fn revert_arrows_take_the_right_lane_over_host_glyphs(cx: &mut TestAppContext) {
    use gpui_monaco::decorations::{GlyphMarginLane, GlyphMarginOptions};
    let (diff, cx) = setup("a\nb\nc\nd\ne", "a\nB\nc\nd\ne", cx);
    with_glyph_margin(&diff, cx);
    update_options(&diff, cx, |o| o.render_gutter_menu = false);
    let (_, modified) = editors(&diff, cx);
    let id = modified.entity_id();
    let model = modified.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| {
        m.delta_decorations(
            Some(id),
            &[],
            vec![(
                2..2,
                gpui_monaco::DecorationOptions {
                    glyph: Some(("●".into(), gpui::red())),
                    glyph_margin: Some(GlyphMarginOptions {
                        position: GlyphMarginLane::Center,
                        persist_lane: false,
                    }),
                    z_index: 100,
                    ..Default::default()
                },
            )],
            cx,
        );
    });
    paint(cx);
    assert_eq!(
        diff.read_with(cx, |d, cx| d.revert_arrow_bounds_for_tests(cx))
            .len(),
        1
    );
}

/// Monaco `_adjustOptionsForRightHandSide`: the modified editor reveals
/// with the overview ruler's width more padding and has no scrollbar
/// arrows, whatever the host sets.
#[gpui::test]
fn the_modified_editor_overrides_reveal_padding_and_scrollbar_arrows(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    let (original, modified) = editors(&diff, cx);
    let options = modified.read_with(cx, |e, _| e.options().clone());
    assert_eq!(options.reveal_horizontal_right_padding, px(45.));
    assert!(!options.scrollbar.vertical_has_arrows);
    set_editor_options(&modified, cx, |o| {
        o.reveal_horizontal_right_padding = px(5.);
        o.scrollbar.vertical_has_arrows = true;
    });
    let options = modified.read_with(cx, |e, _| e.options().clone());
    assert_eq!(options.reveal_horizontal_right_padding, px(45.));
    assert!(!options.scrollbar.vertical_has_arrows);
    // The original editor keeps the default.
    let options = original.read_with(cx, |e, _| e.options().clone());
    assert_eq!(options.reveal_horizontal_right_padding, px(15.));
}

/// Monaco `SashLayout`: `_sashRatio ?? splitViewDefaultRatio`, so a new
/// default moves the sash only until it's dragged, and a reset follows it
/// again.
#[gpui::test]
fn a_dragged_sash_survives_a_new_default_ratio(cx: &mut TestAppContext) {
    let (diff, cx) = setup("a", "b", cx);
    cx.simulate_resize(gpui::size(px(1030.), px(600.)));
    paint(cx);
    // Not dragged: follows the default.
    update_options(&diff, cx, |o| o.split_view_default_ratio = 0.4);
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(400.));
    diff.update(cx, |d, cx| d.drag_sash_for_tests(px(400.), px(300.), cx));
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(300.));
    update_options(&diff, cx, |o| o.split_view_default_ratio = 0.6);
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(300.));
    diff.update(cx, |d, cx| d.reset_sash(cx));
    assert_eq!(diff.read_with(cx, |d, _| d.sash_left_for_tests()), px(600.));
}

/// Monaco's legacy diff leaves `innerChanges` undefined for changes of 20+
/// lines: no `charChanges` and no margin revert arrow.
#[gpui::test]
fn legacy_changes_without_inner_changes(cx: &mut TestAppContext) {
    let original: String = (0..25).map(|i| format!("old {i}\n")).collect();
    let modified: String = (0..25).map(|i| format!("new {i}\n")).collect();
    let (diff, cx) = setup(&original, &modified, cx);
    with_glyph_margin(&diff, cx);
    update_options(&diff, cx, |o| {
        o.diff_algorithm = gpui_monaco::DiffAlgorithm::Legacy;
        o.render_gutter_menu = false;
    });
    let changes = diff.read_with(cx, |d, _| d.line_changes().unwrap());
    assert_eq!(changes.len(), 1);
    assert!(changes[0].char_changes.is_none());
    assert!(
        diff.read_with(cx, |d, cx| d.revert_arrow_bounds_for_tests(cx))
            .is_empty()
    );
    // A small change has them.
    let (original, modified) = (new_model("a\nb\nc", cx), new_model("a\nB\nc", cx));
    diff.update(cx, |d, cx| d.set_models(original, modified, cx));
    paint(cx);
    let changes = diff.read_with(cx, |d, _| d.line_changes().unwrap());
    assert_eq!(changes[0].char_changes.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        diff.read_with(cx, |d, cx| d.revert_arrow_bounds_for_tests(cx))
            .len(),
        1
    );
}

/// Monaco `WorkerBasedDocumentDiffProvider`: an empty original skips the
/// algorithms, with one change and one inner change covering everything
/// (so the legacy algorithm marks the inserted text too).
#[gpui::test]
fn an_empty_original_takes_the_fast_path(cx: &mut TestAppContext) {
    let (diff, cx) = setup("", "one\ntwo\nthree", cx);
    update_options(&diff, cx, |o| {
        o.diff_algorithm = gpui_monaco::DiffAlgorithm::Legacy
    });
    let lines_diff = diff.read_with(cx, |d, _| d.diff().cloned().unwrap());
    assert_eq!(lines_diff.changes.len(), 1);
    let change = &lines_diff.changes[0];
    assert_eq!(
        (change.original.to_string(), change.modified.to_string()),
        ("[1,2)".to_string(), "[1,4)".to_string())
    );
    assert_eq!(change.inner_changes.len(), 1);
    assert_eq!(
        (
            change.inner_changes[0].original_range.to_string(),
            change.inner_changes[0].modified_range.to_string()
        ),
        ("[1,1 -> 1,1]".to_string(), "[1,1 -> 3,6]".to_string())
    );
    let changes = diff.read_with(cx, |d, _| d.line_changes().unwrap());
    assert_eq!(changes[0].char_changes.as_ref().map(Vec::len), Some(1));
    assert_eq!(diff.read_with(cx, |d, _| d.identical()), Some(false));
    // Both empty: identical.
    let (original, modified) = (new_model("", cx), new_model("", cx));
    diff.update(cx, |d, cx| d.set_models(original, modified, cx));
    paint(cx);
    assert!(diff.read_with(cx, |d, _| d.diff().unwrap().changes.is_empty()));
    assert_eq!(diff.read_with(cx, |d, _| d.identical()), Some(true));
}

/// Monaco registers `editor.action.diffReview.next`/`.prev` as aliases of
/// the accessible diff viewer's commands.
#[gpui::test]
fn the_old_diff_review_ids_are_aliases(cx: &mut TestAppContext) {
    let (original, modified) = two_change_texts();
    let (diff, cx) = setup(&original, &modified, cx);
    for id in [
        "editor.action.diffReview.next",
        "editor.action.diffReview.prev",
    ] {
        cx.update(|window, cx| diff.update(cx, |d, cx| d.trigger(id, window, cx)))
            .unwrap();
        paint(cx);
        assert!(
            diff.read_with(cx, |d, cx| d.accessible_viewer_state_for_tests(cx))
                .is_some(),
            "{id}"
        );
    }
}

/// Monaco `revealFirstDiff` returns at once without a model, and waits only
/// on the view model it captured.
#[gpui::test]
fn a_pending_reveal_does_not_outlive_the_models(cx: &mut TestAppContext) {
    let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 60\n", "changed\n");
    let (diff, cx) = setup("a", "a", cx);
    let (_, modified_editor) = editors(&diff, cx);
    let head =
        |cx: &mut VisualTestContext| modified_editor.read_with(cx, |e, _| e.selections()[0].head);
    // Without a model, nothing waits.
    diff.update(cx, |d, cx| {
        d.set_model(None, cx);
        d.reveal_first_diff(cx);
    });
    let (a, b) = (new_model(&original, cx), new_model(&modified, cx));
    diff.update(cx, |d, cx| d.set_models(a, b, cx));
    paint(cx);
    assert_eq!(head(cx), PointUtf16::new(0, 0));
    // Waiting for a diff of models that go.
    let (a, b) = (new_model(&original, cx), new_model(&modified, cx));
    let (c, d) = (new_model(&original, cx), new_model(&modified, cx));
    diff.update(cx, |diff, cx| {
        diff.set_models(a, b, cx);
        diff.reveal_first_diff(cx);
        diff.set_models(c, d, cx);
    });
    paint(cx);
    assert_eq!(head(cx), PointUtf16::new(0, 0));
    // Models over `max_file_size`: no diff to reveal.
    let big = "x".repeat(1024 * 1024 + 1);
    update_options(&diff, cx, |o| o.max_file_size = 1);
    let (a, b) = (new_model(&big, cx), new_model(&modified, cx));
    diff.update(cx, |d, cx| {
        d.set_models(a, b, cx);
        d.reveal_first_diff(cx);
    });
    paint(cx);
    update_options(&diff, cx, |o| o.max_file_size = 0);
    assert_eq!(head(cx), PointUtf16::new(0, 0));
    // A reveal with the models it waits for still happens.
    let (a, b) = (new_model(&original, cx), new_model(&modified, cx));
    diff.update(cx, |d, cx| {
        d.set_models(a, b, cx);
        d.reveal_first_diff(cx);
    });
    paint(cx);
    assert_eq!(head(cx), PointUtf16::new(60, 0));
}
