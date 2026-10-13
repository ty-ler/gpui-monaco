//! The multi-file diff editor (Monaco `MultiDiffEditorWidget`).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext, Entity, Focusable as _, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent,
    TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    MultiDiffDocument, MultiDiffEditor, MultiDiffEditorEvent, MultiDiffEditorVariant,
    MultiDiffFileAction, TextModel,
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
                ..Default::default()
            },
        ];
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    blur(cx);
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
    blur(cx);
    (multi, cx)
}

/// Takes the focus loading gave the first change, so that file's editor
/// isn't kept for having focus (Monaco `shouldKeepAlive`).
fn blur(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.blur(cx));
    paint(cx);
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
                start < top + px(800.) && start + height > top
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
    // A collapsed file in view keeps its (hidden) editor, as in Monaco.
    assert!(multi.read_with(cx, |m, _| m.diff_editor(0).is_some()));
    assert!(cx.debug_bounds("multi-diff-editor-0").is_none());
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
    let grabbed = scroll_top(&multi, cx);
    assert!(
        (grabbed - px(300.)).abs() < px(0.01),
        "grabbing doesn't jump"
    );
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
    // Loading went to the first file's first change.
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
            ..Default::default()
        });
        documents.push(MultiDiffDocument {
            label: "img/new.png".into(),
            original_label: None,
            original: None,
            modified: Some(empty(cx)),
            binary: true,
            ..Default::default()
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
    // The animation runs on the executor's clock (fake in tests).
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(120));
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

    // Files loaded anew start fresh (at the first change), then get the
    // state.
    // (At the top, past Monaco's 100ms scroll direction, so measuring the
    // new files anchors the top.)
    multi.update(cx, |m, cx| m.set_scroll_top(px(0.), cx));
    paint(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            m.set_documents_loading(cx);
            let documents = many_documents(20, cx);
            m.set_documents(documents, window, cx);
        })
    });
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(3)));
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(0));
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

/// The documents shown, for passing back to `set_documents`.
fn documents(
    multi: &Entity<MultiDiffEditor>,
    cx: &mut VisualTestContext,
) -> Vec<MultiDiffDocument> {
    multi.read_with(cx, |m, _| {
        (0..m.len())
            .map(|i| m.document(i).unwrap().clone())
            .collect()
    })
}

/// Monaco's observable documents, mapped with `mapObservableArrayCached`:
/// updating the list keeps each file still there as it was, and the scroll
/// position (audit 3 §E.19).
#[gpui::test]
fn updating_the_files_keeps_their_state(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(20, cx);
    let top5 = multi.read_with(cx, |m, _| m.item_bounds_for_tests(5).0);
    multi.update(cx, |m, cx| {
        m.set_collapsed(2, true, cx);
        m.set_scroll_top(top5, cx);
    });
    paint(cx);
    multi.update(cx, |m, cx| {
        let editor = m.diff_editor(5).unwrap().read(cx).modified_editor().clone();
        editor.update(cx, |editor, cx| {
            editor.set_selections(
                &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                    20, 4,
                ))],
                cx,
            )
        });
        m.reveal(5, cx);
    });
    paint(cx);
    let scroll = scroll_top(&multi, cx);
    let height5 = multi.read_with(cx, |m, _| m.item_bounds_for_tests(5).1);
    let diff5 = multi.read_with(cx, |m, _| m.diff_editor(5).cloned().unwrap());

    // A new file at the end, and file 19 gone.
    let mut new_documents = documents(&multi, cx);
    new_documents.pop();
    new_documents.push(MultiDiffDocument::new(
        "src/added.rs",
        cx.update(|_, cx| cx.new(|_| TextModel::new("a\n"))),
        cx.update(|_, cx| cx.new(|_| TextModel::new("b\n"))),
    ));
    cx.update(|window, cx| multi.update(cx, |m, cx| m.set_documents(new_documents, window, cx)));
    paint(cx);
    multi.read_with(cx, |m, _| {
        assert_eq!(m.len(), 20);
        assert!(m.is_collapsed(2));
        assert!(!m.is_collapsed(19));
        assert_eq!(m.document(19).unwrap().label.as_ref(), "src/added.rs");
        // The active file stays, so loading doesn't go to the first change.
        assert_eq!(m.active_item(), Some(5));
        assert_eq!(m.item_bounds_for_tests(5).1, height5);
        assert_eq!(m.scroll_top(), scroll);
        // The file keeps its editor.
        assert_eq!(m.diff_editor(5), Some(&diff5));
    });
    assert_eq!(cursor(&multi, 5, cx), PointUtf16::new(20, 4));

    // A file inserted above: the others follow it with their state, and
    // the list keeps its scroll position.
    let mut new_documents = documents(&multi, cx);
    new_documents.insert(
        0,
        MultiDiffDocument::new(
            "src/first.rs",
            cx.update(|_, cx| cx.new(|_| TextModel::new("x\n"))),
            cx.update(|_, cx| cx.new(|_| TextModel::new("y\n"))),
        ),
    );
    cx.update(|window, cx| multi.update(cx, |m, cx| m.set_documents(new_documents, window, cx)));
    paint(cx);
    multi.read_with(cx, |m, _| {
        assert!(m.is_collapsed(3) && !m.is_collapsed(2));
        assert_eq!(m.active_item(), Some(6));
        assert_eq!(m.item_bounds_for_tests(6).1, height5);
        assert_eq!(m.scroll_top(), scroll);
    });
    assert_eq!(cursor(&multi, 6, cx), PointUtf16::new(20, 4));

    // The same files with other models keep their state too (Monaco's
    // workbench keys its documents by their uris).
    let fresh = cx.update(|_, cx| many_documents(20, cx));
    cx.update(|window, cx| multi.update(cx, |m, cx| m.set_documents(fresh, window, cx)));
    paint(cx);
    multi.read_with(cx, |m, _| {
        assert_eq!(m.len(), 20);
        assert!(m.is_collapsed(2));
        assert_eq!(m.active_item(), Some(5));
    });
}

/// Monaco's "Initialize first change": once files load and none is
/// active, the first change is selected and focused (audit 3 §E.20).
#[gpui::test]
fn loading_files_goes_to_the_first_change(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let original: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 250\n", "changed\n");
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let documents = vec![MultiDiffDocument::new(
            "far.rs",
            cx.new(|_| TextModel::new(&original)),
            cx.new(|_| TextModel::new(&modified)),
        )];
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(0));
    assert_eq!(cursor(&multi, 0, cx), PointUtf16::new(250, 0));
    let focused = cx.update(|window, cx| {
        let diff = multi.read(cx).diff_editor(0).unwrap().read(cx);
        diff.modified_editor().focus_handle(cx).is_focused(window)
    });
    assert!(focused);
}

/// Monaco `setPreserveFocusOnLoad`: the first change is selected, but not
/// focused.
#[gpui::test]
fn loading_files_can_leave_the_focus(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let original: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let modified = original.replace("line 250\n", "changed\n");
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let mut multi = MultiDiffEditor::new(Vec::new(), window, cx);
        multi.set_preserve_focus_on_load(true);
        let documents = vec![MultiDiffDocument::new(
            "far.rs",
            cx.new(|_| TextModel::new(&original)),
            cx.new(|_| TextModel::new(&modified)),
        )];
        multi.set_documents(documents, window, cx);
        multi
    });
    paint(cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(0));
    assert_eq!(cursor(&multi, 0, cx), PointUtf16::new(250, 0));
    let focused =
        cx.update(|window, cx| multi.read(cx).focus_handle(cx).contains_focused(window, cx));
    assert!(!focused);
}

/// Monaco's `.placeholder`: "No Changed Files", or "Loading..." while the
/// documents load (audit 3 §E.21).
#[gpui::test]
fn an_empty_list_shows_a_placeholder(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| MultiDiffEditor::new(Vec::new(), window, cx));
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.placeholder_message()),
        Some("No Changed Files")
    );
    let placeholder = cx.debug_bounds("multi-diff-placeholder").unwrap();
    assert_eq!(placeholder.size, gpui::size(px(1200.), px(800.)));

    multi.update(cx, |m, cx| m.set_documents_loading(cx));
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_loading()));
    assert_eq!(
        multi.read_with(cx, |m, _| m.placeholder_message()),
        Some("Loading...")
    );

    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            let documents = many_documents(2, cx);
            m.set_documents(documents, window, cx)
        })
    });
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.is_loading()));
    assert_eq!(multi.read_with(cx, |m, _| m.placeholder_message()), None);
    assert!(cx.debug_bounds("multi-diff-placeholder").is_none());
}

/// Monaco `IDocumentDiffItem.options`, `onOptionsDidChange` and
/// `contextKeys` (audit 3 §E.22).
#[gpui::test]
fn per_file_options_and_context_keys(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let mut documents = many_documents(2, cx);
        documents[1].options = Some(gpui_monaco::DiffEditorOptions {
            render_side_by_side: false,
            ..Default::default()
        });
        documents[1].context_keys = vec![("reviewFile".into(), true.into())];
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    let side_by_side = |index: usize, cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            m.diff_editor(index)
                .unwrap()
                .read(cx)
                .options()
                .render_side_by_side
        })
    };
    assert!(side_by_side(0, cx));
    assert!(!side_by_side(1, cx));
    // Unchanged regions are hidden in a file with its own options too.
    assert!(multi.read_with(cx, |m, cx| {
        m.diff_editor(1)
            .unwrap()
            .read(cx)
            .options()
            .hide_unchanged_regions
            .enabled
    }));
    let has_key = |index: usize, cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            let diff = m.diff_editor(index).unwrap().read(cx);
            [diff.original_editor(), diff.modified_editor()]
                .iter()
                .all(|editor| editor.read(cx).context_keys(cx).contains("reviewFile"))
        })
    };
    assert!(has_key(1, cx));
    assert!(!has_key(0, cx));

    // `onOptionsDidChange`: the file's editor takes them at once.
    multi.update(cx, |m, cx| m.set_document_options(1, None, None, cx));
    assert!(side_by_side(1, cx));
    multi.update(cx, |m, cx| {
        m.set_document_options(
            0,
            Some(gpui_monaco::DiffEditorOptions {
                render_side_by_side: false,
                ..Default::default()
            }),
            None,
            cx,
        )
    });
    assert!(!side_by_side(0, cx));

    // The keys go with the file: a list without it leaves its editor
    // without them.
    let diff1 = multi.read_with(cx, |m, _| m.diff_editor(1).cloned().unwrap());
    let first = documents(&multi, cx).remove(0);
    cx.update(|window, cx| multi.update(cx, |m, cx| m.set_documents(vec![first], window, cx)));
    paint(cx);
    let keyed = multi.read_with(cx, |_, cx| {
        let diff = diff1.read(cx);
        diff.modified_editor()
            .read(cx)
            .context_keys(cx)
            .contains("reviewFile")
    });
    assert!(!keyed);
}

/// Monaco's "resolves original line number visibility from variant and
/// explicit options" (`multiDiffEditorWidget.test.ts`; audit 3 §E.23).
#[gpui::test]
fn original_line_numbers_follow_the_variant_and_explicit_options(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(1, cx);
    let hidden = |cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            m.diff_editor(0)
                .unwrap()
                .read(cx)
                .options()
                .hide_original_line_numbers
        })
    };
    let set_file = |hide: Option<bool>, cx: &mut VisualTestContext| {
        multi.update(cx, |m, cx| m.set_document_options(0, None, hide, cx));
        paint(cx);
    };
    set_variant(&multi, MultiDiffEditorVariant::NoCards, cx);
    let default_no_cards_hidden = hidden(cx);
    set_file(Some(false), cx);
    let explicit_false_shows = !hidden(cx);
    set_file(Some(true), cx);
    let explicit_true_hides = hidden(cx);
    set_file(None, cx);
    set_variant(&multi, MultiDiffEditorVariant::Cards, cx);
    let default_cards_hidden = hidden(cx);
    set_variant(&multi, MultiDiffEditorVariant::NoCardsNonCompact, cx);
    let default_non_compact_shows = !hidden(cx);
    assert_eq!(
        (
            default_no_cards_hidden,
            explicit_false_shows,
            explicit_true_hides,
            default_cards_hidden,
            default_non_compact_shows,
        ),
        (true, true, true, true, true)
    );

    // The list's own choice comes first (Monaco's widget options override).
    set_variant(&multi, MultiDiffEditorVariant::NoCards, cx);
    set_file(Some(true), cx);
    multi.update(cx, |m, cx| {
        m.set_hide_original_line_numbers(Some(false), cx)
    });
    paint(cx);
    assert!(!hidden(cx));
}

/// Monaco's "models bottom padding as trailing scroll content"
/// (`multiDiffEditorWidget.test.ts`; audit 3 §E.24).
#[gpui::test]
fn bottom_padding_is_trailing_scroll_content(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| MultiDiffEditor::new(Vec::new(), window, cx));
    cx.simulate_resize(gpui::size(px(800.), px(200.)));
    paint(cx);
    let initial = multi.read_with(cx, |m, _| m.scroll_height());
    multi.update(cx, |m, cx| m.set_padding_bottom(px(24.), cx));
    paint(cx);
    assert_eq!(
        (
            multi.read_with(cx, |m, _| m.scroll_height()) - initial,
            multi.read_with(cx, |m, _| m.len())
        ),
        (px(24.), 0)
    );
}

/// With files, the list scrolls past the last one by the bottom padding.
#[gpui::test]
fn bottom_padding_below_the_last_file(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(10, cx);
    multi.update(cx, |m, cx| m.set_padding_bottom(px(100.), cx));
    // Scrolled to the end once every file is measured.
    for _ in 0..2 {
        multi.update(cx, |m, cx| m.set_scroll_top(px(100_000.), cx));
        paint(cx);
    }
    let (top, height) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(9));
    let (_, leading, trailing) = multi.read_with(cx, |m, _| m.scroll_state_for_tests());
    let scroll_height = multi.read_with(cx, |m, _| m.scroll_height());
    assert_eq!(scroll_height - leading - trailing, top + height + px(100.));
    assert_eq!(scroll_top(&multi, cx), scroll_height - px(800.));
}

/// Monaco `getActiveControl` and `onDidChangeActiveControl` (audit 3
/// §E.24).
#[gpui::test]
fn the_active_control(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let events = Rc::new(RefCell::new(0));
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let documents = many_documents(10, cx);
        MultiDiffEditor::new(documents, window, cx)
    });
    let log = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&multi, move |_, event: &MultiDiffEditorEvent, _| {
            if *event == MultiDiffEditorEvent::ActiveControlChanged {
                *log.borrow_mut() += 1;
            }
        })
        .detach()
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    // Loading made the first file active.
    let active = multi.read_with(cx, |m, _| m.active_control().cloned());
    assert_eq!(
        active,
        multi.read_with(cx, |m, _| m.diff_editor(0).cloned())
    );
    assert!(active.is_some());

    cx.update(|window, cx| multi.update(cx, |m, cx| m.go_to_file(7, window, cx)));
    paint(cx);
    assert_eq!(*events.borrow(), 1);
    let active = multi.read_with(cx, |m, _| m.active_control().cloned());
    assert_eq!(
        active,
        multi.read_with(cx, |m, _| m.diff_editor(7).cloned())
    );
    assert!(active.is_some());
    // No change, no event.
    paint(cx);
    assert_eq!(*events.borrow(), 1);
}

/// Monaco `inMultiDiffEditor`, which the files' editors inherit with the
/// diff editor keys, and `multiDiffEditorAllCollapsed`.
#[gpui::test]
fn context_keys(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    let diff = multi.read_with(cx, |m, _| m.diff_editor(0).unwrap().clone());
    let modified = diff.read_with(cx, |d, _| d.modified_editor().clone());
    let keys = modified.read_with(cx, |e, cx| e.context_keys(cx));
    assert!(keys.contains("inMultiDiffEditor") && keys.contains("isInDiffEditor"));
    assert!(!keys.contains("multiDiffEditorAllCollapsed"));
    // All but this file collapsed.
    multi.update(cx, |m, cx| {
        m.set_collapsed(1, true, cx);
        m.set_collapsed(2, true, cx);
    });
    paint(cx);
    let keys = modified.read_with(cx, |e, cx| e.context_keys(cx));
    assert!(!keys.contains("multiDiffEditorAllCollapsed"));
    assert!(
        diff.read_with(cx, |d, cx| d.context_keys(cx))
            .contains("inMultiDiffEditor")
    );
}

/// Whether file `index`'s modified editor has the focus.
fn modified_focused(
    multi: &Entity<MultiDiffEditor>,
    index: usize,
    cx: &mut VisualTestContext,
) -> bool {
    cx.update(|window, cx| {
        multi.read(cx).diff_editor(index).is_some_and(|diff| {
            diff.read(cx)
                .modified_editor()
                .focus_handle(cx)
                .is_focused(window)
        })
    })
}

/// A saved state of 20 files: 3 and 5 collapsed, a cursor at (20, 2) in
/// file 0 and file `active` active.
fn saved_state(
    active: usize,
    cx: &mut TestAppContext,
) -> (
    Entity<MultiDiffEditor>,
    &mut VisualTestContext,
    gpui_monaco::MultiDiffEditorViewState,
) {
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
        m.reveal(active, cx);
    });
    paint(cx);
    let saved = multi.read_with(cx, |m, cx| m.save_view_state(cx));
    assert!(saved.scroll_top > px(0.));
    // Back at the top, past Monaco's 100ms scroll direction, so measuring
    // the new files anchors the top.
    multi.update(cx, |m, cx| m.set_scroll_top(px(0.), cx));
    paint(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    (multi, cx, saved)
}

/// Monaco `setViewState` before the documents load: the states wait in
/// `_lastDocStates` and the active item in `_lastActiveDiffItemKey`, which
/// replaces the first-change navigation and gets the focus (audit 4
/// §E.21).
#[gpui::test]
fn a_view_state_restored_while_loading_waits_for_the_files(cx: &mut TestAppContext) {
    let (multi, cx, mut saved) = saved_state(1, cx);
    // In file 1 while the new files have Monaco's initial 500px height.
    saved.scroll_top = px(960.);
    multi.update(cx, |m, cx| {
        m.set_documents_loading(cx);
        m.restore_view_state(&saved, cx);
    });
    paint(cx);
    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            let documents = many_documents(20, cx);
            m.set_documents(documents, window, cx);
        })
    });
    paint(cx);
    multi.read_with(cx, |m, _| {
        assert!(m.is_collapsed(3) && m.is_collapsed(5) && !m.is_collapsed(4));
        assert_eq!(m.active_item(), Some(1));
    });
    // The restored position lands, then follows file 1 as the files get
    // measured (Monaco's anchored geometry edits): not the first change's.
    let (top1, _) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(1));
    assert_eq!(scroll_top(&multi, cx), top1);
    assert!(modified_focused(&multi, 1, cx));
    // File 0 has its cursor, not the first change.
    multi.update(cx, |m, cx| m.set_scroll_top(px(0.), cx));
    paint(cx);
    assert_eq!(cursor(&multi, 0, cx), PointUtf16::new(20, 2));

    // Files that show up later take their part too, until cleared.
    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            m.set_documents(many_documents(3, cx), window, cx);
            let documents = many_documents(20, cx);
            m.set_documents(documents, window, cx);
        })
    });
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(5)));
    multi.update(cx, |m, _| m.clear_pending_restoration_state());
    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            m.set_documents(many_documents(3, cx), window, cx);
            let documents = many_documents(20, cx);
            m.set_documents(documents, window, cx);
        })
    });
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(5)));
}

/// Monaco `setViewModel(vm, { viewState })`: documents and view state in
/// one transaction, so the restored active file wins over the first change
/// and gets the focus, unless `preserveFocus` (audit 4 §E.21).
#[gpui::test]
fn a_view_state_restored_with_the_files_focuses_its_active_file(cx: &mut TestAppContext) {
    let (multi, cx, mut saved) = saved_state(3, cx);
    // Collapsed, file 3 would have no editor to focus.
    for state in saved.doc_states.values_mut() {
        state.collapsed = false;
    }
    let load = |multi: &Entity<MultiDiffEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            multi.update(cx, |m, cx| {
                m.set_documents_loading(cx);
                let documents = many_documents(20, cx);
                m.set_documents(documents, window, cx);
                m.restore_view_state(&saved, cx);
            })
        });
        paint(cx);
    };
    load(&multi, cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(3));
    assert!(modified_focused(&multi, 3, cx));
    assert!(!modified_focused(&multi, 0, cx));

    // `preserveFocus`: active, not focused.
    blur(cx);
    multi.update(cx, |m, _| m.set_preserve_focus_on_load(true));
    load(&multi, cx);
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(3));
    let focused =
        cx.update(|window, cx| multi.read(cx).focus_handle(cx).contains_focused(window, cx));
    assert!(!focused);
}

/// Monaco `IDocumentDiffItem.options` with editor options: a read-only
/// file stays read-only when its editor is reused, and doesn't pass it on
/// (audit 4 §E.22).
#[gpui::test]
fn per_file_editor_options_stay_with_the_file(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let mut documents = many_documents(20, cx);
        documents[0].editor_options.read_only = Some(true);
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    blur(cx);
    let read_only = |index: usize, cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            let diff = m.diff_editor(index).unwrap().read(cx);
            diff.modified_editor().read(cx).options().read_only
        })
    };
    assert!(read_only(0, cx));
    assert!(!read_only(1, cx));
    let diff0 = multi.read_with(cx, |m, _| m.diff_editor(0).cloned().unwrap());
    // Scrolled out, file 0's editor goes to another file, not read-only.
    let top = multi.read_with(cx, |m, _| m.item_bounds_for_tests(15).0);
    multi.update(cx, |m, cx| m.set_scroll_top(top, cx));
    paint(cx);
    let reused = multi.read_with(cx, |m, _| {
        m.bound_items_for_tests()
            .into_iter()
            .find(|&i| m.diff_editor(i) == Some(&diff0))
    });
    let reused = reused.expect("file 0's editor is reused");
    assert!(!read_only(reused, cx));
    // Back in view, file 0 is read-only again.
    multi.update(cx, |m, cx| m.set_scroll_top(px(0.), cx));
    paint(cx);
    assert!(read_only(0, cx));

    // `onOptionsDidChange`.
    multi.update(cx, |m, cx| {
        m.set_document_editor_options(
            0,
            gpui_monaco::MultiDiffDocumentEditorOptions::default(),
            cx,
        )
    });
    assert!(!read_only(0, cx));
}

/// Monaco's "applies document and responsive layout options before
/// attaching the diff model" (`multiDiffEditorWidget.test.ts`): a file's
/// `accessibilitySupport`, and `setViewMode('automatic')` showing inline
/// while narrow, with `diffEditorAutomaticRenderSideBySide` following
/// (audit 4 §E.22, §E.23).
#[gpui::test]
fn applies_document_and_responsive_layout_options(cx: &mut TestAppContext) {
    use gpui_monaco::options::AccessibilitySupport;
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let mut documents = many_documents(1, cx);
        documents[0].editor_options.accessibility_support = Some(AccessibilitySupport::Off);
        let mut multi = MultiDiffEditor::new(Vec::new(), window, cx);
        multi.set_view_mode(gpui_monaco::DiffEditorViewMode::Automatic, cx);
        multi.set_documents(documents, window, cx);
        multi
    });
    cx.simulate_resize(gpui::size(px(800.), px(600.)));
    paint(cx);
    let state = |cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            let diff = m.active_control().unwrap().read(cx);
            (
                diff.options().render_side_by_side,
                diff.options().use_inline_view_when_space_is_limited,
                diff.renders_side_by_side(),
                m.context_keys(cx)
                    .contains("diffEditorAutomaticRenderSideBySide"),
                diff.modified_editor()
                    .read(cx)
                    .options()
                    .accessibility_support,
            )
        })
    };
    let narrow = state(cx);
    cx.simulate_resize(gpui::size(px(1000.), px(600.)));
    paint(cx);
    let wide = state(cx);
    assert_eq!(
        (narrow, wide),
        (
            (true, true, false, false, AccessibilitySupport::Off),
            (true, true, true, true, AccessibilitySupport::Off),
        )
    );
}

/// Monaco `setRenderSideBySide`, `toggleRenderSideBySide` and
/// `setDiffWordWrap`: the list's layout goes over every file's options,
/// and `multiDiffEditorRenderSideBySide` follows (audit 4 §E.23).
#[gpui::test]
fn the_list_layout_overrides_every_file(cx: &mut TestAppContext) {
    use gpui_monaco::options::WordWrapOverride;
    cx.update(gpui_monaco::init);
    let (multi, cx) = cx.add_window_view(|window, cx| {
        let mut documents = many_documents(2, cx);
        documents[1].options = Some(gpui_monaco::DiffEditorOptions::default());
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    let options = |cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            (0..2)
                .map(|i| {
                    let options = m.diff_editor(i).unwrap().read(cx).options();
                    (options.render_side_by_side, options.diff_word_wrap)
                })
                .collect::<Vec<_>>()
        })
    };
    let key = |cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            m.context_keys(cx)
                .contains("multiDiffEditorRenderSideBySide")
        })
    };
    assert!(key(cx));
    multi.update(cx, |m, cx| m.set_render_side_by_side(false, false, cx));
    assert_eq!(options(cx), vec![(false, WordWrapOverride::Inherit); 2]);
    assert!(!key(cx));
    multi.update(cx, |m, cx| m.toggle_render_side_by_side(cx));
    assert_eq!(options(cx), vec![(true, WordWrapOverride::Inherit); 2]);
    assert!(key(cx));
    // The word wrap merges in, keeping the layout.
    multi.update(cx, |m, cx| {
        m.set_view_mode(gpui_monaco::DiffEditorViewMode::Inline, cx);
        m.set_diff_word_wrap(WordWrapOverride::On, cx);
    });
    assert_eq!(options(cx), vec![(false, WordWrapOverride::On); 2]);
    // A file's option change keeps the list's layout over it.
    multi.update(cx, |m, cx| {
        m.set_document_options(1, Some(gpui_monaco::DiffEditorOptions::default()), None, cx)
    });
    assert_eq!(options(cx), vec![(false, WordWrapOverride::On); 2]);
}

/// Monaco matches files by identity, so new models get a new view model:
/// a kept file whose models changed doesn't get its revealed regions back,
/// but keeps its collapsed state and selections (audit 4 §E.24).
#[gpui::test]
fn a_kept_file_with_new_models_drops_its_revealed_regions(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(3, cx);
    let hidden = |cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, cx| {
            m.diff_editor(1)
                .unwrap()
                .read(cx)
                .hidden_modified_ranges_for_tests()
        })
    };
    assert!(!hidden(cx).is_empty());
    multi.update(cx, |m, cx| {
        let diff = m.diff_editor(1).unwrap().clone();
        diff.update(cx, |diff, cx| diff.show_all_unchanged_regions(cx));
        let editor = diff.read(cx).modified_editor().clone();
        editor.update(cx, |editor, cx| {
            editor.set_selections(
                &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                    20, 3,
                ))],
                cx,
            )
        });
        m.set_collapsed(2, true, cx);
    });
    paint(cx);
    assert!(hidden(cx).is_empty());
    // The same files with other models.
    cx.update(|window, cx| {
        multi.update(cx, |m, cx| {
            let documents = many_documents(3, cx);
            m.set_documents(documents, window, cx)
        })
    });
    paint(cx);
    assert!(!hidden(cx).is_empty());
    assert_eq!(cursor(&multi, 1, cx), PointUtf16::new(20, 3));
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(2)));
}

/// Monaco binds every file in view, collapsed or not, so collapsing the
/// active file keeps `getActiveControl` and fires no
/// `onDidChangeActiveControl` (audit 4 §E.25).
#[gpui::test]
fn collapsed_files_in_view_keep_their_editor(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    let events = Rc::new(RefCell::new(0));
    let log = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&multi, move |_, event: &MultiDiffEditorEvent, _| {
            if *event == MultiDiffEditorEvent::ActiveControlChanged {
                *log.borrow_mut() += 1;
            }
        })
        .detach()
    });
    let active = multi.read_with(cx, |m, _| m.active_control().cloned());
    assert!(active.is_some());
    multi.update(cx, |m, cx| m.set_collapsed(0, true, cx));
    paint(cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.active_control().cloned()),
        active
    );
    multi.update(cx, |m, cx| m.set_collapsed(0, false, cx));
    paint(cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.active_control().cloned()),
        active
    );
    assert_eq!(*events.borrow(), 0);
    // Expanded again, it shows at its measured height.
    assert!(cx.debug_bounds("multi-diff-editor-0").is_some());
    let (_, height) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(0));
    assert!(height < px(200.), "{height:?}");

    // Collapsing everything binds every file in view.
    multi.update(cx, |m, cx| m.set_all_collapsed(true, cx));
    paint(cx);
    assert_eq!(
        multi.read_with(cx, |m, _| m.bound_items_for_tests()),
        vec![0, 1, 2]
    );
}

/// Monaco's `expandedContentHeight` starts at 500 for the whole file,
/// header and padding included (the vendored test's
/// `firstObservedLastItemHeight: 500`; audit 4 §E.26).
#[gpui::test]
fn an_unmeasured_file_is_500px_in_all(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    for variant in [
        MultiDiffEditorVariant::NoCardsNonCompact,
        MultiDiffEditorVariant::NoCards,
        MultiDiffEditorVariant::Cards,
    ] {
        // Before the first frame, so before any file is measured.
        cx.add_window_view(|window, cx| {
            let documents = many_documents(20, cx);
            let mut m = MultiDiffEditor::new(documents, window, cx);
            m.set_variant(variant, cx);
            for index in [0, 10, 19] {
                assert_eq!(
                    m.item_bounds_for_tests(index),
                    (px(500.) * index as f32, px(500.)),
                    "{variant:?}"
                );
            }
            m.reveal(10, cx);
            assert_eq!(m.scroll_top(), px(5000.), "{variant:?}");
            m
        });
    }
}

/// Monaco's chevron is a focusable `Button` in every variant, clicked by
/// Enter or Space (audit 4 §E.27).
#[gpui::test]
fn the_chevron_takes_the_keyboard_focus(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    // The first tab stop is the first file's chevron: Enter collapses it.
    cx.update(|window, cx| window.focus_next(cx));
    paint(cx);
    cx.simulate_keystrokes("enter");
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(0)));
    cx.simulate_keystrokes("space");
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(0)));
    cx.simulate_keystrokes("enter");
    paint(cx);
    // Its editor is hidden: Tab goes on to the next file's chevron, whose
    // Space collapses and activates its file.
    cx.simulate_keystrokes("tab");
    paint(cx);
    cx.simulate_keystrokes("space");
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(1)));
    assert_eq!(multi.read_with(cx, |m, _| m.active_item()), Some(1));
    // Expanded, Tab goes from the chevron into its file's editor.
    cx.simulate_keystrokes("space");
    paint(cx);
    cx.simulate_keystrokes("tab");
    paint(cx);
    let in_editor = cx.update(|window, cx| {
        let diff = multi.read(cx).diff_editor(1).unwrap().read(cx);
        [diff.original_editor(), diff.modified_editor()]
            .iter()
            .any(|editor| editor.focus_handle(cx).is_focused(window))
    });
    assert!(in_editor);
}

/// The compact variants' headers take the focus too: Tab reaches the
/// header, then its chevron (Monaco's `tabIndex = 0` and `Button`).
#[gpui::test]
fn compact_headers_and_chevrons_are_tab_stops(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    set_variant(&multi, MultiDiffEditorVariant::NoCards, cx);
    blur(cx);
    // The header: Enter toggles the file.
    cx.update(|window, cx| window.focus_next(cx));
    paint(cx);
    cx.simulate_keystrokes("enter");
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(0)));
    // The chevron, once: its Space doesn't reach the header too.
    cx.simulate_keystrokes("tab");
    paint(cx);
    cx.simulate_keystrokes("space");
    paint(cx);
    assert!(!multi.read_with(cx, |m, _| m.is_collapsed(0)));
}

/// Monaco `focus()`: the active file's editor, `false` without one
/// (audit 4 §E.28).
#[gpui::test]
fn focusing_the_active_file(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(20, cx);
    assert!(!modified_focused(&multi, 0, cx));
    assert!(cx.update(|window, cx| multi.update(cx, |m, cx| m.focus_active(window, cx))));
    paint(cx);
    assert!(modified_focused(&multi, 0, cx));
    blur(cx);
    // Out of view, the file has no editor to focus.
    multi.update(cx, |m, cx| m.set_scroll_top(px(5000.), cx));
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.diff_editor(0).is_none()));
    assert!(!cx.update(|window, cx| multi.update(cx, |m, cx| m.focus_active(window, cx))));
    assert!(cx.update(|window, cx| window.focused(cx).is_none()));

    // No active file.
    cx.update(|window, cx| multi.update(cx, |m, cx| m.set_documents(Vec::new(), window, cx)));
    paint(cx);
    assert!(!cx.update(|window, cx| multi.update(cx, |m, cx| m.focus_active(window, cx))));
}

/// Monaco's `R` flag and struck-through original path show only when the
/// two paths differ (audit 4 §E.28).
#[gpui::test]
fn the_rename_flag_needs_different_paths(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (_multi, cx) = cx.add_window_view(|window, cx| {
        let model = |text: &str, cx: &mut gpui::Context<MultiDiffEditor>| {
            let text = text.to_string();
            cx.new(|_| TextModel::new(&text))
        };
        let documents = vec![
            MultiDiffDocument {
                original_label: Some("same.rs".into()),
                ..MultiDiffDocument::new("same.rs", model("a", cx), model("b", cx))
            },
            MultiDiffDocument {
                original_label: Some("old.rs".into()),
                ..MultiDiffDocument::new("new.rs", model("a", cx), model("b", cx))
            },
        ];
        MultiDiffEditor::new(documents, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    paint(cx);
    assert!(cx.debug_bounds("multi-diff-status-0").is_none());
    assert!(cx.debug_bounds("multi-diff-status-1").is_some());
    assert!(cx.debug_bounds("multi-diff-original-label-0").is_none());
    assert!(cx.debug_bounds("multi-diff-original-label-1").is_some());
}

/// Monaco toggles `shadow` on `.header`, but the standard rule targets
/// `.header-content.shadow` and never matches: only the cards variant's
/// stuck header floats with a shadow.
#[gpui::test]
fn only_the_cards_variant_shadows_a_stuck_header(cx: &mut TestAppContext) {
    let (multi, cx) = setup_many(10, cx);
    let shadowed = |multi: &Entity<MultiDiffEditor>, cx: &mut VisualTestContext| {
        multi.read_with(cx, |m, _| m.shadowed_headers_for_tests())
    };
    multi.update(cx, |m, cx| m.set_scroll_top(px(100.), cx));
    paint(cx);
    assert!(shadowed(&multi, cx).is_empty(), "standard variant");

    set_variant(&multi, MultiDiffEditorVariant::NoCards, cx);
    multi.update(cx, |m, cx| m.set_scroll_top(px(100.), cx));
    paint(cx);
    assert!(shadowed(&multi, cx).is_empty(), "noCards variant");

    set_variant(&multi, MultiDiffEditorVariant::Cards, cx);
    multi.update(cx, |m, cx| m.set_scroll_top(px(100.), cx));
    paint(cx);
    assert_eq!(shadowed(&multi, cx), vec![0], "cards variant");

    multi.update(cx, |m, cx| m.set_scroll_top(px(0.), cx));
    paint(cx);
    assert!(
        shadowed(&multi, cx).is_empty(),
        "not scrolled under its header"
    );
}
