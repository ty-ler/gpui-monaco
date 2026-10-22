//! Monaco `copyWithSyntaxHighlighting` (`ViewModel.getRichTextToCopy`):
//! the HTML a copy puts beside the text.

use std::sync::Arc;

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel, Theme};

fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

fn editor<'a>(
    text: &str,
    language: Option<&'static str>,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut model = TextModel::new(&text);
        if let Some(language) = language {
            model = model.with_language(language);
        }
        let model = cx.new(|_| model);
        let mut editor = CodeEditor::new(model, window, cx);
        editor.set_theme(Arc::new(Theme::vs_dark()), cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn rich_text(
    editor: &Entity<CodeEditor>,
    selections: &[Selection],
    cx: &mut VisualTestContext,
) -> Option<String> {
    editor.update(cx, |e, cx| e.set_selections(selections, cx));
    editor.read_with(cx, |e, cx| e.rich_text_to_copy_for_tests(cx))
}

#[gpui::test]
fn selection_as_html_lines_of_colored_spans(cx: &mut TestAppContext) {
    let (editor, cx) = editor("fn main() {\n\n\tlet a = 1 < 2;\n}", Some("rust"), cx);
    let html = rich_text(&editor, &[Selection::new(p(1, 1), p(3, 16))], cx).unwrap();
    // The editor's colors and font.
    assert!(html.starts_with("<div style=\"color: #d4d4d4;background-color: #1e1e1e;"));
    assert!(html.contains("white-space: pre;\">"));
    assert!(html.ends_with("</div>"));
    // One `<div>` per line, `<br>` for the empty one.
    let body = &html[html.find("\">").unwrap() + 2..html.len() - "</div>".len()];
    assert!(body.starts_with("<div><span style=\"color: #"));
    assert_eq!(body.matches("<div>").count(), 2);
    assert!(body.contains("</div><br><div>"));
    // `fn` is a keyword, colored apart from the default foreground.
    assert!(body.starts_with("<div><span style=\"color: #569cd6;\">fn</span>"));
    // The tab is spaces to the next tab stop (4), and `<` is escaped.
    let text: String = {
        let mut out = String::new();
        let mut in_tag = false;
        for ch in body.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                ch if !in_tag => out.push(ch),
                _ => {}
            }
        }
        out
    };
    assert_eq!(text, "fn main() {    let a = 1 &lt; 2;");
}

#[gpui::test]
fn empty_selection_copies_its_line(cx: &mut TestAppContext) {
    let (editor, cx) = editor("fn a() {}\nfn b() {}", Some("rust"), cx);
    let html = rich_text(&editor, &[Selection::collapsed(p(2, 3))], cx).unwrap();
    assert!(html.contains(">b</span>"));
    assert!(!html.contains(">a</span>"));
    // Not with `emptySelectionClipboard` off.
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.empty_selection_clipboard = false, cx)
    });
    assert!(rich_text(&editor, &[Selection::collapsed(p(2, 3))], cx).is_none());
}

#[gpui::test]
fn no_html_for_several_selections_or_plain_text(cx: &mut TestAppContext) {
    let (editor, cx) = editor("fn a() {}\nfn b() {}", Some("rust"), cx);
    let two = [
        Selection::new(p(1, 1), p(1, 3)),
        Selection::new(p(2, 1), p(2, 3)),
    ];
    assert!(rich_text(&editor, &two, cx).is_none());
    let (plain, cx) = editor_plain(cx);
    assert!(rich_text(&plain, &[Selection::new(p(1, 1), p(1, 3))], cx).is_none());
}

fn editor_plain(cx: &mut VisualTestContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    let plain = cx.update(|window, cx| {
        let model = cx.new(|_| TextModel::new("plain text"));
        cx.new(|cx| CodeEditor::new(model, window, cx))
    });
    (plain, cx)
}
