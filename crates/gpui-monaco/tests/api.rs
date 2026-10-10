//! Embedder API parity with Monaco's `ITextModel` / `ICodeEditor` helpers.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext, px};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, RevealType, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

#[gpui::test]
fn model_helpers(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("let foo_bar = 1;\nfoo(foo_bar)\n😀x"));
    model.read_with(cx, |m, _| {
        assert_eq!(m.line_count(), 3);
        assert_eq!(m.line_content(1), "foo(foo_bar)");
        assert_eq!(m.line_len_utf16(2), 3);
        // Validation clamps rows, columns and surrogate halves.
        assert_eq!(m.validate_position(p(9, 9)), p(2, 3));
        assert_eq!(m.validate_position(p(0, 99)), p(0, 16));
        assert_eq!(m.validate_position(p(2, 1)), p(2, 0));
        assert_eq!(m.validate_range(p(1, 3)..p(0, 4)), p(0, 4)..p(1, 3));
        assert_eq!(m.offset_at(p(1, 0)), 17);
        assert_eq!(m.position_at(17), p(1, 0));
        assert_eq!(m.value_in_range(p(0, 4)..p(1, 3)), "foo_bar = 1;\nfoo");

        let word = m.word_at_position(p(0, 6)).unwrap();
        assert_eq!(
            (word.word.as_str(), word.range),
            ("foo_bar", p(0, 4)..p(0, 11))
        );
        let until = m.word_until_position(p(0, 6));
        assert_eq!((until.word.as_str(), until.range), ("fo", p(0, 4)..p(0, 6)));
        assert!(m.word_at_position(p(0, 13)).is_none());

        assert_eq!(
            m.find_matches("foo", false, true, false, 10),
            vec![p(0, 4)..p(0, 7), p(1, 0)..p(1, 3), p(1, 4)..p(1, 7)]
        );
        assert_eq!(
            m.find_matches("foo", false, true, true, 10),
            vec![p(1, 0)..p(1, 3)]
        );
        assert_eq!(m.find_matches("f.o", true, true, false, 1).len(), 1);
    });
}

#[gpui::test]
fn reveal_and_visible_ranges(cx: &mut TestAppContext) {
    let text: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
    let (editor, cx) = setup(&text.join("\n"), cx);
    draw(cx);
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());

    editor.update(cx, |e, cx| e.reveal_line_in_center(250, cx));
    draw(cx);
    let ranges = editor.read_with(cx, |e, cx| e.visible_ranges(cx));
    let middle = (ranges[0].start.row + ranges[0].end.row) / 2;
    assert!(middle.abs_diff(250) <= 1, "{ranges:?}");

    // Already visible: `IfOutsideViewport` doesn't move.
    let top = editor.read_with(cx, |e, _| e.scroll_position().y);
    editor.update(cx, |e, cx| {
        e.reveal_line_in_center_if_outside_viewport(251, cx)
    });
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_position().y), top);

    editor.update(cx, |e, cx| {
        e.reveal_range(p(400, 0)..p(400, 0), RevealType::Top, cx)
    });
    draw(cx);
    let top = editor.read_with(cx, |e, _| e.scroll_position().y);
    // Sticky scroll keeps five context lines above.
    assert_eq!(top, line_height * 395.);

    // Minimal reveal of a line above the viewport.
    editor.update(cx, |e, cx| e.reveal_line(10, cx));
    draw(cx);
    let ranges = editor.read_with(cx, |e, cx| e.visible_ranges(cx));
    assert!(ranges[0].start.row <= 10 && 10 <= ranges[0].end.row);
    assert!(editor.read_with(cx, |e, _| e.scroll_position().y) > px(0.));
}

#[gpui::test]
fn execute_edits_and_trigger(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello world", cx);
    let changed = editor.update(cx, |e, cx| {
        e.execute_edits(
            vec![(p(0, 0)..p(0, 5), "goodbye".into())],
            Some(vec![Selection::collapsed(p(0, 7))]),
            cx,
        )
    });
    assert!(changed);
    let (text, head) = editor.read_with(cx, |e, cx| {
        (e.model().read(cx).text(), e.selections()[0].head)
    });
    assert_eq!((text.as_str(), head), ("goodbye world", p(0, 7)));
    // Undo takes the whole edit back.
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "hello world"
    );

    // Commands by name.
    editor.update_in(cx, |e, window, cx| {
        assert!(e.has_text_focus(window));
        e.trigger("code_editor::TransformToUppercase", window, cx)
            .unwrap();
        assert!(e.trigger("code_editor::NoSuchThing", window, cx).is_err());
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "HELLO world"
    );

    // Read-only editors refuse edits.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
        assert!(!e.execute_edits(vec![(p(0, 0)..p(0, 1), "x".into())], None, cx));
    });
}
