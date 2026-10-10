//! The multi-file diff editor (Monaco `MultiDiffEditorWidget`).

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext, px};
use gpui_monaco::{MultiDiffDocument, MultiDiffEditor, TextModel};

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
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
fn collapsing_files(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    multi.update(cx, |m, cx| m.set_collapsed(0, true, cx));
    paint(cx);
    assert!(multi.read_with(cx, |m, _| m.is_collapsed(0)));
    assert_eq!(
        multi.read_with(cx, |m, _| m.item_bounds_for_tests(0).1),
        px(40.)
    );
    multi.update(cx, |m, cx| m.set_all_collapsed(true, cx));
    paint(cx);
    // Everything fits: scrolling clamps to the top.
    multi.update(cx, |m, cx| m.set_scroll_top(px(500.), cx));
    paint(cx);
    assert_eq!(multi.read_with(cx, |m, _| m.scroll_top()), px(0.));
}

#[gpui::test]
fn revealing_a_file(cx: &mut TestAppContext) {
    let (multi, cx) = setup(cx);
    cx.simulate_resize(gpui::size(px(1200.), px(250.)));
    paint(cx);
    multi.update(cx, |m, cx| m.reveal(2, cx));
    paint(cx);
    let (top2, _) = multi.read_with(cx, |m, _| m.item_bounds_for_tests(2));
    let scroll = multi.read_with(cx, |m, _| m.scroll_top());
    // At the top, unless that would scroll past the end.
    assert!(scroll <= top2 && scroll > px(0.), "{scroll:?} {top2:?}");
}
