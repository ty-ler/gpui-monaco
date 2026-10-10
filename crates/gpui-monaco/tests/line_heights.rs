//! Variable line heights (Monaco `lineHeight` decorations and
//! `allowVariableLineHeights`).

use gpui::{Focusable, TestAppContext, VisualTestContext, point, px};
use gpui_monaco::CodeEditor;
use gpui_monaco::decorations::DecorationOptions;
use gpui_monaco::text::PointUtf16;

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
fn decorations_make_lines_taller(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) =
        cx.add_window_view(|window, cx| CodeEditor::with_text("one\ntwo\nthree\nfour", window, cx));
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.line_height = Some(px(20.));
        e.update_options(options, cx);
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    let top = |cx: &mut VisualTestContext, row: u32| {
        editor.read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(row, 0))
                .unwrap()
                .y
        })
    };
    let first = top(cx, 0);
    assert_eq!(top(cx, 2) - first, px(40.));

    // Rows 1 and 2 at twice the height ("two\nthree" spans both).
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.delta_decorations(
                None,
                &[],
                vec![(
                    4..9,
                    DecorationOptions {
                        line_height: Some(2.),
                        ..Default::default()
                    },
                )],
                cx,
            );
        })
    });
    paint(cx);
    assert_eq!(top(cx, 1) - first, px(20.));
    assert_eq!(top(cx, 2) - first, px(60.));
    assert_eq!(top(cx, 3) - first, px(100.));

    // Clicking the lower half of the tall row 1 lands on it.
    let y = first + px(20. + 35.);
    let hit = editor.update_in(cx, |e, window, cx| {
        e.mouse_target(point(px(200.), y), window, cx)
    });
    assert_eq!(hit.position.map(|p| p.row), Some(1));

    // `allow_variable_line_heights: false` ignores them.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.allow_variable_line_heights = false;
        e.update_options(options, cx);
    });
    paint(cx);
    assert_eq!(top(cx, 3) - first, px(60.));
}
