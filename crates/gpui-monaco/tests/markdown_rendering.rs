//! Markdown in hovers (Monaco's `MarkdownRenderer`): tables, code blocks,
//! the editor's line height, and codicons.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext, px};
use gpui_monaco::codicons::{Codicon, codicons_available, register_codicon_font};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

/// A hover showing whatever markdown the test sets.
struct MarkdownHover(Rc<RefCell<MarkdownString>>);
impl HoverProvider for MarkdownHover {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> gpui::Task<anyhow::Result<Option<Hover>>> {
        gpui::Task::ready(Ok(Some(Hover {
            contents: vec![self.0.borrow().clone()],
            range: None,
            ..Default::default()
        })))
    }
}

struct Setup<'a> {
    editor: Entity<CodeEditor>,
    content: Rc<RefCell<MarkdownString>>,
    cx: &'a mut VisualTestContext,
}

fn setup(cx: &mut TestAppContext) -> Setup<'_> {
    cx.update(gpui_monaco::init);
    let content = Rc::new(RefCell::new(MarkdownString::default()));
    let provider = MarkdownHover(content.clone());
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, provider));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("alpha beta gamma\n"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.run_until_parked();
    Setup {
        editor,
        content,
        cx,
    }
}

impl Setup<'_> {
    fn paint(&mut self) {
        self.cx.update(|window, _| window.refresh());
        self.cx.run_until_parked();
    }

    /// Shows a hover with `markdown` and returns its card's height.
    fn hover_height(&mut self, markdown: impl Into<MarkdownString>) -> gpui::Pixels {
        *self.content.borrow_mut() = markdown.into();
        self.editor
            .update(self.cx, |editor, cx| editor.hide_hover(cx));
        self.paint();
        self.cx
            .dispatch_action(gpui_monaco::actions::ShowHover::default());
        self.cx.run_until_parked();
        self.paint();
        self.paint();
        assert!(self.editor.read_with(self.cx, |e, _| e.is_hover_visible()));
        self.editor
            .read_with(self.cx, |e, cx| e.hover_card_bounds_for_tests(cx))
            .expect("hover laid out")
            .size
            .height
    }

    fn line_height(&mut self) -> gpui::Pixels {
        self.editor
            .read_with(self.cx, |e, _| e.options().resolved_line_height())
    }
}

fn code_block(lines: usize) -> String {
    let body: Vec<String> = (0..lines).map(|i| format!("let x{i} = {i};")).collect();
    format!("```rust\n{}\n```", body.join("\n"))
}

fn table(rows: usize) -> String {
    // Single characters, which can't wrap in the test platform's text
    // layout (its grid columns come out narrower than the app's).
    let mut table = String::from("| a | b |\n|---|--:|\n");
    for i in 0..rows {
        table.push_str(&format!("| {i} | {i} |\n"));
    }
    table
}

#[gpui::test]
fn code_blocks_use_the_editor_line_height(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    let line_height = setup.line_height();
    let short = setup.hover_height(code_block(2));
    let long = setup.hover_height(code_block(6));
    // Each line of a code block is one editor line tall (Monaco sets the
    // hover's line height to the editor's), not GPUI's default ~1.6em.
    let per_line = (long - short) / 4.;
    assert!(
        (per_line - line_height).abs() < px(0.5),
        "{per_line:?} per line, editor line height {line_height:?}"
    );
}

#[gpui::test]
fn tables_render_a_row_per_line(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    let line_height = setup.line_height();
    let one = setup.hover_height(table(1));
    let three = setup.hover_height(table(3));
    // Each body row is a line of text with 2px padding above and below
    // and a 1px separator, not text run together into one paragraph.
    let per_row = (three - one) / 2.;
    assert!(
        (per_row - (line_height + px(5.))).abs() < px(0.5),
        "{per_row:?} per row, editor line height {line_height:?}"
    );
}

#[gpui::test]
fn hovers_mix_tables_code_and_markup(cx: &mut TestAppContext) {
    let mut setup = setup(cx);
    let markdown = MarkdownString::new(format!(
        "# Title\n\n$(zap) **bold** ~~gone~~ `code`\n\n> quoted\n\n- a\n  - b\n\n{}\n\n{}\n\n![logo](not-a-url.png)\n\n<span style=\"color:#ff0000\">red</span><br>",
        table(2),
        code_block(2)
    ))
    .with_theme_icons(true)
    .with_html(true);
    let height = setup.hover_height(markdown);
    assert!(height > setup.line_height() * 8.);
    // The copy button copies the source.
    let contents = setup
        .editor
        .read_with(setup.cx, |e, cx| e.hover_contents_for_tests(cx));
    assert!(contents[0].starts_with("# Title"));
}

/// Markdown in a narrow box, with a style.
struct Narrow(MarkdownString, gpui_monaco::markdown::MarkdownStyle);

impl gpui::Render for Narrow {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};
        let theme = gpui_monaco::theme::Theme::vs_dark();
        gpui::div()
            .w(px(60.))
            .child(gpui_monaco::markdown::render_markdown(
                &self.0, &self.1, &theme, cx,
            ))
    }
}

/// Monaco's suggest details and parameter hints round inline code
/// (`border-radius: 3px`): the backgrounds drawn under the text follow it
/// when it wraps.
#[gpui::test]
fn rounded_inline_code_wraps(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let colors = gpui_monaco::theme::EditorColors::vs_dark();
    let style = gpui_monaco::markdown::MarkdownStyle {
        foreground: colors.foreground,
        link: colors.text_link_foreground,
        code_background: colors.text_code_block_background,
        code_corner_radius: px(3.),
        code_font: "Menlo".into(),
        font_size: px(13.),
        line_height: 1.4,
        separator: colors.text_separator_foreground,
        quote_background: None,
        quote_border: colors.text_block_quote_border,
    };
    let markdown = MarkdownString::new("call `a_rather_long_function_name()` then `b`");
    let (view, cx) = cx.add_window_view(|_, _| Narrow(markdown, style));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    // Square backgrounds too.
    view.update(cx, |view, cx| {
        view.1.code_corner_radius = px(0.);
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn plain_text_documentation_is_not_markdown(cx: &mut TestAppContext) {
    let plain: Documentation = "*not emphasis*\n    indented".into();
    assert_eq!(
        plain,
        Documentation::PlainText("*not emphasis*\n    indented".into())
    );
    let markdown: Documentation = MarkdownString::new("*emphasis*").into();
    assert!(matches!(markdown, Documentation::Markdown(_)));
    // As hover markdown (inlay hint tooltips), plain text is escaped.
    assert_eq!(
        plain.to_markdown().value.as_ref(),
        "\\*not emphasis\\*\n\n\u{a0}\u{a0}\u{a0}\u{a0}indented"
    );
    // Codicons fall back when the test platform has no fonts.
    let available = cx.update(register_codicon_font);
    assert_eq!(cx.update(|cx| codicons_available(cx)), available);
    assert_eq!(
        Codicon::from_id("symbol-method"),
        Some(Codicon::SymbolMethod)
    );
}
