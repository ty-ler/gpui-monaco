//! The multi-file diff editor (Monaco `MultiDiffEditorWidget`).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext, Entity, Focusable as _, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent,
    TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    MultiDiffDocument, MultiDiffEditor, MultiDiffEditorVariant, MultiDiffFileAction, TextModel,
};

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
                binary: false,
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
    // At the end. The files there were measured meanwhile, shorter than
    // guessed: as Monaco's anchoring, the view stayed put, with room below
    // the last file (Monaco `trailingScrollSlack`).
    let (total, slack) = multi.read_with(cx, |m, _| {
        let (top, height) = m.item_bounds_for_tests(m.len() - 1);
        (top + height, m.scroll_state_for_tests().2)
    });
    assert!(slack > px(0.));
    assert_eq!(multi.read_with(cx, |m, _| m.scroll_height()), total + slack);
    assert_eq!(scroll_top(&multi, cx), total + slack - px(800.));
    // Scrolling back up uses the room up.
    wheel(cx, 100.);
    let (_, _, slack_after) = multi.read_with(cx, |m, _| m.scroll_state_for_tests());
    assert!(slack_after < slack, "{slack_after:?} {slack:?}");
}

/// Monaco captures the pointer on the slider: the arrow over the whole
/// window, over the editors' text too, until the release (or a move
/// without the button).
#[gpui::test]
fn a_scrollbar_drag_keeps_the_arrow_everywhere(cx: &mut TestAppContext) {
    use gpui::CursorStyle;
    let (multi, cx) = setup_many(40, cx);
    let captured =
        |cx: &mut VisualTestContext| multi.read_with(cx, |m, _| m.captured_cursor_for_tests());
    let (bar, slider) = multi
        .read_with(cx, |m, _| m.scrollbar_for_tests(true))
        .unwrap();
    assert_eq!(captured(cx), None);
    let grab = slider.center();
    cx.simulate_mouse_move(grab, None, Modifiers::default());
    cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::Arrow));
    // Over the first file's text, then outside the window.
    for to in [point(px(300.), px(60.)), point(px(1500.), grab.y + px(50.))] {
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
        paint(cx);
        assert_eq!(captured(cx), Some(CursorStyle::Arrow));
    }
    cx.simulate_mouse_up(grab, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), None);

    // A press on the track drags too.
    cx.simulate_mouse_move(bar.center(), None, Modifiers::default());
    cx.simulate_mouse_down(bar.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(px(300.), px(60.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::Arrow));
    cx.simulate_mouse_move(point(px(300.), px(60.)), None, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), None);
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

/// `count` documents of 40 lines with one change each, as `setup_many`.
fn many_documents(count: usize, cx: &mut gpui::App) -> Vec<MultiDiffDocument> {
    (0..count)
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
        .collect()
}

fn set_variant(
    multi: &Entity<MultiDiffEditor>,
    variant: MultiDiffEditorVariant,
    cx: &mut VisualTestContext,
) {
    multi.update(cx, |m, cx| m.set_variant(variant, cx));
    paint(cx);
}

fn content_height(
    multi: &Entity<MultiDiffEditor>,
    index: usize,
    cx: &mut VisualTestContext,
) -> gpui::Pixels {
    multi.read_with(cx, |m, cx| {
        m.diff_editor(index)
            .unwrap()
            .read(cx)
            .content_height(cx)
            .unwrap()
    })
}

/// Monaco's `noCards` and `cards` variants
/// (`getMultiDiffEditorVariantConfiguration`).
#[gpui::test]
fn compact_and_card_variants(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(3, cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.variant()),
        MultiDiffEditorVariant::NoCardsNonCompact
    );
    // The standard variant: 40px headers, 9px insets, and the header isn't
    // a button.
    let height = content_height(&multi, 0, cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(0).1),
        px(40.) + height
    );
    let editor = cx.debug_bounds("multi-diff-editor-0").unwrap();
    assert_eq!(editor.left(), px(9.));
    assert!(cx.debug_bounds("multi-diff-header-0").is_none());

    // `noCards`: 32px headers, 8px below each diff, the whole width, no
    // original line numbers.
    set_variant(&multi, MultiDiffEditorVariant::NoCards, cx);
    let height = content_height(&multi, 0, cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(0).1),
        px(32.) + height + px(8.)
    );
    let editor = cx.debug_bounds("multi-diff-editor-0").unwrap();
    assert_eq!((editor.left(), editor.top()), (px(0.), px(32.)));
    assert!(multi.read_with(cx, |m, cx| {
        m.diff_editor(0)
            .unwrap()
            .read(cx)
            .options()
            .hide_original_line_numbers
    }));
    // A header click collapses (`headerClickToCollapse`) and activates.
    let header = cx.debug_bounds("multi-diff-header-1").unwrap();
    assert_eq!(header.size.height, px(32.));
    cx.simulate_click(header.center(), Modifiers::default());
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(1)));
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(1));
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(1).1),
        px(32.)
    );
    // The header took focus; Enter toggles it.
    let focused =
        cx.update(|window, cx| multi.read(cx).focus_handle(cx).contains_focused(window, cx));
    assert!(focused, "the clicked header has focus");
    cx.simulate_keystrokes("enter");
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(1)));

    // `cards`: 40px headers, the editor 9px in (8px padding and a border),
    // under a 36px header.
    set_variant(&multi, MultiDiffEditorVariant::Cards, cx);
    let height = content_height(&multi, 0, cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(0).1),
        px(40.) + height
    );
    let editor = cx.debug_bounds("multi-diff-editor-0").unwrap();
    assert_eq!((editor.left(), editor.top()), (px(9.), px(36.)));
    let header = cx.debug_bounds("multi-diff-header-0").unwrap();
    cx.simulate_click(header.center(), Modifiers::default());
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(0)));
    // Back to the standard variant: original line numbers again.
    set_variant(&multi, MultiDiffEditorVariant::NoCardsNonCompact, cx);
    assert!(!multi.read_with(cx, |m, cx| {
        m.diff_editor(1)
            .unwrap()
            .read(cx)
            .options()
            .hide_original_line_numbers
    }));
}

/// Monaco's binary file placeholder (`isBinary`,
/// `binaryFilePlaceholderContentHeight`).
#[gpui::test]
fn binary_file_placeholders(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let mut documents = many_documents(1, cx);
        let empty = |cx: &mut gpui::Context<MultiDiffEditor>| cx.new(|_| TextModel::new(""));
        documents.push(MultiDiffDocument {
            label: "img/logo.png".into(),
            original_label: None,
            original: Some(empty(cx)),
            modified: Some(empty(cx)),
            binary: true,
        });
        documents.push(MultiDiffDocument {
            label: "img/new.png".into(),
            original_label: None,
            original: None,
            modified: Some(empty(cx)),
            binary: true,
        });
        documents.extend(many_documents(1, cx));
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    // The header and 100px for the placeholder, no editor.
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(1).1),
        px(140.)
    );
    assert!(multi.read_with(cx, |m, _| m.diff_editor(1).is_none()));
    assert_eq!(
        multi.read_with(cx, |m, _| m.bound_items_for_tests()),
        vec![0, 3]
    );
    let placeholder = cx.debug_bounds("multi-diff-binary-1").unwrap();
    let (top, _) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(1));
    assert_eq!(placeholder.top(), top + px(40.));
    assert_eq!(placeholder.size.height, px(100.));
    // No "Open Diff" without a handler (standalone Monaco has none).
    assert!(cx.debug_bounds("multi-diff-open-binary-diff-1").is_none());

    let opened = Rc::new(RefCell::new(Vec::new()));
    let log = opened.clone();
    multi.update(cx, |m, cx| {
        m.set_open_binary_diff_handler(
            Some(Rc::new(move |index, document, _, _| {
                log.borrow_mut().push((index, document.label.to_string()))
            })),
            cx,
        )
    });
    paint(cx);
    // Only with both sides.
    assert!(cx.debug_bounds("multi-diff-open-binary-diff-2").is_none());
    let button = cx.debug_bounds("multi-diff-open-binary-diff-1").unwrap();
    cx.simulate_click(button.center(), Modifiers::default());
    paint(cx);
    assert_eq!(*opened.borrow(), vec![(1, "img/logo.png".to_string())]);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(1));

    // Navigation focuses the placeholder, then goes past it: a binary file
    // has no changes.
    cx.update(|window, cx| multi.update(cx, |m, cx| m.go_to_file(2, window, cx)));
    paint(cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(2));
    assert!(multi.read_with(cx, |m, _| m.diff_editor(2).is_none()));
    cx.update(|window, cx| multi.update(cx, |m, cx| m.go_to_next_change(window, cx)));
    paint(cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(3));
    assert_eq!(cursor(&multi, 3, cx).row, 20);
    // Collapsed, a binary file is its header.
    multi.update(cx, |m, cx| m.set_collapsed(1, true, cx));
    paint(cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(1).1),
        px(40.)
    );
    assert!(cx.debug_bounds("multi-diff-binary-1").is_none());
}

/// Monaco `SmoothScrollableElement`: a physical wheel (line deltas)
/// scrolls the list smoothly over 100ms, a trackpad (pixel deltas) at once.
#[gpui::test]
fn smooth_wheel_scrolling(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(40, cx);
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(600.), px(400.)),
        delta: ScrollDelta::Lines(point(0., -10.)),
        ..Default::default()
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    // Where it's going, at once; what's shown, on the way.
    let target = scroll_top(&multi, cx);
    assert_eq!(target, px(180.));
    assert!(multi.read_with(cx, |m, _| m.has_pending_scroll_animation()));
    let shown = multi.read_with(cx, |m, _| m.scroll_state_for_tests().0);
    assert!(shown < target, "{shown:?}");
    std::thread::sleep(std::time::Duration::from_millis(120));
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.has_pending_scroll_animation()));
    assert_eq!(
        multi.read_with(cx, |m, _| m.scroll_state_for_tests().0),
        target
    );

    // A trackpad's pixels: at once.
    wheel(cx, -100.);
    assert!(!multi.read_with(cx, |m, _| m.has_pending_scroll_animation()));
    assert_eq!(
        multi.read_with(cx, |m, _| m.scroll_state_for_tests().0),
        px(280.)
    );
}

/// Monaco `CompressedVirtualizedScrollView`'s anchored geometry edits:
/// files changing height keep what's at the top of the viewport there.
#[gpui::test]
fn the_compressed_layout_anchors_the_viewport(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(40, cx);
    let top10 = multi.read_with(cx, |m, _| m.item_bounds_for_tests(10).0);
    multi.update(cx, |m, cx| m.set_scroll_top(top10, cx));
    paint(cx);
    let top10 = multi.read_with(cx, |m, _| m.item_bounds_for_tests(10).0);
    assert_eq!(scroll_top(&multi, cx), top10);
    // A file above collapses: the list scrolls with it, so file 10 stays
    // at the top.
    let (_, height2) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(2));
    multi.update(cx, |m, cx| m.set_collapsed(2, true, cx));
    paint(cx);
    let moved = multi.read_with(cx, |m, _| m.item_bounds_for_tests(10).0);
    assert_eq!(moved, top10 - (height2 - px(40.)));
    assert_eq!(scroll_top(&multi, cx), moved);
    // Collapsing the file scrolled under its header keeps its header at the
    // top (the anchor maps to the file's start).
    multi.update(cx, |m, cx| m.set_scroll_top(moved + px(60.), cx));
    paint(cx);
    multi.update(cx, |m, cx| m.set_collapsed(10, true, cx));
    paint(cx);
    assert_eq!(
        scroll_top(&multi, cx),
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(10).0)
    );
    let (_, leading, _) = multi.read_with(cx, |m, _| m.scroll_state_for_tests());
    assert_eq!(leading, px(0.));
}

/// Monaco `getViewState`/`setViewState`: the scroll position, each file's
/// collapsed state and selections, and the active file.
#[gpui::test]
fn whole_list_view_state(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(20, cx);
    multi.update(cx, |m, cx| {
        m.set_collapsed(3, true, cx);
        m.set_collapsed(5, true, cx);
        let editor = m.diff_editor(0).unwrap().read(cx).modified_editor().clone();
        editor.update(cx, |editor, cx| {
            editor.set_selections(
                &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                    20, 2,
                ))],
                cx,
            )
        });
        m.reveal(1, cx);
    });
    paint(cx);
    let saved = multi.read_with(cx, |m, cx| m.save_view_state(cx));
    assert_eq!(saved.doc_states.len(), 20);
    let key0 = gpui_monaco::MultiDiffDocumentKey {
        original: Some("src/dir/file0.rs".into()),
        modified: Some("src/dir/file0.rs".into()),
    };
    assert_eq!(
        saved.doc_states[&key0]
            .selections
            .as_deref()
            .map(|s| s[0].head),
        Some(PointUtf16::new(20, 2))
    );
    assert_eq!(saved.doc_states.values().filter(|s| s.collapsed).count(), 2);
    assert_eq!(
        saved.active_item.as_ref().and_then(|k| k.modified.clone()),
        Some("src/dir/file1.rs".into())
    );
    assert!(saved.scroll_top > px(0.));

    // New documents with the same paths start fresh, then get the state.
    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            let documents = many_documents(20, cx);
            m.set_documents(documents, window, cx);
        })
    });
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(3)));
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), None);
    multi.update(cx, |m, cx| m.restore_view_state(&saved, cx));
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(3) && m.is_collapsed(5)));
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(4)));
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(1));
    assert_eq!(scroll_top(&multi, cx), saved.scroll_top);
    multi.update(cx, |m, cx| m.set_scroll_top(px(0.), cx));
    paint(cx);
    assert_eq!(cursor(&multi, 0, cx), PointUtf16::new(20, 2));

    // A scroll position the list isn't tall enough for yet waits for it
    // (Monaco `_applyPendingScrollState`).
    let mut all_collapsed = saved.clone();
    for state in all_collapsed.doc_states.values_mut() {
        state.collapsed = true;
    }
    all_collapsed.scroll_top = px(2000.);
    // Every file measured, so expanding them gives their final heights.
    multi.update(cx, |m, cx| m.set_all_collapsed(false, cx));
    for step in 0..20 {
        multi.update(cx, |m, cx| m.set_scroll_top(px(step as f32 * 300.), cx));
        paint(cx);
    }
    multi.update(cx, |m, cx| {
        m.set_scroll_top(px(0.), cx);
        m.set_all_collapsed(true, cx)
    });
    paint(cx);
    multi.update(cx, |m, cx| m.restore_view_state(&all_collapsed, cx));
    paint(cx);
    assert_eq!(scroll_top(&multi, cx), px(0.));
    // Expanded, it lands.
    multi.update(cx, |m, cx| m.set_all_collapsed(false, cx));
    paint(cx);
    assert_eq!(scroll_top(&multi, cx), px(2000.));
}

/// Moving the cursor with the keyboard past what shows of a file scrolls
/// the list along (the editor reveals it within its visible part).
#[gpui::test]
fn keyboard_cursor_moves_keep_the_cursor_in_view(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let original: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let modified: String = (0..200).map(|i| format!("changed {i}\n")).collect();
        let documents = vec![MultiDiffDocument::new(
            "tall.rs",
            cx.new(|_| TextModel::new(&original)),
            cx.new(|_| TextModel::new(&modified)),
        )];
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    cx.update(|window, cx| multi.update(cx, |m, cx| m.go_to_file(0, window, cx)));
    paint(cx);
    assert_eq!(scroll_top(&multi, cx), px(0.));
    let cursor_in_view = |cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            let editor = m
                .diff_editor(0)
                .unwrap()
                .read(cx)
                .modified_editor()
                .read(cx);
            let row = editor.selections()[0].head.row;
            let top = px(40.) + editor.top_for_line_number(row, true).unwrap();
            let bottom = px(40.) + editor.bottom_for_line_number(row, false).unwrap();
            let scroll = m.scroll_top();
            (row, top >= scroll + px(40.) && bottom <= scroll + px(800.))
        })
    };
    for _ in 0..80 {
        cx.simulate_keystrokes("down");
    }
    paint(cx);
    let (row, in_view) = cursor_in_view(cx);
    assert_eq!(row, 80);
    assert!(in_view);
    let scrolled = scroll_top(&multi, cx);
    assert!(scrolled > px(0.));
    // And back up.
    for _ in 0..70 {
        cx.simulate_keystrokes("up");
    }
    paint(cx);
    let (row, in_view) = cursor_in_view(cx);
    assert_eq!(row, 10);
    assert!(in_view);
    assert!(scroll_top(&multi, cx) < scrolled);
}

/// Monaco `highlightRange`: a revealed range is highlighted for 350ms.
#[gpui::test]
fn the_reveal_highlight_lasts_350ms(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(10, cx);
    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            m.reveal_range(
                4,
                PointUtf16::new(20, 0)..PointUtf16::new(20, 7),
                false,
                window,
                cx,
            )
        })
    });
    paint(cx);
    let highlights = |cx: &mut VisualTestContext| {
        multi.update(cx, |m, cx| {
            let editor = m.diff_editor(4).unwrap().read(cx).modified_editor().clone();
            editor.update(cx, |editor, cx| {
                let color = editor.colors().symbol_highlight_background;
                editor
                    .decorations_in_range(PointUtf16::new(20, 0)..PointUtf16::new(20, 7), cx)
                    .into_iter()
                    .filter(|d| d.options.background == Some(color))
                    .map(|d| d.range)
                    .collect::<Vec<_>>()
            })
        })
    };
    assert_eq!(
        highlights(cx),
        vec![PointUtf16::new(20, 0)..PointUtf16::new(20, 7)]
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    paint(cx);
    assert_eq!(highlights(cx).len(), 1);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(60));
    paint(cx);
    assert!(highlights(cx).is_empty());
}
