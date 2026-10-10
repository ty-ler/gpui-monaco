//! Minimal markdown rendering for hovers, suggestion details and signature
//! help (Monaco renders `IMarkdownString` the same places). Fenced code
//! blocks are syntax highlighted with the registered languages and theme.

use std::ops::Range;

use gpui::{
    AnyElement, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText, IntoElement,
    ParentElement, Pixels, SharedString, Styled, StyledText, UnderlineStyle, div, px,
};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use syntax::LanguageRegistry;

use crate::theme::Theme;

/// Colors and fonts for rendered markdown.
#[derive(Clone, Debug)]
pub struct MarkdownStyle {
    /// Text color; also tints horizontal rules.
    pub foreground: Hsla,
    /// Color of link text and its underline.
    pub link: Hsla,
    /// Background of inline code spans and fenced code blocks.
    pub code_background: Hsla,
    /// Font family of code spans and fenced code blocks.
    pub code_font: SharedString,
    /// Base text size; headings are scaled up from it.
    pub font_size: Pixels,
}

#[derive(Default)]
struct Inline {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    /// Code spans, which use the code font.
    code: Vec<Range<usize>>,
    /// Link text and its destination.
    links: Vec<(Range<usize>, SharedString)>,
}

struct Renderer<'a> {
    style: &'a MarkdownStyle,
    theme: &'a Theme,
    registry: &'a LanguageRegistry,
    blocks: Vec<AnyElement>,
    inline: Inline,
    bold: usize,
    italic: usize,
    /// Destinations of the links being rendered (innermost last).
    links: Vec<SharedString>,
    heading: Option<u8>,
    list_stack: Vec<Option<u64>>,
    code_block: Option<(Option<String>, String)>,
}

impl Renderer<'_> {
    fn push_text(&mut self, text: &str, code: bool) {
        let start = self.inline.text.len();
        self.inline.text.push_str(text);
        let end = self.inline.text.len();
        let mut highlight = HighlightStyle::default();
        if self.bold > 0 || self.heading.is_some() {
            highlight.font_weight = Some(FontWeight::BOLD);
        }
        if self.italic > 0 {
            highlight.font_style = Some(FontStyle::Italic);
        }
        if let Some(url) = self.links.last() {
            match self.inline.links.last_mut() {
                Some((range, last)) if range.end == start && last == url => range.end = end,
                _ => self.inline.links.push((start..end, url.clone())),
            }
            highlight.color = Some(self.style.link);
            highlight.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(self.style.link),
                wavy: false,
            });
        }
        if code {
            highlight.background_color = Some(self.style.code_background);
            self.inline.code.push(start..end);
        }
        if highlight != HighlightStyle::default() {
            self.inline.highlights.push((start..end, highlight));
        }
    }

    fn flush_inline(&mut self) {
        let inline = std::mem::take(&mut self.inline);
        if inline.text.trim().is_empty() {
            return;
        }
        let size = match self.heading {
            Some(1) => self.style.font_size * 1.4,
            Some(2) => self.style.font_size * 1.2,
            Some(_) => self.style.font_size * 1.1,
            None => self.style.font_size,
        };
        let code_font = self.style.code_font.clone();
        let text = StyledText::new(inline.text)
            .with_highlights(inline.highlights)
            .with_font_family_overrides(
                inline
                    .code
                    .into_iter()
                    .map(|range| (range, code_font.clone())),
            );
        let block = if inline.links.is_empty() {
            text.into_any_element()
        } else {
            let (ranges, urls): (Vec<_>, Vec<_>) = inline.links.into_iter().unzip();
            InteractiveText::new(("markdown-links", self.blocks.len()), text)
                .on_click(ranges, move |ix, _, cx| {
                    if is_openable(&urls[ix]) {
                        cx.open_url(&urls[ix]);
                    }
                })
                .into_any_element()
        };
        self.blocks
            .push(div().text_size(size).child(block).into_any_element());
    }

    fn code_block(&mut self, language: Option<String>, code: String) {
        let code = code.trim_end_matches('\n').to_string();
        let highlights = language
            .as_deref()
            .and_then(|name| self.registry.language_for_name(name))
            .map(|language| {
                crate::colorize::colorize_highlights(&code, &language.id, self.theme, self.registry)
            })
            .unwrap_or_default();
        let text = StyledText::new(code).with_highlights(highlights);
        self.blocks.push(
            div()
                .font_family(self.style.code_font.clone())
                .text_size(self.style.font_size)
                .p(px(4.))
                .rounded(px(3.))
                .bg(self.style.code_background)
                .child(text)
                .into_any_element(),
        );
    }
}

/// Whether a link may be opened: Monaco's markdown renderer opens web and
/// mail links by default (`command:` and other schemes need a trusted
/// string and an opener, which hosts don't have here).
fn is_openable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// Renders `source` as a column of blocks. Links to web and mail addresses
/// open on click.
pub fn render_markdown(
    source: &str,
    style: &MarkdownStyle,
    theme: &Theme,
    registry: &LanguageRegistry,
) -> AnyElement {
    let mut renderer = Renderer {
        style,
        theme,
        registry,
        blocks: Vec::new(),
        inline: Inline::default(),
        bold: 0,
        italic: 0,
        links: Vec::new(),
        heading: None,
        list_stack: Vec::new(),
        code_block: None,
    };
    let parser = Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    );
    for event in parser {
        if let Some((_, code)) = renderer.code_block.as_mut() {
            match event {
                Event::Text(text) => {
                    code.push_str(&text);
                    continue;
                }
                Event::End(TagEnd::CodeBlock) => {
                    if let Some((language, code)) = renderer.code_block.take() {
                        renderer.code_block(language, code);
                    }
                    continue;
                }
                _ => continue,
            }
        }
        match event {
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => renderer.flush_inline(),
            Event::Start(Tag::Heading { level, .. }) => {
                renderer.flush_inline();
                renderer.heading = Some(level as u8);
            }
            Event::End(TagEnd::Heading(_)) => {
                renderer.flush_inline();
                renderer.heading = None;
            }
            Event::Start(Tag::Strong) => renderer.bold += 1,
            Event::End(TagEnd::Strong) => renderer.bold = renderer.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => renderer.italic += 1,
            Event::End(TagEnd::Emphasis) => renderer.italic = renderer.italic.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => {
                renderer.links.push(dest_url.to_string().into());
            }
            Event::End(TagEnd::Link) => {
                renderer.links.pop();
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                renderer.flush_inline();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|l| !l.is_empty())
                        .map(str::to_string),
                    CodeBlockKind::Indented => None,
                };
                renderer.code_block = Some((language, String::new()));
            }
            Event::Start(Tag::List(start)) => {
                renderer.flush_inline();
                renderer.list_stack.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                renderer.flush_inline();
                renderer.list_stack.pop();
            }
            Event::Start(Tag::Item) => {
                renderer.flush_inline();
                let depth = renderer.list_stack.len().saturating_sub(1);
                let bullet = match renderer.list_stack.last_mut() {
                    Some(Some(n)) => {
                        let bullet = format!("{n}. ");
                        *n += 1;
                        bullet
                    }
                    _ => "• ".to_string(),
                };
                renderer.push_text(&format!("{}{bullet}", "  ".repeat(depth)), false);
            }
            Event::End(TagEnd::Item) => renderer.flush_inline(),
            Event::Text(text) => renderer.push_text(&text, false),
            Event::Code(code) => renderer.push_text(&code, true),
            Event::SoftBreak => renderer.push_text(" ", false),
            Event::HardBreak => renderer.push_text("\n", false),
            Event::Rule => {
                renderer.flush_inline();
                renderer.blocks.push(
                    div()
                        .h(px(1.))
                        .bg(style.foreground.opacity(0.2))
                        .into_any_element(),
                );
            }
            _ => {}
        }
    }
    renderer.flush_inline();
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .text_color(style.foreground)
        .children(renderer.blocks)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openable_links() {
        assert!(is_openable("https://example.com"));
        assert!(is_openable("HTTP://example.com"));
        assert!(is_openable("mailto:someone@example.com"));
        assert!(!is_openable("command:editor.action.format"));
        assert!(!is_openable("javascript:alert(1)"));
        assert!(!is_openable("file:///etc/passwd"));
    }
}
