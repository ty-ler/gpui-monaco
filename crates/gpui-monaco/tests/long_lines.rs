//! Very long lines (Monaco `stopRenderingLineAfter`,
//! `maxTokenizationLineLength`) and chunked shaping.

use gpui::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext, point, px};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("javascript"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn minified(statements: usize) -> String {
    (0..statements)
        .map(|i| format!("function f{i}(a){{return [a,{i}]}}"))
        .collect::<Vec<_>>()
        .join(";")
}

#[gpui::test]
fn rendering_stops_after_the_limit(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&minified(2_000), cx);
    let max_left = |cx: &mut VisualTestContext| {
        paint(cx);
        f32::from(editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().1))
    };
    let limited = max_left(cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.stop_rendering_line_after = None;
        e.update_options(options, cx);
    });
    let full = max_left(cx);
    // ~60 000 characters drawn versus 10 000 (plus the "Show more" chip).
    assert!(full > limited * 4., "{full} vs {limited}");
}

#[gpui::test]
fn chunked_lines_hit_test_exactly(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&minified(300), cx);
    // Columns well past the first shaped chunk map to x and back.
    for column in [5u32, 900, 2_500, 7_000] {
        editor.update(cx, |e, cx| {
            e.reveal_position(PointUtf16::new(0, column), cx)
        });
        paint(cx);
        let at = editor.read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(0, column))
                .unwrap()
        });
        let hit = editor.update_in(cx, |e, window, cx| {
            e.mouse_target(point(at.x + px(1.), at.y + px(5.)), window, cx)
        });
        assert_eq!(
            hit.position,
            Some(PointUtf16::new(0, column)),
            "column {column}"
        );
    }
}

#[gpui::test]
fn typing_in_a_long_line(cx: &mut TestAppContext) {
    let text = minified(3_000);
    let (editor, cx) = setup(&text, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                0, 50_000,
            ))],
            cx,
        )
    });
    cx.simulate_input("x");
    paint(cx);
    let after = editor.read_with(cx, |e, cx| e.model().read(cx).text());
    assert_eq!(after.len(), text.len() + 1);
    assert_eq!(&after[50_000..50_001], "x");
}
