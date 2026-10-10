//! Unicode highlighting in the editor (Monaco `UnicodeHighlighter`).

use std::time::Duration;

use gpui::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

const TEXT: &str = "fn main() {\n    let p\u{430}ssword = 1;\n    let x = 1;\u{200b}\n    let s = \"caf\u{e9}\";\n    // \u{430} in a comment\n}\n";

fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(TEXT).with_language("rust"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
    }
    cx.run_until_parked();
}

fn highlights(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    paint(cx);
    editor.read_with(cx, |e, cx| e.unicode_highlights_for_tests(cx))
}

#[gpui::test]
fn ambiguous_and_invisible_characters(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    // Not the comment's (include_comments is off) or the non-ASCII `é`.
    assert_eq!(highlights(&editor, cx), vec!["\u{430}", "\u{200b}"]);

    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.unicode_highlight.non_basic_ascii = true;
        options.unicode_highlight.include_comments = true;
        e.update_options(options, cx);
    });
    assert_eq!(
        highlights(&editor, cx),
        vec!["\u{430}", "\u{200b}", "\u{e9}", "\u{430}"]
    );
}

#[gpui::test]
fn hover_explains_and_excludes(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    paint(cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                1, 9,
            ))],
            cx,
        );
        e.show_hover_at_cursor(cx);
    });
    paint(cx);
    let reasons = editor.read_with(cx, |e, cx| e.hover_unicode_reasons_for_tests(cx));
    assert_eq!(
        reasons,
        vec![
            "The character `U+0430` \"`\u{430}`\" could be confused with the ASCII character \
             `U+0061` \"`a`\", which is more common in source code."
        ]
    );
}
