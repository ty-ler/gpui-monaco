//! Markdown rendering for hovers, suggestion details, parameter hints and
//! messages (Monaco's `MarkdownRenderer` over marked, where Monaco renders
//! an `IMarkdownString`).
//!
//! Supported: paragraphs, headings, emphasis, strong, strikethrough, inline
//! code (`textCodeBlock.background`, padded), fenced and indented code
//! blocks (syntax highlighted with the registered languages and theme, in
//! the editor font, with no background, as Monaco's tokenized `div.code`),
//! links (`textLink.foreground`; web and mail links open, `command:` links
//! run commands when [`MarkdownString::is_trusted`]), block quotes, nested
//! lists with hanging markers (`padding-left: 20px`), GFM tables, rules,
//! images (`http(s):` and base64 `data:` URIs load through GPUI's `img`,
//! anything else shows its alt text), `$(icon)` codicons when
//! [`MarkdownString::support_theme_icons`], and with
//! [`MarkdownString::support_html`] a sanitized HTML subset (see
//! [`HTML_TAGS`]).
//!
//! The text is laid out by GPUI's `StyledText`, which can't be selected:
//! hovers copy through their copy button, and suggest details through
//! `editor.action.suggestWidgetCopy` (Monaco lets the text be selected).
//! Inline code is padded with spaces in its background, rounded by
//! [`MarkdownStyle::code_corner_radius`] (the backgrounds are drawn under
//! the text, as GPUI's text backgrounds are square), and an image inside a
//! paragraph is placed on its own line.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, App, BorderStyle, Bounds, Corners, Edges, FontStyle, FontWeight, HighlightStyle,
    Hsla, Image, ImageFormat, ImageSource, InteractiveText, IntoElement, ParentElement, Pixels,
    Point, SharedString, SharedUri, StrikethroughStyle, Styled, StyledImage as _, StyledText,
    UnderlineStyle, canvas, div, img, point, px, quad, relative,
};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::codicons::{self, Codicon, LabelPart};
use crate::providers::{Command, Documentation, MarkdownString};
use crate::theme::Theme;

/// Colors and fonts for rendered markdown.
#[derive(Clone, Debug)]
pub struct MarkdownStyle {
    /// Text color.
    pub foreground: Hsla,
    /// Link text and underline (`textLink.foreground`).
    pub link: Hsla,
    /// Background of inline code spans (`textCodeBlock.background`).
    pub code_background: Hsla,
    /// Corner radius of inline code backgrounds: Monaco's suggest details
    /// and parameter hints round them by 3px (`border-radius: 3px`); its
    /// hover CSS doesn't (zero).
    pub code_corner_radius: Pixels,
    /// Font family of code spans and code blocks (the editor's).
    pub code_font: SharedString,
    /// Base text size; headings are scaled up from it.
    pub font_size: Pixels,
    /// Line height as a multiple of the font size (Monaco sets the hover's
    /// and parameter hints' `line-height` to the editor's line height over
    /// its font size, and the suggest details' to the suggest line height).
    pub line_height: f32,
    /// Horizontal rules and table borders.
    pub separator: Hsla,
    /// Block quote background (`textBlockQuote.background`), if any.
    pub quote_background: Option<Hsla>,
    /// Block quote left border (`textBlockQuote.border`).
    pub quote_border: Hsla,
}

/// The HTML tags rendered when [`MarkdownString::support_html`] is set
/// (a subset of Monaco's `allowedMarkdownHtmlTags`). `span` takes
/// `style="color: #rrggbb; background-color: #rrggbb"` (Monaco allows only
/// these styles), `a` takes `href`, `img` takes `src`, `alt`, `width` and
/// `height`. Other tags are dropped and their text kept, except `script`
/// and `style`, which are dropped with their content. Without
/// `support_html` every tag is dropped.
pub const HTML_TAGS: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "br",
    "code",
    "del",
    "div",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "p",
    "s",
    "samp",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "tt",
    "u",
];

/// Inline formatting of a run of text.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
    pub code: bool,
    pub color: Option<Hsla>,
    pub background: Option<Hsla>,
}

/// A paragraph's text with its formatting.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Inline {
    pub text: String,
    /// Formatted runs (unformatted text has none).
    pub spans: Vec<(Range<usize>, SpanStyle)>,
    /// Link text and its destination.
    pub links: Vec<(Range<usize>, SharedString)>,
    /// Codicon characters, drawn with the codicon font.
    pub icons: Vec<Range<usize>>,
}

impl Inline {
    fn is_blank(&self) -> bool {
        self.text.trim().is_empty() && self.icons.is_empty()
    }
}

/// A rendered block (marked's block tokens).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Block {
    Paragraph(Inline),
    Heading(u8, Inline),
    Code {
        language: Option<String>,
        code: String,
    },
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Table {
        alignments: Vec<Alignment>,
        head: Vec<Inline>,
        rows: Vec<Vec<Inline>>,
    },
    Image {
        url: String,
        alt: String,
        width: Option<f32>,
        height: Option<f32>,
    },
    Rule,
}

/// A container being built.
enum Frame {
    Blocks(Vec<Block>),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Item(Vec<Block>),
    Table {
        alignments: Vec<Alignment>,
        head: Vec<Inline>,
        rows: Vec<Vec<Inline>>,
        in_head: bool,
    },
}

/// Builds [`Block`]s from pulldown-cmark events.
struct Builder<'a> {
    markdown: &'a MarkdownString,
    icons_available: bool,
    frames: Vec<Frame>,
    inline: Inline,
    heading: Option<u8>,
    bold: usize,
    italic: usize,
    strikethrough: usize,
    underline: usize,
    /// HTML `<code>`, `<kbd>`, ... nesting.
    html_code: usize,
    /// HTML `<span style>` colors (innermost last).
    colors: Vec<(Option<Hsla>, Option<Hsla>)>,
    /// Open HTML tags that pushed a color, a link or nothing.
    html_stack: Vec<String>,
    /// Destinations of the links being rendered (innermost last).
    links: Vec<SharedString>,
    /// Inside `<script>` or `<style>`: dropped.
    skipping: usize,
    code_block: Option<(Option<String>, String)>,
    /// An image's destination and its alt text so far.
    image: Option<(String, String)>,
}

impl Builder<'_> {
    fn style(&self, code: bool) -> SpanStyle {
        let (color, background) = self
            .colors
            .iter()
            .rev()
            .fold((None, None), |(color, background), (c, b)| {
                (color.or(*c), background.or(*b))
            });
        SpanStyle {
            bold: self.bold > 0 || self.heading.is_some(),
            italic: self.italic > 0,
            strikethrough: self.strikethrough > 0,
            underline: self.underline > 0,
            code: code || self.html_code > 0,
            color,
            background,
        }
    }

    /// Appends `text`, rendering `$(icon)`s when the string supports them.
    fn push_text(&mut self, text: &str, code: bool) {
        if self.skipping > 0 {
            return;
        }
        if let Some((_, alt)) = self.image.as_mut() {
            alt.push_str(text);
            return;
        }
        if self.markdown.support_theme_icons && !code {
            for part in codicons::parse_label_with_icons(text) {
                match part {
                    LabelPart::Text(text) => self.push_run(text, false),
                    LabelPart::Icon(icon) => self.push_icon(icon),
                }
            }
        } else if code {
            // Monaco pads inline code by 0.4em: a three-per-em space on
            // each side, inside the code background.
            self.push_run(&format!("\u{2004}{text}\u{2004}"), true);
        } else {
            self.push_run(text, false);
        }
    }

    fn push_icon(&mut self, icon: Codicon) {
        if self.icons_available {
            let start = self.inline.text.len();
            self.push_run(&icon.char().to_string(), false);
            self.inline.icons.push(start..self.inline.text.len());
        } else {
            self.push_run(icon.fallback(), false);
        }
    }

    fn push_run(&mut self, text: &str, code: bool) {
        if text.is_empty() {
            return;
        }
        let start = self.inline.text.len();
        self.inline.text.push_str(text);
        let end = self.inline.text.len();
        let style = self.style(code);
        if style != SpanStyle::default() {
            match self.inline.spans.last_mut() {
                Some((range, last)) if range.end == start && *last == style => range.end = end,
                _ => self.inline.spans.push((start..end, style)),
            }
        }
        if let Some(url) = self.links.last() {
            match self.inline.links.last_mut() {
                Some((range, last)) if range.end == start && last == url => range.end = end,
                _ => self.inline.links.push((start..end, url.clone())),
            }
        }
    }

    fn push_block(&mut self, block: Block) {
        match self.frames.last_mut() {
            Some(Frame::Blocks(blocks) | Frame::Quote(blocks) | Frame::Item(blocks)) => {
                blocks.push(block)
            }
            // Loose content directly in a list or table: keep it in a new
            // item, or drop it.
            Some(Frame::List { items, .. }) => items.push(vec![block]),
            Some(Frame::Table { .. }) | None => {}
        }
    }

    fn flush_inline(&mut self) {
        let inline = std::mem::take(&mut self.inline);
        if matches!(self.frames.last(), Some(Frame::Table { .. })) || inline.is_blank() {
            return;
        }
        let block = match self.heading {
            Some(level) => Block::Heading(level, inline),
            None => Block::Paragraph(inline),
        };
        self.push_block(block);
    }

    fn close_frame(&mut self) {
        self.flush_inline();
        let Some(frame) = self.frames.pop() else {
            return;
        };
        let block = match frame {
            Frame::Blocks(blocks) | Frame::Quote(blocks) if blocks.is_empty() => return,
            Frame::Blocks(blocks) | Frame::Quote(blocks) => Block::Quote(blocks),
            Frame::List { start, items } => Block::List { start, items },
            Frame::Item(blocks) => {
                if let Some(Frame::List { items, .. }) = self.frames.last_mut() {
                    items.push(blocks);
                }
                return;
            }
            Frame::Table {
                alignments,
                head,
                rows,
                ..
            } => Block::Table {
                alignments,
                head,
                rows,
            },
        };
        self.push_block(block);
    }

    fn image(&mut self, url: String, alt: String, width: Option<f32>, height: Option<f32>) {
        if matches!(self.frames.last(), Some(Frame::Table { .. })) {
            // Images in table cells show their alt text.
            self.push_run(&alt, false);
            return;
        }
        self.flush_inline();
        self.push_block(Block::Image {
            url,
            alt,
            width,
            height,
        });
    }

    fn event(&mut self, event: Event) {
        if let Some((_, code)) = self.code_block.as_mut() {
            match event {
                Event::Text(text) => code.push_str(&text),
                Event::End(TagEnd::CodeBlock) => {
                    if let Some((language, code)) = self.code_block.take() {
                        let code = code.trim_end_matches('\n').to_string();
                        self.push_block(Block::Code { language, code });
                    }
                }
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => self.flush_inline(),
            Event::Start(Tag::Heading { level, .. }) => {
                self.flush_inline();
                self.heading = Some(level as u8);
            }
            Event::End(TagEnd::Heading(_)) => {
                self.flush_inline();
                self.heading = None;
            }
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => self.strikethrough += 1,
            Event::End(TagEnd::Strikethrough) => {
                self.strikethrough = self.strikethrough.saturating_sub(1)
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                self.links.push(dest_url.to_string().into());
            }
            Event::End(TagEnd::Link) => {
                self.links.pop();
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                self.image = Some((dest_url.to_string(), String::new()));
            }
            Event::End(TagEnd::Image) => {
                if let Some((url, alt)) = self.image.take() {
                    self.image(url, alt, None, None);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                self.flush_inline();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|l| !l.is_empty())
                        .map(str::to_string),
                    CodeBlockKind::Indented => None,
                };
                self.code_block = Some((language, String::new()));
            }
            Event::Start(Tag::BlockQuote(_)) => {
                self.flush_inline();
                self.frames.push(Frame::Quote(Vec::new()));
            }
            Event::End(TagEnd::BlockQuote(_)) => self.close_frame(),
            Event::Start(Tag::List(start)) => {
                self.flush_inline();
                self.frames.push(Frame::List {
                    start,
                    items: Vec::new(),
                });
            }
            Event::End(TagEnd::List(_)) => self.close_frame(),
            Event::Start(Tag::Item) => {
                self.flush_inline();
                self.frames.push(Frame::Item(Vec::new()));
            }
            Event::End(TagEnd::Item) => self.close_frame(),
            Event::Start(Tag::Table(alignments)) => {
                self.flush_inline();
                self.frames.push(Frame::Table {
                    alignments,
                    head: Vec::new(),
                    rows: Vec::new(),
                    in_head: false,
                });
            }
            Event::End(TagEnd::Table) => self.close_frame(),
            Event::Start(Tag::TableHead) => {
                if let Some(Frame::Table { in_head, .. }) = self.frames.last_mut() {
                    *in_head = true;
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(Frame::Table { in_head, .. }) = self.frames.last_mut() {
                    *in_head = false;
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(Frame::Table { rows, .. }) = self.frames.last_mut() {
                    rows.push(Vec::new());
                }
            }
            Event::Start(Tag::TableCell) => self.inline = Inline::default(),
            Event::End(TagEnd::TableCell) => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(Frame::Table {
                    head,
                    rows,
                    in_head,
                    ..
                }) = self.frames.last_mut()
                {
                    match (*in_head, rows.last_mut()) {
                        (true, _) => head.push(cell),
                        (false, Some(row)) => row.push(cell),
                        (false, None) => rows.push(vec![cell]),
                    }
                }
            }
            Event::Text(text) => self.push_text(&text, false),
            Event::Code(code) => self.push_text(&code, true),
            Event::SoftBreak => self.push_text(" ", false),
            Event::HardBreak => self.push_text("\n", false),
            Event::Rule => {
                self.flush_inline();
                self.push_block(Block::Rule);
            }
            Event::Html(html) | Event::InlineHtml(html) => self.html(&html),
            _ => {}
        }
    }

    /// Raw HTML: Monaco's sanitized subset when the string supports HTML,
    /// else the tags are dropped and their text kept.
    fn html(&mut self, html: &str) {
        for token in html_tokens(html) {
            match token {
                HtmlToken::Text(text) => {
                    // Block HTML keeps its source line breaks; they're
                    // whitespace in HTML.
                    let text = decode_entities(&text.replace('\n', " "));
                    self.push_text(&text, false);
                }
                HtmlToken::Open { name, attributes } => {
                    if name == "script" || name == "style" {
                        self.skipping += 1;
                        continue;
                    }
                    if self.markdown.support_html {
                        self.open_tag(&name, &attributes);
                    }
                }
                HtmlToken::Close(name) => {
                    if name == "script" || name == "style" {
                        self.skipping = self.skipping.saturating_sub(1);
                        continue;
                    }
                    if self.markdown.support_html {
                        self.close_tag(&name);
                    }
                }
            }
        }
    }

    fn open_tag(&mut self, name: &str, attributes: &[(String, String)]) {
        let attribute = |key: &str| {
            attributes
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };
        match name {
            "b" | "strong" => self.bold += 1,
            "i" | "em" => self.italic += 1,
            "s" | "del" | "strike" => self.strikethrough += 1,
            "u" | "ins" => self.underline += 1,
            "code" | "kbd" | "samp" | "tt" => self.html_code += 1,
            "br" => self.push_text("\n", false),
            "p" | "div" | "blockquote" => self.flush_inline(),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush_inline();
                self.heading = name[1..].parse().ok();
            }
            "hr" => {
                self.flush_inline();
                self.push_block(Block::Rule);
            }
            "span" => {
                let style = attribute("style").unwrap_or_default();
                self.colors.push(span_colors(&style));
                self.html_stack.push("span".into());
            }
            "a" => {
                self.links
                    .push(attribute("href").unwrap_or_default().into());
                self.html_stack.push("a".into());
            }
            "img" => {
                if let Some(src) = attribute("src") {
                    let alt = attribute("alt").unwrap_or_default();
                    let size =
                        |key| attribute(key).and_then(|v| v.trim_end_matches("px").parse().ok());
                    self.image(src, alt, size("width"), size("height"));
                }
            }
            _ => {}
        }
    }

    fn close_tag(&mut self, name: &str) {
        match name {
            "b" | "strong" => self.bold = self.bold.saturating_sub(1),
            "i" | "em" => self.italic = self.italic.saturating_sub(1),
            "s" | "del" | "strike" => self.strikethrough = self.strikethrough.saturating_sub(1),
            "u" | "ins" => self.underline = self.underline.saturating_sub(1),
            "code" | "kbd" | "samp" | "tt" => self.html_code = self.html_code.saturating_sub(1),
            "p" | "div" | "blockquote" => self.flush_inline(),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush_inline();
                self.heading = None;
            }
            "span" | "a" => {
                if let Some(ix) = self.html_stack.iter().rposition(|tag| tag == name) {
                    self.html_stack.remove(ix);
                    if name == "span" {
                        self.colors.pop();
                    } else {
                        self.links.pop();
                    }
                }
            }
            _ => {}
        }
    }
}

/// Monaco's allowed `span` styles: `color` and `background-color` as hex.
fn span_colors(style: &str) -> (Option<Hsla>, Option<Hsla>) {
    let mut color = None;
    let mut background = None;
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        match property.trim().to_ascii_lowercase().as_str() {
            "color" => color = crate::theme::parse_hex_color(value),
            "background-color" => background = crate::theme::parse_hex_color(value),
            _ => {}
        }
    }
    (color, background)
}

#[derive(Debug, PartialEq)]
enum HtmlToken {
    Text(String),
    Open {
        name: String,
        attributes: Vec<(String, String)>,
    },
    Close(String),
}

/// Splits raw HTML into text, opening tags (with lowercased names and
/// attributes) and closing tags. Comments and declarations are dropped.
fn html_tokens(html: &str) -> Vec<HtmlToken> {
    let mut tokens = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            tokens.push(HtmlToken::Text(rest.to_string()));
            break;
        };
        if lt > 0 {
            tokens.push(HtmlToken::Text(rest[..lt].to_string()));
        }
        rest = &rest[lt..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        let Some(gt) = rest.find('>') else {
            tokens.push(HtmlToken::Text(rest.to_string()));
            break;
        };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            tokens.push(HtmlToken::Close(name.trim().to_ascii_lowercase()));
            continue;
        }
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        let tag = tag.trim_end_matches('/');
        let name_end = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
        let name = tag[..name_end].to_ascii_lowercase();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
            tokens.push(HtmlToken::Text(format!("<{tag}>")));
            continue;
        }
        tokens.push(HtmlToken::Open {
            name,
            attributes: html_attributes(&tag[name_end..]),
        });
    }
    tokens
}

fn html_attributes(mut text: &str) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    loop {
        text = text.trim_start();
        let key_end = text
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(text.len());
        if key_end == 0 {
            return attributes;
        }
        let key = text[..key_end].to_ascii_lowercase();
        text = text[key_end..].trim_start();
        let value = if let Some(after) = text.strip_prefix('=') {
            let after = after.trim_start();
            let (value, rest) = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let inner = &after[1..];
                    let end = inner.find(quote).unwrap_or(inner.len());
                    (&inner[..end], inner.get(end + 1..).unwrap_or(""))
                }
                _ => {
                    let end = after.find(char::is_whitespace).unwrap_or(after.len());
                    (&after[..end], &after[end..])
                }
            };
            text = rest;
            decode_entities(value)
        } else {
            String::new()
        };
        attributes.push((key, value));
    }
}

/// Decodes the common HTML entities (pulldown-cmark already decodes them
/// in markdown text).
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

/// Parses `markdown` into blocks (marked with GFM: tables and
/// strikethrough). `icons_available` says whether `$(icon)`s become
/// codicon characters or their Unicode stand-ins.
pub(crate) fn parse(markdown: &MarkdownString, icons_available: bool) -> Vec<Block> {
    let source: std::borrow::Cow<str> = if markdown.support_theme_icons {
        // Monaco `markdownEscapeEscapedIcons`: keep `\$(icon)` escaped
        // through markdown's own backslash escapes.
        markdown.value.replace("\\$(", "\\\\$(").into()
    } else {
        markdown.value.as_ref().into()
    };
    let mut builder = Builder {
        markdown,
        icons_available,
        frames: vec![Frame::Blocks(Vec::new())],
        inline: Inline::default(),
        heading: None,
        bold: 0,
        italic: 0,
        strikethrough: 0,
        underline: 0,
        html_code: 0,
        colors: Vec::new(),
        html_stack: Vec::new(),
        links: Vec::new(),
        skipping: 0,
        code_block: None,
        image: None,
    };
    let parser = Parser::new_ext(
        &source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    );
    for event in parser {
        builder.event(event);
    }
    builder.flush_inline();
    while builder.frames.len() > 1 {
        builder.close_frame();
    }
    match builder.frames.pop() {
        Some(Frame::Blocks(blocks)) => blocks,
        _ => Vec::new(),
    }
}

/// Whether a link may be opened: Monaco's markdown renderer opens web and
/// mail links by default (`command:` links need a trusted string).
fn is_openable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// A `command:id?args` link's command (Monaco's `command:` links in trusted
/// markdown): `args` is URI-encoded JSON, an array of arguments or a single
/// one.
fn command_link(url: &str) -> Option<Command> {
    let rest = url.strip_prefix("command:")?;
    let (id, query) = match rest.split_once('?') {
        Some((id, query)) => (id, Some(query)),
        None => (rest, None),
    };
    if id.is_empty() {
        return None;
    }
    let arguments = match query.map(percent_decode) {
        None => Vec::new(),
        Some(query) => match serde_json::from_str(&query) {
            Ok(serde_json::Value::Array(values)) => values,
            Ok(value) => vec![value],
            Err(_) => Vec::new(),
        },
    };
    Some(Command {
        id: percent_decode(id).into(),
        title: SharedString::default(),
        tooltip: None,
        arguments,
    })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = text
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Opens a clicked link as Monaco's `openLinkFromMarkdown` does, through
/// the opener service: the [link openers](crate::register_link_opener)
/// first, then web and mail links in the browser, and `command:` links of
/// trusted strings. `internal:` links aren't openable.
fn open_link(url: &str, trusted: bool, cx: &mut App) {
    let scheme = url.split_once(':').map(|(scheme, _)| scheme);
    if scheme.is_some_and(|scheme| scheme.eq_ignore_ascii_case("internal")) {
        return;
    }
    if crate::openers::offer_link(url, cx) {
        return;
    }
    if is_openable(url) {
        cx.open_url(url);
    } else if trusted
        && let Some(command) = command_link(url)
        && let Err(error) = crate::execute_command(cx, &command)
    {
        log::warn!("markdown command link: {error}");
    }
}

/// Decodes a base64 `data:image/...` URI.
fn data_uri_image(url: &str) -> Option<Arc<Image>> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    let format = ImageFormat::from_mime_type(mime)?;
    Some(Arc::new(Image::from_bytes(format, base64_decode(data)?)))
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// The background boxes of a span of text from its character boundaries'
/// positions (in order): one per line it's wrapped over, from the span's
/// start (or the line's `left` after a wrap) to its last boundary there.
fn code_background_bounds(
    points: &[Point<Pixels>],
    left: Pixels,
    line_height: Pixels,
) -> Vec<Bounds<Pixels>> {
    let mut boxes = Vec::new();
    let mut ix = 0;
    while ix < points.len() {
        let y = points[ix].y;
        let start = if ix == 0 { points[0].x } else { left };
        let mut end = start;
        while let Some(at) = points.get(ix).filter(|at| at.y == y) {
            end = end.max(at.x);
            ix += 1;
        }
        if end > start {
            boxes.push(Bounds::from_corners(
                point(start, y),
                point(end, y + line_height),
            ));
        }
    }
    boxes
}

/// Renders blocks with a style.
struct Renderer<'a> {
    style: &'a MarkdownStyle,
    theme: &'a Theme,
    registry: &'a syntax::LanguageRegistry,
    trusted: bool,
    /// Ids for interactive text.
    next_id: usize,
    /// Takes a clicked link first; it's opened as usual unless this says
    /// it handled it.
    on_link: Option<LinkHandler>,
}

/// Handles a clicked markdown link's URL; returns whether it did.
pub(crate) type LinkHandler = std::rc::Rc<dyn Fn(&str, &mut gpui::Window, &mut App) -> bool>;

impl Renderer<'_> {
    fn inline(&mut self, inline: Inline, font_size: Pixels) -> AnyElement {
        let style = self.style;
        // Rounded code backgrounds are drawn under the text, at each
        // character boundary's position (to follow wrapping).
        let rounded_code: Vec<Vec<usize>> = if style.code_corner_radius > px(0.) {
            inline
                .spans
                .iter()
                .filter(|(_, span)| span.code && span.background.is_none())
                .map(|(range, _)| {
                    inline.text[range.clone()]
                        .char_indices()
                        .map(|(ix, _)| range.start + ix)
                        .chain([range.end])
                        .collect()
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = inline
            .spans
            .iter()
            .map(|(range, span)| {
                let mut highlight = HighlightStyle::default();
                if span.bold {
                    highlight.font_weight = Some(FontWeight::BOLD);
                }
                if span.italic {
                    highlight.font_style = Some(FontStyle::Italic);
                }
                if span.strikethrough {
                    highlight.strikethrough = Some(StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    });
                }
                if span.underline {
                    highlight.underline = Some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    });
                }
                highlight.color = span.color;
                highlight.background_color = span
                    .background
                    .or((span.code && rounded_code.is_empty()).then_some(style.code_background));
                (range.clone(), highlight)
            })
            .collect();
        for (range, _) in &inline.links {
            highlights.push((
                range.clone(),
                HighlightStyle {
                    color: Some(style.link),
                    ..HighlightStyle::default()
                },
            ));
        }
        // Icons get a run of their own, for their font override.
        for range in &inline.icons {
            highlights.push((range.clone(), HighlightStyle::default()));
        }
        let highlights = merge_highlights(highlights);
        let code_ranges = inline
            .spans
            .iter()
            .filter(|(_, span)| span.code)
            .map(|(range, _)| (range.clone(), style.code_font.clone()));
        let icon_ranges = inline.icons.iter().map(|range| {
            (
                range.clone(),
                SharedString::from(codicons::CODICON_FONT_FAMILY),
            )
        });
        let mut overrides: Vec<_> = code_ranges.chain(icon_ranges).collect();
        overrides.sort_by_key(|(range, _)| range.start);
        let text = StyledText::new(inline.text)
            .with_highlights(highlights)
            .with_font_family_overrides(overrides);
        let code_backgrounds = (!rounded_code.is_empty()).then(|| {
            let layout = text.layout().clone();
            let (radius, color) = (style.code_corner_radius, style.code_background);
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let line_height = layout.line_height();
                    let left = layout.bounds().left();
                    for boundaries in &rounded_code {
                        let points: Vec<_> = boundaries
                            .iter()
                            .filter_map(|ix| layout.position_for_index(*ix))
                            .collect();
                        for bounds in code_background_bounds(&points, left, line_height) {
                            window.paint_quad(quad(
                                bounds,
                                Corners::all(radius),
                                color,
                                Edges::default(),
                                gpui::transparent_black(),
                                BorderStyle::Solid,
                            ));
                        }
                    }
                },
            )
            .absolute()
            .size_full()
        });
        let element = if inline.links.is_empty() {
            text.into_any_element()
        } else {
            self.next_id += 1;
            let trusted = self.trusted;
            let on_link = self.on_link.clone();
            let (ranges, urls): (Vec<_>, Vec<_>) = inline.links.into_iter().unzip();
            InteractiveText::new(("markdown-links", self.next_id), text)
                .on_click(ranges, move |ix, window, cx| {
                    if on_link
                        .as_ref()
                        .is_some_and(|on_link| on_link(&urls[ix], window, cx))
                    {
                        return;
                    }
                    open_link(&urls[ix], trusted, cx)
                })
                .into_any_element()
        };
        div()
            .text_size(font_size)
            .when_some(code_backgrounds, |div, backgrounds| {
                div.relative().child(backgrounds)
            })
            .child(element)
            .into_any_element()
    }

    fn blocks(&mut self, blocks: Vec<Block>, depth: usize) -> Vec<AnyElement> {
        blocks
            .into_iter()
            .map(|block| self.block(block, depth))
            .collect()
    }

    fn block(&mut self, block: Block, depth: usize) -> AnyElement {
        let style = self.style;
        match block {
            Block::Paragraph(inline) => self.inline(inline, style.font_size),
            Block::Heading(level, inline) => {
                let size = match level {
                    1 => style.font_size * 1.4,
                    2 => style.font_size * 1.2,
                    _ => style.font_size * 1.1,
                };
                self.inline(inline, size)
            }
            Block::Code { language, code } => {
                let highlights = language
                    .as_deref()
                    .and_then(|name| self.registry.language_for_name(name))
                    .map(|language| {
                        crate::colorize::colorize_highlights(
                            &code,
                            &language.id,
                            self.theme,
                            self.registry,
                        )
                    })
                    .unwrap_or_default();
                // Monaco's tokenized `div.code`: the editor font,
                // `white-space: pre-wrap`, no background.
                div()
                    .font_family(style.code_font.clone())
                    .text_size(style.font_size)
                    .child(StyledText::new(code).with_highlights(highlights))
                    .into_any_element()
            }
            Block::Quote(blocks) => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .pl(px(10.))
                .pr(px(16.))
                .border_l(px(5.))
                .border_color(style.quote_border)
                .when_some(style.quote_background, |this, background| {
                    this.bg(background)
                })
                .children(self.blocks(blocks, depth))
                .into_any_element(),
            Block::List { start, items } => {
                let mut number = start;
                let rows: Vec<AnyElement> = items
                    .into_iter()
                    .map(|item| {
                        let marker: SharedString = match number.as_mut() {
                            Some(n) => {
                                let marker = format!("{n}.");
                                *n += 1;
                                marker.into()
                            }
                            None => ["•", "◦", "▪"][depth % 3].into(),
                        };
                        // `ul, ol { padding-left: 20px }`: the marker hangs
                        // in the padding, and wrapped lines and nested
                        // blocks line up with the item's text.
                        div()
                            .flex()
                            .child(
                                div()
                                    .flex_none()
                                    .min_w(px(20.))
                                    .pr(px(6.))
                                    .flex()
                                    .justify_end()
                                    .child(marker),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .children(self.blocks(item, depth + 1)),
                            )
                            .into_any_element()
                    })
                    .collect();
                div().flex().flex_col().children(rows).into_any_element()
            }
            Block::Table {
                alignments,
                head,
                rows,
            } => self.table(alignments, head, rows),
            Block::Image {
                url,
                alt,
                width,
                height,
            } => image_element(url, alt, width, height, style),
            Block::Rule => div().h(px(1.)).bg(style.separator).into_any_element(),
        }
    }

    /// A GFM table (marked's `<table>`): columns sized to their content,
    /// a bold header row over a separator, and rows split by separators.
    fn table(
        &mut self,
        alignments: Vec<Alignment>,
        head: Vec<Inline>,
        rows: Vec<Vec<Inline>>,
    ) -> AnyElement {
        let style = self.style;
        let columns = rows
            .iter()
            .map(Vec::len)
            .chain([head.len(), alignments.len()])
            .max()
            .unwrap_or(0)
            .max(1);
        let row_count = rows.len();
        let mut cells = Vec::new();
        let all_rows =
            std::iter::once((true, head)).chain(rows.into_iter().map(|row| (false, row)));
        for (ix, (is_head, mut row)) in all_rows.enumerate() {
            row.resize_with(columns, Inline::default);
            for (column, mut cell) in row.into_iter().enumerate() {
                if is_head {
                    let len = cell.text.len();
                    if len > 0 {
                        cell.spans.insert(
                            0,
                            (
                                0..len,
                                SpanStyle {
                                    bold: true,
                                    ..SpanStyle::default()
                                },
                            ),
                        );
                    }
                }
                let last = !is_head && ix == row_count;
                let element = self.inline(cell, style.font_size);
                let cell = div()
                    .px(px(6.))
                    .py(px(2.))
                    .when(!last, |this| {
                        this.border_b_1().border_color(style.separator)
                    })
                    .map(|this| match alignments.get(column) {
                        Some(Alignment::Center) => this.text_center(),
                        Some(Alignment::Right) => this.text_right(),
                        _ => this,
                    })
                    .child(element);
                cells.push(cell.into_any_element());
            }
        }
        div()
            .grid()
            .grid_cols_max_content(columns as u16)
            .children(cells)
            .into_any_element()
    }
}

/// Merges overlapping highlights (link color over a bold run, say), since
/// `StyledText` wants them sorted and disjoint. Every boundary stays a run
/// boundary: `StyledText` applies a font family override only to runs
/// inside it (code spans and icons).
fn merge_highlights(
    highlights: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut bounds: Vec<usize> = highlights
        .iter()
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut merged: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for pair in bounds.windows(2) {
        let range = pair[0]..pair[1];
        let mut style: Option<HighlightStyle> = None;
        for (r, highlight) in &highlights {
            if r.start <= range.start && range.end <= r.end {
                style = Some(match style {
                    Some(style) => style.highlight(*highlight),
                    None => *highlight,
                });
            }
        }
        if let Some(style) = style {
            merged.push((range, style));
        }
    }
    merged
}

/// An image (`img { max-width: 100% }`): `http(s):` URIs load with the
/// app's HTTP client, base64 `data:` URIs are decoded, and anything else,
/// or an image that fails to load, shows its alt text.
fn image_element(
    url: String,
    alt: String,
    width: Option<f32>,
    height: Option<f32>,
    style: &MarkdownStyle,
) -> AnyElement {
    let placeholder = {
        let alt: SharedString = if alt.is_empty() {
            url.clone().into()
        } else {
            alt.into()
        };
        let color = style.foreground.opacity(0.7);
        move || {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(color)
                .child(codicons::icon(Codicon::FileMedia))
                .child(alt.clone())
                .into_any_element()
        }
    };
    let lower = url.to_ascii_lowercase();
    let source: Option<ImageSource> =
        if lower.starts_with("http://") || lower.starts_with("https://") {
            Some(SharedUri::from(url).into())
        } else if lower.starts_with("data:") {
            data_uri_image(&url).map(Into::into)
        } else {
            None
        };
    let Some(source) = source else {
        return placeholder();
    };
    let fallback = placeholder.clone();
    img(source)
        .max_w_full()
        .when_some(width, |this, width| this.w(px(width)))
        .when_some(height, |this, height| this.h(px(height)))
        .with_fallback(fallback)
        .with_loading(placeholder)
        .into_any_element()
}

/// Renders `markdown` as a column of blocks.
pub fn render_markdown(
    markdown: &MarkdownString,
    style: &MarkdownStyle,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    render_markdown_with_links(markdown, style, theme, None, cx)
}

/// Renders `markdown` as [`render_markdown`] does, giving clicked links to
/// `on_link` first.
pub(crate) fn render_markdown_with_links(
    markdown: &MarkdownString,
    style: &MarkdownStyle,
    theme: &Theme,
    on_link: Option<LinkHandler>,
    cx: &App,
) -> AnyElement {
    let blocks = parse(markdown, codicons::codicons_available(cx));
    let registry = crate::model::language_registry(cx);
    let mut renderer = Renderer {
        style,
        theme,
        registry: &registry,
        trusted: markdown.is_trusted,
        next_id: 0,
        on_link,
    };
    let children = renderer.blocks(blocks, 0);
    // Monaco's `p, .code, ul, ol, h1…h6 { margin: 8px 0 }`, without the
    // outer margins.
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_color(style.foreground)
        .text_size(style.font_size)
        .line_height(relative(style.line_height))
        .children(children)
        .into_any_element()
}

/// Renders documentation: markdown, or plain text as written (Monaco's
/// `white-space: pre-wrap` for string documentation).
pub fn render_documentation(
    documentation: &Documentation,
    style: &MarkdownStyle,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    match documentation {
        Documentation::Markdown(markdown) => render_markdown(markdown, style, theme, cx),
        Documentation::PlainText(text) => div()
            .text_color(style.foreground)
            .text_size(style.font_size)
            .line_height(relative(style.line_height))
            .child(text.clone())
            .into_any_element(),
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(markdown: impl Into<MarkdownString>) -> Vec<Block> {
        parse(&markdown.into(), true)
    }

    fn text(inline: &Inline) -> &str {
        &inline.text
    }

    fn paragraph(block: &Block) -> &Inline {
        match block {
            Block::Paragraph(inline) => inline,
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    fn style_at(inline: &Inline, offset: usize) -> SpanStyle {
        inline
            .spans
            .iter()
            .find(|(range, _)| range.contains(&offset))
            .map(|(_, style)| *style)
            .unwrap_or_default()
    }

    #[test]
    fn openable_links() {
        assert!(is_openable("https://example.com"));
        assert!(is_openable("HTTP://example.com"));
        assert!(is_openable("mailto:someone@example.com"));
        assert!(!is_openable("command:editor.action.format"));
        assert!(!is_openable("javascript:alert(1)"));
        assert!(!is_openable("file:///etc/passwd"));
    }

    /// Monaco `openLinkFromMarkdown`: every clicked link goes through the
    /// opener service, so the host's link openers see `file:`, custom and
    /// untrusted `command:` links too (#353).
    #[gpui::test]
    fn links_go_through_the_link_openers(cx: &mut gpui::TestAppContext) {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = seen.clone();
        cx.update(|cx| {
            let _opener = crate::register_link_opener(cx, move |url, _| {
                sink.borrow_mut().push(url.to_string());
                !url.starts_with("x-unhandled:")
            });
            for url in [
                "file:///a/b.rs#L3",
                "x-custom:thing",
                "command:editor.action.format",
                "https://example.com",
                "internal:x",
                "x-unhandled:y",
            ] {
                open_link(url, false, cx);
            }
        });
        assert_eq!(
            *seen.borrow(),
            [
                "file:///a/b.rs#L3",
                "x-custom:thing",
                "command:editor.action.format",
                "https://example.com",
                "x-unhandled:y",
            ]
        );
    }

    #[test]
    fn command_links() {
        let command = command_link("command:editor.action.format").unwrap();
        assert_eq!(command.id.as_ref(), "editor.action.format");
        assert!(command.arguments.is_empty());
        let command = command_link("command:foo.bar?%5B1%2C%22a%22%5D").unwrap();
        assert_eq!(
            command.arguments,
            vec![serde_json::json!(1), serde_json::json!("a")]
        );
        let command = command_link("command:foo?%7B%22x%22%3A1%7D").unwrap();
        assert_eq!(command.arguments, vec![serde_json::json!({"x": 1})]);
        assert!(command_link("command:").is_none());
        assert!(command_link("https://x").is_none());
    }

    // Ported from Monaco's `markdownRenderer.test.ts` where they apply to
    // the rendered structure rather than to DOM output.

    #[test]
    fn empty_and_plain() {
        assert_eq!(blocks(""), []);
        let result = blocks("hello world");
        assert_eq!(text(paragraph(&result[0])), "hello world");
    }

    #[test]
    fn emphasis_strong_strikethrough() {
        let result = blocks("a **b** *c* ~~d~~");
        let inline = paragraph(&result[0]);
        assert_eq!(inline.text, "a b c d");
        assert!(style_at(inline, 2).bold);
        assert!(style_at(inline, 4).italic);
        assert!(style_at(inline, 6).strikethrough);
        assert_eq!(style_at(inline, 0), SpanStyle::default());
    }

    #[test]
    fn inline_code_is_padded() {
        let result = blocks("use `foo` here");
        let inline = paragraph(&result[0]);
        assert_eq!(inline.text, "use \u{2004}foo\u{2004} here");
        assert!(style_at(inline, "use ".len()).code);
        assert!(!style_at(inline, inline.text.len() - 1).code);
    }

    #[test]
    fn rounded_code_backgrounds_follow_wrapping() {
        let at = |x: f32, y: f32| point(px(x), px(y));
        // On one line: one box from the first boundary to the last.
        assert_eq!(
            code_background_bounds(&[at(10., 0.), at(18., 0.), at(26., 0.)], px(0.), px(19.)),
            [Bounds::from_corners(at(10., 0.), at(26., 19.))]
        );
        // Wrapped after its second character: the rest starts at the
        // line's left.
        assert_eq!(
            code_background_bounds(
                &[
                    at(90., 0.),
                    at(98., 0.),
                    at(106., 0.),
                    at(8., 19.),
                    at(16., 19.)
                ],
                px(2.),
                px(19.)
            ),
            [
                Bounds::from_corners(at(90., 0.), at(106., 19.)),
                Bounds::from_corners(at(2., 19.), at(16., 38.)),
            ]
        );
    }

    #[test]
    fn code_blocks() {
        assert_eq!(
            blocks("```rust\nfn main() {}\n```"),
            [Block::Code {
                language: Some("rust".into()),
                code: "fn main() {}".into()
            }]
        );
        assert_eq!(
            blocks("    indented\n      more"),
            [Block::Code {
                language: None,
                code: "indented\n  more".into()
            }]
        );
    }

    #[test]
    fn tables() {
        let result = blocks("| a | b |\n|:-|-:|\n| 1 | **2** |\n| 3 |");
        let [
            Block::Table {
                alignments,
                head,
                rows,
            },
        ] = result.as_slice()
        else {
            panic!("{result:?}");
        };
        assert_eq!(alignments, &[Alignment::Left, Alignment::Right]);
        assert_eq!(head.iter().map(text).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].iter().map(text).collect::<Vec<_>>(), ["1", "2"]);
        assert!(style_at(&rows[0][1], 0).bold);
        // A short row keeps its cells; the renderer pads it.
        assert_eq!(rows[1].iter().map(text).collect::<Vec<_>>(), ["3", ""]);
    }

    #[test]
    fn block_quotes() {
        let result = blocks("> quoted\n>\n> more\n\nafter");
        let [Block::Quote(inner), Block::Paragraph(after)] = result.as_slice() else {
            panic!("{result:?}");
        };
        assert_eq!(inner.len(), 2);
        assert_eq!(text(paragraph(&inner[0])), "quoted");
        assert_eq!(after.text, "after");
    }

    #[test]
    fn nested_lists() {
        let result = blocks("- a\n  - b\n  - c\n- d\n\n3. x\n4. y");
        let [
            Block::List { start: None, items },
            Block::List {
                start: Some(3),
                items: ordered,
            },
        ] = result.as_slice()
        else {
            panic!("{result:?}");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(text(paragraph(&items[0][0])), "a");
        let Block::List { items: nested, .. } = &items[0][1] else {
            panic!("{:?}", items[0]);
        };
        assert_eq!(nested.len(), 2);
        assert_eq!(ordered.len(), 2);
        // A code block inside an item stays in the item.
        let result = blocks("- item\n\n  ```\n  code\n  ```");
        let [Block::List { items, .. }] = result.as_slice() else {
            panic!("{result:?}");
        };
        assert!(matches!(items[0][1], Block::Code { .. }));
    }

    #[test]
    fn links() {
        let result = blocks("see [the docs](https://example.com) now");
        let inline = paragraph(&result[0]);
        assert_eq!(inline.text, "see the docs now");
        assert_eq!(
            inline.links,
            [(4..12, SharedString::from("https://example.com"))]
        );
    }

    #[test]
    fn images() {
        assert_eq!(
            blocks("![alt text](https://example.com/a.png)"),
            [Block::Image {
                url: "https://example.com/a.png".into(),
                alt: "alt text".into(),
                width: None,
                height: None
            }]
        );
        // Text around an image stays in paragraphs around it.
        let result = blocks("before ![x](y.png) after");
        assert_eq!(result.len(), 3);
        assert_eq!(text(paragraph(&result[0])), "before ");
        assert_eq!(text(paragraph(&result[2])), " after");
    }

    #[test]
    fn data_uri_images_decode() {
        // A 1×1 PNG.
        let uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let image = data_uri_image(uri).unwrap();
        assert_eq!(image.format, ImageFormat::Png);
        assert_eq!(&image.bytes[1..4], b"PNG");
        assert!(data_uri_image("data:text/plain;base64,aGk=").is_none());
        assert!(data_uri_image("data:image/png,raw").is_none());
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
    }

    #[test]
    fn theme_icons() {
        let markdown = MarkdownString::new("$(zap) Fast").with_theme_icons(true);
        let result = parse(&markdown, true);
        let inline = paragraph(&result[0]);
        assert_eq!(inline.text, format!("{} Fast", Codicon::Zap.char()));
        assert_eq!(inline.icons, vec![0..Codicon::Zap.char().len_utf8()]);
        // Without the font, the stand-in.
        let result = parse(&markdown, false);
        assert_eq!(paragraph(&result[0]).text, "• Fast");
        // Without `supportThemeIcons` the syntax stays text.
        assert_eq!(text(paragraph(&blocks("$(zap) Fast")[0])), "$(zap) Fast");
        // Escaped icons stay text (Monaco `markdownEscapeEscapedIcons`).
        let markdown = MarkdownString::new(r"\$(zap) Fast").with_theme_icons(true);
        assert_eq!(text(paragraph(&parse(&markdown, true)[0])), "$(zap) Fast");
        // Not in code.
        let markdown = MarkdownString::new("`$(zap)`").with_theme_icons(true);
        assert!(parse(&markdown, true)[0] != Block::Paragraph(Inline::default()));
        assert!(paragraph(&parse(&markdown, true)[0]).icons.is_empty());
    }

    #[test]
    fn html_is_dropped_without_support_html() {
        let result = blocks("a <b>bold</b> <span style=\"color:#ff0000\">red</span>");
        let inline = paragraph(&result[0]);
        assert_eq!(inline.text, "a bold red");
        assert!(inline.spans.is_empty());
        // Script and style content is dropped either way.
        let result = blocks("x <script>alert(1)</script> y");
        assert_eq!(text(paragraph(&result[0])), "x  y");
    }

    #[test]
    fn html_subset() {
        let markdown = MarkdownString::new(
            "a <b>bold</b><br><span style=\"color:#ff0000;background-color:#00ff00\">red</span> <a href=\"https://x.y\">link</a> <img src=\"https://x.y/i.png\" alt=\"pic\" width=\"20\">",
        )
        .with_html(true);
        let result = parse(&markdown, true);
        let inline = paragraph(&result[0]);
        assert_eq!(inline.text, "a bold\nred link ");
        assert!(style_at(inline, 2).bold);
        let red = style_at(inline, "a bold\n".len());
        assert_eq!(red.color, crate::theme::parse_hex_color("#ff0000"));
        assert_eq!(red.background, crate::theme::parse_hex_color("#00ff00"));
        assert_eq!(inline.links, [(11..15, SharedString::from("https://x.y"))]);
        assert_eq!(
            result[1],
            Block::Image {
                url: "https://x.y/i.png".into(),
                alt: "pic".into(),
                width: Some(20.),
                height: None
            }
        );
    }

    #[test]
    fn html_tokenizer() {
        assert_eq!(
            html_tokens("<a href='x' data-y=1 disabled>t</A><!-- c --><br/>"),
            [
                HtmlToken::Open {
                    name: "a".into(),
                    attributes: vec![
                        ("href".into(), "x".into()),
                        ("data-y".into(), "1".into()),
                        ("disabled".into(), String::new()),
                    ]
                },
                HtmlToken::Text("t".into()),
                HtmlToken::Close("a".into()),
                HtmlToken::Open {
                    name: "br".into(),
                    attributes: vec![]
                },
            ]
        );
        assert_eq!(decode_entities("&lt;a&gt; &amp;amp;"), "<a> &amp;");
    }

    #[test]
    fn rules_and_headings() {
        let result = blocks("# Title\n\ntext\n\n---\n\nmore");
        assert!(matches!(&result[0], Block::Heading(1, inline) if inline.text == "Title"));
        assert!(style_at(paragraph(&result[1]), 0) == SpanStyle::default());
        assert_eq!(result[2], Block::Rule);
    }

    #[test]
    fn merged_highlights_are_disjoint() {
        let bold = HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let red = HighlightStyle {
            color: Some(gpui::red()),
            ..Default::default()
        };
        let merged = merge_highlights(vec![(0..4, bold), (2..6, red)]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0], (0..2, bold));
        assert_eq!(merged[1].0, 2..4);
        assert_eq!(merged[1].1.font_weight, Some(FontWeight::BOLD));
        assert_eq!(merged[1].1.color, Some(gpui::red()));
        assert_eq!(merged[2], (4..6, red));
    }

    #[test]
    fn plain_text_escaping() {
        use crate::providers::escape_markdown_syntax_tokens;
        let source = "*not* _emph_ # 1. <b>\n  indented";
        let result = blocks(escape_markdown_syntax_tokens(source));
        assert_eq!(text(paragraph(&result[0])), "*not* _emph_ # 1. <b>");
        assert_eq!(text(paragraph(&result[1])), "\u{a0}\u{a0}indented");
    }
}
