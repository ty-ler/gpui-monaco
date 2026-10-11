//! The multi-file diff editor (Monaco `MultiDiffEditorWidget`).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext, Entity, Focusable as _, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent,
    TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{MultiDiffDocument, MultiDiffEditor, MultiDiffFileAction, TextModel};

fn paint(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<MultiDiffEditor>, &mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let model = |text: &str, cx: &mut gpui::Context<MultiDiffEditor>| {
            let text = text.to_string();
            cx.new(|_| TextModel::new(&text))
        };
        let documents = vec![
            MultiDiffDocument::new("a.rs", model("a\nb\nc", cx), model("a\nB\nc", cx)),
            MultiDiffDocument::new(
                "long.rs",
                model(&long, cx),
                model(&long.replace("line 100\n", "changed\n"), cx),
            ),
            MultiDiffDocument {
                label: "new.rs".into(),
                original_label: None,
                original: None,
                modified: Some(model("fresh\nfile", cx)),
            },
        ];
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    (multi, cx)
}

/// `count` files of 40 lines, each with one change at line 21 (row 20).
fn setup_many(
    count: usize,
    cx: &mut TestAppContext,
) -> (Entity<MultiDiffEditor>, &mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let documents = (0..count)
            .map(|i| {
                let original: String = (0..40).map(|l| format!("file {i} line {l}\n")).collect();
                let modified = original.replace(
                    &format!("file {i} line 20\n"),
                    &format!("file {i} changed\n"),
                );
                MultiDiffDocument::new(
                    format!("src/dir/file{i}.rs"),
                    cx.new(|_| TextModel::new(&original)),
                    cx.new(|_| TextModel::new(&modified)),
                )
            })
            .collect();
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    (multi, cx)
}

fn scroll_top(multi: &Entity<MultiDiffEditor>, cx: &mut VisualTestContext) -> gpui::Pixels {
    multi.read_with(cx, |m, _| m.scroll_top())
}

/// The files whose top or bottom is in the 800px viewport.
fn files_in_view(multi: &Entity<MultiDiffEditor>, cx: &mut VisualTestContext) -> Vec<usize> {
    multi.read_with(cx, |m, _| {
        let top = m.scroll_top();
        (0..m.len())
            .filter(|&i| {
                let (start, height) = m.item_bounds_for_tests(i);
                start < top + px(800.) && start + height > top && !m.is_collapsed(i)
            })
            .collect()
    })
}

#[gpui::test]
fn files_stack_at_their_content_height(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    assert_eq!(multi.read_with(cx, |m, _| m.len()), 3);
    let (top0, height0) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(0));
    let (top1, height1) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(1));
    assert_eq!(top0, px(0.));
    assert_eq!(top1, height0);
    // Measured, not the 500px guess: three lines, and the long file's
    // unchanged regions hidden.
    assert!(height0 < px(200.), "{height0:?}");
    assert!(height1 < px(500.), "{height1:?}");
    // Unchanged regions are hidden in each file.
    let hidden = multi.read_with(cx, |m, cx| {
        m.diff_editor(1)
            .unwrap()
            .read(cx)
            .hidden_modified_ranges_for_tests()
    });
    assert!(!hidden.is_empty());
}

#[gpui::test]
fn item_editors_have_no_overview_ruler(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    // Monaco's ruler is part of the editors' hidden scrollbars.
    multi.read_with(cx, |m, cx| {
        let diff = m.diff_editor(0).unwrap().read(cx);
        for editor in [diff.original_editor(), diff.modified_editor()] {
            let options = editor.read(cx).options();
            assert_eq!(options.overview_ruler_lanes, 0);
            assert!(!options.overview_ruler_border);
        }
    });
}

#[gpui::test]
fn collapsing_files(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    multi.update(cx, |m, cx| m.set_collapsed(0, true, cx));
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(0)));
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(0).1),
        px(40.)
    );
    // A collapsed file has no editor.
    assert!(multi.read_with(cx, |m, _| m.diff_editor(0).is_none()));
    multi.update(cx, |m, cx| m.set_all_collapsed(true, cx));
    paint(cx);
    // Everything fits: scrolling clamps to the top, and there's no
    // scrollbar.
    multi.update(cx, |m, cx| m.set_scroll_top(px(500.), cx));
    paint(cx);
    assert_eq!(scroll_top(&multi, cx), px(0.));
    assert!(multi.read_with(cx, |m, _| m.scrollbar_for_tests(true).is_none()));
}

#[gpui::test]
fn header_chevron_collapses_and_activates(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    let chevron = cx
        .debug_bounds("multi-diff-collapse-1")
        .expect("the second file's chevron");
    cx.simulate_click(chevron.center(), Modifiers::default());
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(1)));
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(1));
    let chevron = cx.debug_bounds("multi-diff-collapse-1").unwrap();
    cx.simulate_click(chevron.center(), Modifiers::default());
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(1)));
}

#[gpui::test]
fn header_file_actions(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    let runs = Rc::new(RefCell::new(Vec::new()));
    let log = runs.clone();
    multi.update(cx, |m, cx| {
        m.add_file_action(
            MultiDiffFileAction {
                id: "open".into(),
                label: "Open File".into(),
                icon: "↗".into(),
                run: Rc::new(move |index, document, _, _| {
                    log.borrow_mut().push((index, document.label.to_string()))
                }),
            },
            cx,
        )
    });
    paint(cx);
    let chevron = cx.debug_bounds("multi-diff-collapse-0").unwrap();
    // The toolbar is at the header's right end, left of the scrollbar.
    cx.simulate_click(
        point(px(1200.) - px(25.), chevron.center().y),
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(*runs.borrow(), vec![(0, "a.rs".to_string())]);
    assert!(multi.update(cx, |m, cx| m.remove_file_action("open", cx)));
}

#[gpui::test]
fn revealing_a_file(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    cx.simulate_resize(gpui::size(px(1200.), px(250.)));
    paint(cx);
    multi.update(cx, |m, cx| m.reveal(2, cx));
    paint(cx);
    let (top2, _) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(2));
    let scroll = scroll_top(&multi, cx);
    // At the top, unless that would scroll past the end.
    assert!(scroll <= top2 && scroll > px(0.), "{scroll:?} {top2:?}");
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(2));
}

#[gpui::test]
fn only_files_in_view_have_editors(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(100, cx);
    let bound = multi.read_with(cx, |m, _| m.bound_items_for_tests());
    assert_eq!(bound, files_in_view(&multi, cx));
    assert!(bound.len() < 10, "{bound:?}");
    // Every file is measured once shown: far less than the 500px guess.
    let (_, height) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(0));
    assert!(height < px(400.), "{height:?}");

    // Scrolled to the middle, the editors move to the files there, and the
    // pool keeps at most five unused ones.
    let middle = multi.read_with(cx, |m, _| m.item_bounds_for_tests(50).0);
    multi.update(cx, |m, cx| m.set_scroll_top(middle, cx));
    paint(cx);
    let bound = multi.read_with(cx, |m, _| m.bound_items_for_tests());
    assert_eq!(bound, files_in_view(&multi, cx));
    assert!(bound.contains(&50), "{bound:?}");
    assert!(multi.read_with(cx, |m, _| m.diff_editor(0).is_none()));
    let alive = multi.read_with(cx, |m, _| m.diff_editor_count_for_tests());
    assert!(alive <= bound.len() + 5, "{alive} editors for {bound:?}");

    // Scrolling through everything never creates more.
    for step in 0..30 {
        multi.update(cx, |m, cx| m.set_scroll_top(px(step as f32 * 700.), cx));
        paint(cx);
        let bound = multi.read_with(cx, |m, _| m.bound_items_for_tests());
        let alive = multi.read_with(cx, |m, _| m.diff_editor_count_for_tests());
        assert!(alive <= bound.len() + 5, "{alive} editors for {bound:?}");
    }
}

#[gpui::test]
fn view_state_is_kept_across_scrolling(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(40, cx);
    let selection = |multi: &Entity<MultiDiffEditor>, cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            m.diff_editor(0)
                .unwrap()
                .read(cx)
                .modified_editor()
                .read(cx)
                .selections()[0]
                .head
        })
    };
    multi.update(cx, |m, cx| {
        let editor = m.diff_editor(0).unwrap().read(cx).modified_editor().clone();
        editor.update(cx, |editor, cx| {
            editor.set_selections(
                &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                    20, 3,
                ))],
                cx,
            )
        });
    });
    paint(cx);
    let (_, height) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(0));
    let far = multi.read_with(cx, |m, _| m.item_bounds_for_tests(30).0);
    multi.update(cx, |m, cx| m.set_scroll_top(far, cx));
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.diff_editor(0).is_none()));
    multi.update(cx, |m, cx| m.set_scroll_top(px(0.), cx));
    paint(cx);
    assert_eq!(selection(&multi, cx), PointUtf16::new(20, 3));
    // The measured height is kept too, so the list doesn't jump.
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(0).1),
        height
    );
}

fn wheel(cx: &mut VisualTestContext, dy: f32) {
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(600.), px(400.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        ..Default::default()
    });
    paint(cx);
}

#[gpui::test]
fn scrollbar_wheel_and_drag(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(40, cx);
    let (bar, slider) = multi
        .read_with(cx, |m, _| m.scrollbar_for_tests(true))
        .expect("a vertical scrollbar");
    // Monaco's 10px bar at the right, leaving the corner to the other bar.
    assert_eq!(bar.size.width, px(10.));
    assert_eq!(bar.right(), px(1200.));
    assert_eq!(bar.size.height, px(790.));
    assert_eq!(slider.top(), px(0.));
    // Nothing is wider than the list: no horizontal scrollbar.
    assert!(multi.read_with(cx, |m, _| m.scrollbar_for_tests(false).is_none()));

    // The wheel scrolls the list and moves the slider.
    wheel(cx, -300.);
    assert_eq!(scroll_top(&multi, cx), px(300.));
    let (_, moved) = multi
        .read_with(cx, |m, _| m.scrollbar_for_tests(true))
        .unwrap();
    assert!(moved.top() > slider.top());

    // Dragging the slider, even outside the list, scrolls proportionally.
    let total = multi.read_with(cx, |m, _| {
        let (top, height) = m.item_bounds_for_tests(m.len() - 1);
        top + height
    });
    let grab = moved.center();
    cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(scroll_top(&multi, cx), px(300.), "grabbing doesn't jump");
    cx.simulate_mouse_move(
        point(px(1500.), grab.y + px(100.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    paint(cx);
    let ratio = (bar.size.height - moved.size.height) / (total - px(800.));
    let expected = px(300.) + px(100.) / ratio;
    let scrolled = scroll_top(&multi, cx);
    assert!(
        (scrolled - expected).abs() < px(2.),
        "{scrolled:?} {expected:?}"
    );
    cx.simulate_mouse_up(grab, MouseButton::Left, Modifiers::default());
    paint(cx);
    cx.simulate_mouse_move(grab, None, Modifiers::default());
    paint(cx);
    assert_eq!(scroll_top(&multi, cx), scrolled, "the drag ended");

    // Pressing the track centers the slider there.
    cx.simulate_mouse_down(
        point(bar.center().x, bar.bottom() - px(1.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(bar.center().x, bar.bottom() - px(1.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    // At the end (the files there were measured meanwhile).
    let total = multi.read_with(cx, |m, _| {
        let (top, height) = m.item_bounds_for_tests(m.len() - 1);
        top + height
    });
    assert_eq!(scroll_top(&multi, cx), total - px(800.));
}

#[gpui::test]
fn horizontal_scrollbar_for_long_lines(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let long = format!("short\n{}\nend", "x".repeat(400));
        let documents = vec![MultiDiffDocument::new(
            "wide.rs",
            cx.new(|_| TextModel::new("short\nend")),
            cx.new(|_| TextModel::new(&long)),
        )];
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    let (bar, slider) = multi
        .read_with(cx, |m, _| m.scrollbar_for_tests(false))
        .expect("a horizontal scrollbar");
    assert_eq!(bar.size.height, px(10.));
    assert_eq!(bar.bottom(), px(800.));
    cx.simulate_mouse_down(slider.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        slider.center() + point(px(200.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(slider.center(), MouseButton::Left, Modifiers::default());
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.scroll_left()) > px(0.));
}

fn cursor(multi: &Entity<MultiDiffEditor>, index: usize, cx: &mut VisualTestContext) -> PointUtf16 {
    multi.read_with(cx, |m, cx| {
        m.diff_editor(index)
            .unwrap()
            .read(cx)
            .modified_editor()
            .read(cx)
            .selections()[0]
            .head
    })
}

#[gpui::test]
fn next_and_previous_change_across_files(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(5, cx);
    let next = |multi: &Entity<MultiDiffEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| multi.update(cx, |m, cx| m.go_to_next_change(window, cx)));
        paint(cx);
    };
    let previous = |multi: &Entity<MultiDiffEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| multi.update(cx, |m, cx| m.go_to_previous_change(window, cx)));
        paint(cx);
    };
    // No active file: the first file's first change.
    next(&multi, cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(0));
    assert_eq!(cursor(&multi, 0, cx).row, 20);
    // Each file has one change: the next file's.
    next(&multi, cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(1));
    assert_eq!(cursor(&multi, 1, cx).row, 20);
    // The change is in view, centered.
    let (top1, height1) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(1));
    let scroll = scroll_top(&multi, cx);
    assert!(
        scroll > top1 - px(800.) && scroll < top1 + height1,
        "{scroll:?} {top1:?}"
    );
    // The modified editor is focused.
    let focused = cx.update(|window, cx| {
        let editor = multi
            .read(cx)
            .diff_editor(1)
            .unwrap()
            .read(cx)
            .modified_editor();
        editor.focus_handle(cx).is_focused(window)
    });
    assert!(focused);
    // Back to the previous file's last change.
    previous(&multi, cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(0));
    assert_eq!(cursor(&multi, 0, cx).row, 20);
    // Wrapping around, and collapsed files are expanded.
    multi.update(cx, |m, cx| m.set_collapsed(4, true, cx));
    previous(&multi, cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(4));
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(4)));
    assert_eq!(cursor(&multi, 4, cx).row, 20);
}

#[gpui::test]
fn go_to_file_and_reveal_range(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(20, cx);
    cx.update(|window, cx| multi.update(cx, |m, cx| m.go_to_file(12, window, cx)));
    paint(cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(12));
    assert_eq!(cursor(&multi, 12, cx).row, 20);
    let (top, height) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(12));
    let scroll = scroll_top(&multi, cx);
    assert!(scroll >= top - px(800.) && scroll <= top + height);

    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            m.reveal_range(
                3,
                PointUtf16::new(20, 0)..PointUtf16::new(20, 0),
                false,
                window,
                cx,
            )
        })
    });
    paint(cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(3));
    let bound = multi.read_with(cx, |m, _| m.bound_items_for_tests());
    assert!(bound.contains(&3), "{bound:?}");
    // The line is in the middle of the list.
    let (top, height) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(3));
    let middle = scroll_top(&multi, cx) + px(400.);
    assert!(
        middle > top + px(40.) && middle < top + height,
        "{middle:?} {top:?}"
    );
}
