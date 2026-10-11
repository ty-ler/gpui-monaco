//! Highlighting code without an editor (Monaco `colorizer.ts`,
//! `monaco.editor.colorize`/`colorizeElement`, and `tokenize` in
//! `standaloneEditor.ts`, MIT): for docs, hovers or any host UI.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, FontStyle, FontWeight, HighlightStyle, IntoElement, ParentElement, SharedString,
    Styled, StyledText, div,
};
use syntax::{
    HighlightState, Highlighter, Language, LanguageRegistry, MonarchHighlightState,
    MonarchHighlighter, MonarchToken, SyntaxSnapshot,
};
use text::Buffer;

use crate::theme::{Theme, monaco_token_type};

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

/// The registered language `language_id` (by id, else by name).
fn find_language(language_id: &str, registry: &LanguageRegistry) -> Option<Arc<Language>> {
    registry
        .language_for_id(language_id)
        .or_else(|| registry.language_for_name(language_id))
}

/// A shared copy of `registry`, for a [`syntax::HighlighterFactory`].
fn shared_registry(registry: &LanguageRegistry) -> Arc<LanguageRegistry> {
    let shared = LanguageRegistry::new();
    for language in registry.languages() {
        shared.register((*language).clone());
    }
    Arc::new(shared)
}

/// The Monarch highlighter of `language_id`, if its highlighter is one
/// (see [`set_monarch_tokens_provider`](crate::set_monarch_tokens_provider)).
fn monarch_highlighter(
    language_id: &str,
    registry: &LanguageRegistry,
) -> Option<Arc<dyn Highlighter>> {
    let language = find_language(language_id, registry)?;
    let factory = language.highlighter.as_ref()?;
    let highlighter = factory(&language, &shared_registry(registry));
    MonarchHighlighter::downcast(&*highlighter)?;
    Some(highlighter)
}

/// Monaco's tokens of each line of `text` with a Monarch grammar: byte
/// offsets in the line become UTF-16 ones.
fn monarch_tokens(lines: &[&str], tokens: Vec<Vec<MonarchToken>>) -> Vec<Vec<Token>> {
    lines
        .iter()
        .zip(tokens)
        .map(|(line, tokens)| {
            tokens
                .into_iter()
                .map(|token| Token {
                    offset: line[..token.offset.min(line.len())].encode_utf16().count() as u32,
                    token_type: token.token_type.into(),
                    language: token.language.to_string().into(),
                })
                .collect()
        })
        .collect()
}

/// The highlights of `text` (byte ranges and their captures or Monarch
/// tokens) in `language_id`, if it's registered with a grammar or a custom
/// highlighter.
pub(crate) fn capture_spans(
    text: &str,
    language_id: &str,
    registry: &LanguageRegistry,
) -> Option<Vec<(Range<usize>, syntax::HighlightId)>> {
    let language =
        find_language(language_id, registry).filter(|language| language.has_highlighter())?;
    let buffer = Buffer::new(text);
    if let Some(factory) = &language.highlighter {
        let highlighter = factory(&language, &shared_registry(registry));
        let state = highlighter.update(None, &buffer.snapshot());
        return Some(
            highlighter
                .highlights(&state, buffer.rope(), 0..buffer.len())
                .into_iter()
                .map(|span| (span.range, span.highlight))
                .collect(),
        );
    }
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
/// the start. Unknown languages give one `""` token per line. A Monarch
/// language (see
/// [`set_monarch_tokens_provider`](crate::set_monarch_tokens_provider))
/// gives exactly its tokenizer's tokens, as in Monaco: empty lines may have
/// none, and embedded languages' tokens name them.
pub fn tokenize(text: &str, language_id: &str, registry: &LanguageRegistry) -> Vec<Vec<Token>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if let Some(highlighter) = monarch_highlighter(language_id, registry)
        && let Some(monarch) = MonarchHighlighter::downcast(&*highlighter)
    {
        return monarch_tokens(&split_lines(text), monarch.tokenize_text(text));
    }
    let spans = capture_spans(text, language_id, registry).unwrap_or_default();
    let language: SharedString = language_id.to_string().into();
    let mut result = Vec::new();
    let mut line_start = 0;
    let mut spans = spans.into_iter().peekable();
    for line in split_lines(text) {
        let line_end = line_start + line.len();
        result.push(line_tokens(line, line_start, &mut spans, &|_| {
            language.clone()
        }));
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

/// The tokens of `line`, which starts at byte `line_start` of the text
/// `spans` (sorted highlight captures) are in, covering it from the start.
/// Spans ending in the line are consumed; one continuing past it is left
/// for the next line. `language_at` names the language at a byte offset.
fn line_tokens<I: Iterator<Item = (Range<usize>, syntax::HighlightId)>>(
    line: &str,
    line_start: usize,
    spans: &mut std::iter::Peekable<I>,
    language_at: &dyn Fn(usize) -> SharedString,
) -> Vec<Token> {
    let line_end = line_start + line.len();
    let utf16 = |byte: usize| line[..byte - line_start].encode_utf16().count() as u32;
    let mut tokens: Vec<Token> = Vec::new();
    // A token at the same offset replaces the last one, and equal
    // neighbours merge.
    let push = |tokens: &mut Vec<Token>, at: usize, token_type: SharedString, language| {
        let offset = utf16(at);
        if tokens.last().is_some_and(|t| t.offset == offset) {
            tokens.pop();
        }
        if tokens.last().is_none_or(|t| t.token_type != token_type) {
            tokens.push(Token {
                offset,
                token_type,
                language,
            });
        }
    };
    let mut position = line_start;
    while let Some((range, capture)) = spans.peek() {
        if range.start >= line_end {
            break;
        }
        let (range, capture) = (range.clone(), *capture);
        if range.end <= position {
            spans.next();
            continue;
        }
        let start = range.start.max(position);
        if start > position {
            push(&mut tokens, position, "".into(), language_at(position));
        }
        let language = language_at(start);
        push(
            &mut tokens,
            start,
            monaco_token_type(capture, &language).into(),
            language,
        );
        position = range.end.min(line_end);
        if range.end > line_end {
            // Continues on the next line.
            break;
        }
        spans.next();
    }
    if position < line_end || tokens.is_empty() {
        push(&mut tokens, position, "".into(), language_at(position));
    }
    tokens
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

/// Monaco `colorizeModelLine`: row `row` (0-based) of `model` highlighted
/// with its own tokens, tabs expanded to `tab_size` (Monaco's default is
/// 4). Like Monaco's `forceTokenization`, it doesn't wait for a background
/// parse: a model that has parsed before is brought up to date right away,
/// and one that never has (e.g. not shown yet) is parsed by its language
/// id. Lines past `max_tokenization_line_length` aren't highlighted.
/// `None` for rows past the end, where Monaco throws.
pub fn colorize_model_line(
    model: &crate::TextModel,
    row: u32,
    tab_size: u32,
    theme: &Theme,
    cx: &gpui::App,
) -> Option<ColorizedLine> {
    if row >= model.line_count() {
        return None;
    }
    let line = model.line_content(row);
    let line_start = model
        .buffer()
        .rope()
        .point_to_offset(text::Point::new(row, 0));
    let line_end = line_start + line.len();
    let captures = model_line_captures(model, row, cx);
    let (expanded, map) = expand_tabs(&line, tab_size.max(1) as usize);
    let highlights = captures
        .into_iter()
        .filter(|(range, _)| range.start < line_end && range.end > line_start)
        .map(|(range, capture)| {
            let start = range.start.max(line_start) - line_start;
            let end = range.end.min(line_end) - line_start;
            (map[start]..map[end], token_style(theme, capture))
        })
        .filter(|(range, _)| !range.is_empty())
        .collect();
    Some(ColorizedLine {
        text: expanded.into(),
        highlights,
    })
}

/// The highlight captures over row `row` of `model`, as Monaco's
/// `forceTokenization` would have them: a model that has parsed before is
/// brought up to date right away, and one that never has is parsed by its
/// language id. None past `max_tokenization_line_length`.
fn model_line_captures(
    model: &crate::TextModel,
    row: u32,
    cx: &gpui::App,
) -> Vec<(Range<usize>, syntax::HighlightId)> {
    let rope = model.buffer().rope();
    let line_start = rope.point_to_offset(text::Point::new(row, 0));
    let line_end = line_start + rope.line_len(row) as usize;
    let tokenized = model.line_len_utf16(row) < model.options().max_tokenization_line_length;
    if !tokenized {
        Vec::new()
    } else if let Some((highlighter, state)) = model_state(model) {
        highlighter
            .highlights(&state, rope, line_start..line_end)
            .into_iter()
            .map(|span| (span.range, span.highlight))
            .collect()
    } else if let Some(language_id) = model.language_id() {
        let registry = crate::language_registry(cx);
        capture_spans(&model.buffer().rope().to_string(), language_id, &registry)
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// The model's highlighter and its state brought up to date with the text,
/// if it has parsed before.
fn model_state(model: &crate::TextModel) -> Option<(Arc<dyn Highlighter>, HighlightState)> {
    let syntax = model.syntax_snapshot()?;
    let state = if syntax.text.version() == model.version() {
        syntax.state
    } else {
        let patch = model.buffer().edits_since(syntax.text.version());
        syntax.highlighter.update(
            Some((&syntax.state, syntax.text.rope(), &patch)),
            &model.snapshot(),
        )
    };
    Some((syntax.highlighter, state))
}

/// Monaco's model tokens of row `row` from a Monarch grammar, if the model
/// is highlighted by one. Empty lines get one `""` token.
fn model_monarch_line_tokens(
    model: &crate::TextModel,
    row: u32,
    line: &str,
    cx: &gpui::App,
) -> Option<Vec<Token>> {
    let tokens = match model_state(model) {
        Some((highlighter, state)) => {
            MonarchHighlighter::downcast(&*highlighter)?;
            state
                .downcast_ref::<MonarchHighlightState>()?
                .line_tokens(row as usize)?
        }
        None => {
            let registry = crate::language_registry(cx);
            let highlighter = monarch_highlighter(model.language_id()?, &registry)?;
            let monarch = MonarchHighlighter::downcast(&*highlighter)?;
            let text = model.buffer().rope().to_string();
            monarch.tokenize_text(&text).into_iter().nth(row as usize)?
        }
    };
    let mut tokens = monarch_tokens(&[line], vec![tokens]).pop()?;
    if tokens.is_empty() {
        tokens.push(Token {
            offset: 0,
            token_type: "".into(),
            language: model.language_id().cloned().unwrap_or_default(),
        });
    }
    Some(tokens)
}

impl crate::TextModel {
    /// The tokens of row `row` (Monaco `tokenization.getLineTokens`), like
    /// [`tokenize`]'s: each token's start in UTF-16 units, its Monaco token
    /// type with the language as a suffix (`keyword.rust`, or `""` between
    /// highlighted tokens), and its language, the injected one inside
    /// e.g. a Markdown code fence. A token ends where the next starts.
    ///
    /// Like Monaco's `getLineTokens` after `forceTokenization`, a model
    /// that parsed before is brought up to date right away, and one that
    /// never has (not shown yet) is parsed by its language id. Lines past
    /// `max_tokenization_line_length` and models without a grammar give one
    /// `""` token. A Monarch language gives its tokenizer's tokens (one `""`
    /// token for an empty line). `None` for rows past the end.
    pub fn line_tokens(&self, row: u32, cx: &gpui::App) -> Option<Vec<Token>> {
        if row >= self.line_count() {
            return None;
        }
        let line = self.line_content(row);
        if self.line_len_utf16(row) < self.options().max_tokenization_line_length
            && let Some(tokens) = model_monarch_line_tokens(self, row, &line, cx)
        {
            return Some(tokens);
        }
        let line_start = self
            .buffer()
            .rope()
            .point_to_offset(text::Point::new(row, 0));
        let mut captures = model_line_captures(self, row, cx);
        captures.sort_by_key(|(range, _)| range.start);
        let model_language: SharedString = self
            .language()
            .map(|language| language.id.to_string().into())
            .or_else(|| self.language_id().cloned())
            .unwrap_or_default();
        let language_at = |offset: usize| -> SharedString {
            match self.language_at(offset) {
                Some(language) => language.id.to_string().into(),
                None => model_language.clone(),
            }
        };
        Some(line_tokens(
            &line,
            line_start,
            &mut captures.into_iter().peekable(),
            &language_at,
        ))
    }
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
