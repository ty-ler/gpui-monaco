//! Highlighting code without an editor (Monaco `colorizer.ts`,
//! `monaco.editor.colorize`/`colorizeElement`, and `tokenize` in
//! `standaloneEditor.ts`, MIT): for docs, hovers or any host UI.

use std::ops::Range;

use gpui::{
    AnyElement, FontStyle, FontWeight, HighlightStyle, IntoElement, ParentElement, SharedString,
    Styled, StyledText, div,
};
use syntax::{LanguageRegistry, SyntaxSnapshot};
use text::Buffer;

use crate::theme::{Theme, monaco_token_for_capture};

/// A token of a line (Monaco `Token`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Where it starts in its line, in UTF-16 code units.
    pub offset: u32,
    /// Its Monaco token type with the language as a suffix (`keyword.rust`),
    /// or `""` between highlighted tokens.
    pub token_type: SharedString,
    /// The language id.
    pub language: SharedString,
}

/// Monaco `strings.splitLines`: `\r\n`, `\r` or `\n`.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                lines.push(&text[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = i;
            }
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    lines.push(&text[start..]);
    lines
}

/// The highlights of `text` (byte ranges and their captures) in
/// `language_id`, if it's registered with a grammar.
fn capture_spans(
    text: &str,
    language_id: &str,
    registry: &LanguageRegistry,
) -> Option<Vec<(Range<usize>, syntax::HighlightId)>> {
    let language = registry
        .language_for_id(language_id)
        .or_else(|| registry.language_for_name(language_id))
        .filter(|l| l.grammar.is_some())?;
    let buffer = Buffer::new(text);
    let syntax = SyntaxSnapshot::parse(language, &buffer.snapshot(), registry);
    Some(
        syntax
            .highlights(buffer.rope(), 0..buffer.len())
            .into_iter()
            .map(|span| (span.range, span.highlight))
            .collect(),
    )
}

/// Monaco `monaco.editor.tokenize`: each line's tokens, covering it from
/// the start. Unknown languages give one `""` token per line.
pub fn tokenize(text: &str, language_id: &str, registry: &LanguageRegistry) -> Vec<Vec<Token>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let spans = capture_spans(text, language_id, registry).unwrap_or_default();
    let language: SharedString = language_id.to_string().into();
    let mut result = Vec::new();
    let mut line_start = 0;
    let mut spans = spans.into_iter().peekable();
    for line in split_lines(text) {
        let line_end = line_start + line.len();
        let utf16 = |byte: usize| line[..byte - line_start].encode_utf16().count() as u32;
        let mut tokens: Vec<Token> = Vec::new();
        // A token at the same offset replaces the last one, and equal
        // neighbours merge.
        let push = |tokens: &mut Vec<Token>, offset: u32, token_type: SharedString| {
            if tokens.last().is_some_and(|t| t.offset == offset) {
                tokens.pop();
            }
            if tokens.last().is_none_or(|t| t.token_type != token_type) {
                tokens.push(Token {
                    offset,
                    token_type,
                    language: language.clone(),
                });
            }
        };
        let mut position = line_start;
        while let Some((range, capture)) = spans.peek() {
            if range.start >= line_end {
                break;
            }
            let (range, capture) = (range.clone(), capture.name());
            if range.end <= position {
                spans.next();
                continue;
            }
            let start = range.start.max(position);
            if start > position {
                push(&mut tokens, utf16(position), "".into());
            }
            let token = monaco_token_for_capture(&capture)
                .map(str::to_string)
                .unwrap_or_else(|| capture.to_string());
            push(
                &mut tokens,
                utf16(start),
                format!("{token}.{language_id}").into(),
            );
            position = range.end.min(line_end);
            if range.end > line_end {
                // Continues on the next line.
                break;
            }
            spans.next();
        }
        if position < line_end || tokens.is_empty() {
            push(&mut tokens, utf16(position), "".into());
        }
        result.push(tokens);
        // Past the line break.
        line_start = line_end
            + match text.as_bytes().get(line_end) {
                Some(b'\r') if text.as_bytes().get(line_end + 1) == Some(&b'\n') => 2,
                Some(_) => 1,
                None => 0,
            };
    }
    result
}

/// Monaco `IColorizerOptions`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorizeOptions {
    /// Tabs become spaces up to the next multiple (default 4).
    pub tab_size: u32,
}

impl Default for ColorizeOptions {
    fn default() -> Self {
        Self { tab_size: 4 }
    }
}

/// A colorized line: its text (tabs expanded) and highlights in it.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorizedLine {
    /// The text, with tabs as spaces.
    pub text: SharedString,
    /// Byte ranges of `text` and their style.
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
}

fn token_style(theme: &Theme, capture: syntax::HighlightId) -> HighlightStyle {
    let token = theme.style_for(capture);
    HighlightStyle {
        color: token.foreground,
        font_weight: token.bold.then_some(FontWeight::BOLD),
        font_style: token.italic.then_some(FontStyle::Italic),
        ..Default::default()
    }
}

/// The highlights of `text` in `language_id` with `theme`, as byte ranges
/// (for a [`StyledText`]). Empty for unknown languages.
pub fn colorize_highlights(
    text: &str,
    language_id: &str,
    theme: &Theme,
    registry: &LanguageRegistry,
) -> Vec<(Range<usize>, HighlightStyle)> {
    capture_spans(text, language_id, registry)
        .unwrap_or_default()
        .into_iter()
        .map(|(range, capture)| (range, token_style(theme, capture)))
        .collect()
}

/// `line` with tabs as spaces up to the next multiple of `tab_size`, and
/// each byte offset of `line` (and its end) mapped into the result.
pub(crate) fn expand_tabs(line: &str, tab_size: usize) -> (String, Vec<usize>) {
    let mut expanded = String::with_capacity(line.len());
    let mut map = Vec::with_capacity(line.len() + 1);
    let mut column = 0;
    for (i, ch) in line.char_indices() {
        while map.len() <= i {
            map.push(expanded.len());
        }
        if ch == '\t' {
            let spaces = tab_size - column % tab_size;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(ch);
            column += 1;
        }
    }
    while map.len() <= line.len() {
        map.push(expanded.len());
    }
    (expanded, map)
}

/// Monaco `colorize`: `text` highlighted line by line, with tabs expanded
/// (Monaco renders HTML; this returns the lines for any element).
pub fn colorize(
    text: &str,
    language_id: &str,
    theme: &Theme,
    registry: &LanguageRegistry,
    options: ColorizeOptions,
) -> Vec<ColorizedLine> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let highlights = colorize_highlights(text, language_id, theme, registry);
    let tab_size = options.tab_size.max(1) as usize;
    let mut result = Vec::new();
    let mut line_start = 0;
    for line in split_lines(text) {
        let line_end = line_start + line.len();
        let (expanded, map) = expand_tabs(line, tab_size);
        let line_highlights = highlights
            .iter()
            .filter(|(range, _)| range.start < line_end && range.end > line_start)
            .map(|(range, style)| {
                let start = range.start.max(line_start) - line_start;
                let end = range.end.min(line_end) - line_start;
                (map[start]..map[end], *style)
            })
            .filter(|(range, _)| !range.is_empty())
            .collect();
        result.push(ColorizedLine {
            text: expanded.into(),
            highlights: line_highlights,
        });
        line_start = line_end
            + match text.as_bytes().get(line_end) {
                Some(b'\r') if text.as_bytes().get(line_end + 1) == Some(&b'\n') => 2,
                Some(_) => 1,
                None => 0,
            };
    }
    result
}

/// Monaco `colorizeElement`: `text` highlighted, one line per row, in the
/// theme's editor font color. The host chooses the font and size.
pub fn colorize_element(
    text: &str,
    language_id: &str,
    theme: &Theme,
    registry: &LanguageRegistry,
    options: ColorizeOptions,
) -> AnyElement {
    let lines = colorize(text, language_id, theme, registry, options);
    div()
        .flex()
        .flex_col()
        .text_color(theme.colors.foreground)
        .children(lines.into_iter().map(|line| {
            // An empty line still takes a row.
            let text = if line.text.is_empty() {
                SharedString::from(" ")
            } else {
                line.text
            };
            div()
                .whitespace_nowrap()
                .child(StyledText::new(text).with_highlights(line.highlights))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_like_monaco() {
        assert_eq!(split_lines("a\r\nb\rc\nd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_lines(""), vec![""]);
        assert_eq!(split_lines("a\n"), vec!["a", ""]);
    }

    #[test]
    fn unknown_languages_give_plain_tokens() {
        let registry = LanguageRegistry::default();
        let tokens = tokenize("one\n\ttwo", "nope", &registry);
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            vec![Token {
                offset: 0,
                token_type: "".into(),
                language: "nope".into()
            }]
        );
        let lines = colorize(
            "\tx",
            "nope",
            &Theme::vs_dark(),
            &registry,
            ColorizeOptions::default(),
        );
        assert_eq!(lines[0].text.as_ref(), "    x");
        assert!(lines[0].highlights.is_empty());
    }
}
