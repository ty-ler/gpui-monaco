//! The side-by-side diff editor (Monaco `DiffEditorWidget`).

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext, point, px};
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
