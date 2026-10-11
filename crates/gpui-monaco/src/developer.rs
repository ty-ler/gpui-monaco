//! Monaco's developer and theme actions: `editor.action.forceRetokenize`
//! (`contrib/tokenization/browser/tokenization.ts`),
//! `editor.action.inspectTokens`
//! (`standalone/browser/inspectTokens/inspectTokens.ts`) and
//! `editor.action.toggleHighContrast`
//! (`standalone/browser/toggleHighContrast/toggleHighContrast.ts`), MIT.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, FontWeight, Global, Hsla, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, px,
};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::theme::{Theme, monaco_token_type, textmate_scopes_for_capture};
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

/// What the inspect tokens widget shows about the token at the cursor
/// (Monaco `InspectTokensWidget._compute`).
#[derive(Clone, Debug, PartialEq)]
pub struct TokenInspection {
    /// The token's text, tabs as `→` and spaces as `·`.
    pub text: String,
    /// Its length in UTF-16 units.
    pub length: usize,
    /// The language there (an injected one inside, say, a Markdown code
    /// fence).
    pub language: SharedString,
    /// Monaco's standard token type: `Other`, `Comment`, `String` or
    /// `RegEx`.
    pub token_type: &'static str,
    /// `italic`, `bold`, `underline` and `strikethrough`, or `---`.
    pub font_style: String,
    /// The foreground as `#rrggbb`.
    pub foreground: String,
    /// The background as `#rrggbb`.
    pub background: String,
    /// The Monarch-style token type with the language as a suffix
    /// (`keyword.rust`), or `""` between highlighted tokens, as Monaco's
    /// `tokenize` reports it.
    pub token: String,
    /// The tree-sitter capture behind it (`keyword.control`), or the
    /// Monarch token (`keyword.rust`), if any.
    pub capture: Option<SharedString>,
    /// The TextMate scopes the capture is matched against in VS Code
    /// themes, most specific first.
    pub scopes: Vec<&'static str>,
}

/// Monaco `renderTokenText`.
fn render_token_text(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\t' => '\u{2192}',
            ' ' => '\u{00B7}',
            c => c,
        })
        .collect()
}

/// Monaco `_fontStyleToString`.
fn font_style_to_string(style: &crate::theme::TokenStyle) -> String {
    let mut result = String::new();
    for (on, name) in [
        (style.italic, "italic "),
        (style.bold, "bold "),
        (style.underline, "underline "),
        (style.strikethrough, "strikethrough "),
    ] {
        if on {
            result.push_str(name);
        }
    }
    if result.is_empty() {
        result.push_str("---");
    }
    result
}

/// Monaco `Color.Format.CSS.formatHex`.
fn format_hex(color: Hsla) -> String {
    let rgba = color.to_rgb();
    let byte = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgba.r),
        byte(rgba.g),
        byte(rgba.b)
    )
}

/// The line's tokens from its highlight spans (sorted, non-overlapping):
/// byte offsets in the line and their captures, covering it from the start
/// as Monaco's `tokenize` does (`None` between highlighted tokens).
fn line_tokens(
    line_len: usize,
    spans: &[(Range<usize>, syntax::HighlightId)],
) -> Vec<(usize, Option<syntax::HighlightId>)> {
    let mut tokens: Vec<(usize, Option<syntax::HighlightId>)> = Vec::new();
    let push = |tokens: &mut Vec<(usize, Option<syntax::HighlightId>)>,
                offset: usize,
                capture: Option<syntax::HighlightId>| {
        if tokens.last().is_some_and(|t| t.0 == offset) {
            tokens.pop();
        }
        if tokens.last().is_none_or(|t| t.1 != capture) {
            tokens.push((offset, capture));
        }
    };
    let mut position = 0;
    for (range, capture) in spans {
        let start = range.start.max(position).min(line_len);
        let end = range.end.min(line_len);
        if end <= start {
            continue;
        }
        if start > position {
            push(&mut tokens, position, None);
        }
        push(&mut tokens, start, Some(*capture));
        position = end;
    }
    if position < line_len || tokens.is_empty() {
        push(&mut tokens, position, None);
    }
    tokens
}

/// Monaco `InspectTokensController`: the widget, if launched.
#[derive(Default)]
pub(crate) struct InspectTokensState {
    widget: Option<WidgetId>,
    inspection: Option<TokenInspection>,
}

struct InspectTokensView {
    inspection: TokenInspection,
    background: Hsla,
    foreground: Hsla,
    border: Hsla,
    border_width: f32,
    font_family: SharedString,
}

impl Render for InspectTokensView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let info = &self.inspection;
        let separator = || div().h(px(1.)).my(px(6.)).bg(self.border);
        let row = |key: &'static str, value: String| {
            div().flex().gap(px(16.)).child(div().child(key)).child(
                div()
                    .ml_auto()
                    .font_family(self.font_family.clone())
                    .child(value),
            )
        };
        let length = format!(
            "{} {}",
            info.length,
            if info.length == 1 { "char" } else { "chars" }
        );
        // Monaco `.tokens-inspect-widget`, in the hover widget's colors.
        div()
            .p(px(10.))
            .bg(self.background)
            .text_color(self.foreground)
            .border(px(self.border_width))
            .border_color(self.border)
            .text_size(px(12.))
            .min_w(px(200.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(16.))
                    .child(
                        div()
                            .font_family(self.font_family.clone())
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(16.))
                            .child(info.text.clone()),
                    )
                    .child(div().ml_auto().text_size(px(10.)).child(length)),
            )
            .child(separator())
            .child(row("language", info.language.to_string()))
            .child(row("token type", info.token_type.to_string()))
            .child(row("font style", info.font_style.clone()))
            .child(row("foreground", info.foreground.clone()))
            .child(row("background", info.background.clone()))
            .child(separator())
            .child(
                div()
                    .font_family(self.font_family.clone())
                    .child(info.token.clone()),
            )
            .children(info.capture.as_ref().map(|capture| {
                div()
                    .font_family(self.font_family.clone())
                    .opacity(0.7)
                    .child(format!("capture: {capture}"))
            }))
            .children((!info.scopes.is_empty()).then(|| {
                div()
                    .font_family(self.font_family.clone())
                    .opacity(0.7)
                    .child(format!("scopes: {}", info.scopes.join(", ")))
            }))
    }
}

/// The theme `editor.action.toggleHighContrast` switched away from (Monaco
/// keeps `_originalThemeName` on its single action).
#[derive(Default)]
struct HighContrastToggle {
    original: Option<Arc<Theme>>,
}

impl Global for HighContrastToggle {}

impl CodeEditor {
    /// Monaco `editor.action.forceRetokenize`: throws away the model's
    /// tokens and tokenizes it again right away, logging how long it took.
    pub fn force_retokenize(&mut self, cx: &mut Context<Self>) {
        let elapsed = self
            .model()
            .update(cx, |model, cx| model.force_retokenize(cx));
        if let Some(elapsed) = elapsed {
            log::info!("tokenization took {}ms", elapsed.as_secs_f64() * 1000.);
        }
    }

    /// What the token at `position` is (Monaco `InspectTokensWidget`'s
    /// data): the token containing the character after `position`, or the
    /// line's last one at its end.
    pub fn inspect_token_at(&self, position: PointUtf16, cx: &gpui::App) -> TokenInspection {
        let model = self.model().read(cx);
        let rope = model.buffer().rope();
        let position = model.validate_position(position);
        let line = model.line_content(position.row);
        let line_start = rope.point_to_offset(text::Point::new(position.row, 0));
        let column = rope.point_utf16_to_offset(position) - line_start;
        let tokenized =
            model.line_len_utf16(position.row) < model.options().max_tokenization_line_length;
        let spans: Vec<(Range<usize>, syntax::HighlightId)> = match model.syntax_snapshot() {
            Some(syntax) if tokenized => {
                let state = if syntax.text.version() == model.version() {
                    syntax.state
                } else {
                    let patch = model.buffer().edits_since(syntax.text.version());
                    syntax.highlighter.update(
                        Some((&syntax.state, syntax.text.rope(), &patch)),
                        &model.snapshot(),
                    )
                };
                syntax
                    .highlighter
                    .highlights(&state, rope, line_start..line_start + line.len())
                    .into_iter()
                    .map(|span| {
                        (
                            span.range.start.saturating_sub(line_start)
                                ..span.range.end.saturating_sub(line_start),
                            span.highlight,
                        )
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        let tokens = line_tokens(line.len(), &spans);
        let index = tokens
            .iter()
            .rposition(|(offset, _)| column >= *offset)
            .unwrap_or(0);
        let (start, capture) = tokens[index];
        let end = tokens.get(index + 1).map_or(line.len(), |t| t.0);
        let token_text = &line[start..end];
        let offset = line_start + start;
        let language: SharedString = model
            .language_at(offset)
            .map(|language| language.id.to_string().into())
            .or_else(|| model.language_id().cloned())
            .unwrap_or_else(|| "plaintext".into());
        let style = capture
            .map(|capture| self.theme.style_for_language(capture, Some(&language)))
            .unwrap_or_default();
        let capture_name = capture.map(|capture| capture.name());
        let token = capture
            .map(|capture| monaco_token_type(capture, &language))
            .unwrap_or_default();
        TokenInspection {
            text: render_token_text(token_text),
            length: token_text.encode_utf16().count(),
            token_type: match model.token_type_at(offset) {
                crate::cursor::StandardTokenType::Other => "Other",
                crate::cursor::StandardTokenType::Comment => "Comment",
                crate::cursor::StandardTokenType::String => "String",
                crate::cursor::StandardTokenType::RegEx => "RegEx",
            },
            font_style: font_style_to_string(&style),
            foreground: format_hex(style.foreground.unwrap_or(self.theme.colors.foreground)),
            background: format_hex(self.theme.colors.background),
            language,
            token,
            scopes: capture_name
                .as_deref()
                .map(|name| textmate_scopes_for_capture(name).to_vec())
                .unwrap_or_default(),
            capture: capture_name.map(|name| name.to_string().into()),
        }
    }

    /// Monaco `editor.action.inspectTokens`: shows the token at the cursor
    /// below it (or above), following the cursor until Escape, a model or
    /// language change.
    pub fn launch_inspect_tokens(&mut self, cx: &mut Context<Self>) {
        if self.inspect_tokens.widget.is_some() {
            return;
        }
        self.compute_inspect_tokens(cx);
    }

    fn compute_inspect_tokens(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.inspect_tokens.widget.take() {
            self.remove_widget(id, cx);
        }
        let position = self.cursors.primary().position;
        let inspection = self.inspect_token_at(position, cx);
        let colors = &self.theme.colors;
        let view = cx.new(|_| InspectTokensView {
            inspection: inspection.clone(),
            background: colors.widget_background,
            foreground: colors.widget_foreground,
            border: colors.widget_border,
            border_width: if self.theme.high_contrast { 2. } else { 1. },
            font_family: self.options.font_family.clone(),
        });
        let id = self.add_content_widget(
            ContentWidget {
                position,
                preference: vec![
                    ContentWidgetPreference::Below,
                    ContentWidgetPreference::Above,
                ],
                view: view.into(),
                allow_editor_overflow: true,
            },
            cx,
        );
        self.widgets.set_layer(id, crate::widgets::layer::WIDGET);
        self.inspect_tokens = InspectTokensState {
            widget: Some(id),
            inspection: Some(inspection),
        };
    }

    /// Follows the cursor (Monaco recomputes on `onDidChangeCursorPosition`).
    pub(crate) fn inspect_tokens_cursor_changed(&mut self, cx: &mut Context<Self>) {
        if self.inspect_tokens.widget.is_some() {
            self.compute_inspect_tokens(cx);
        }
    }

    /// Closes the inspect tokens widget (Monaco
    /// `InspectTokensController.stop`). Returns whether it was open.
    pub fn stop_inspect_tokens(&mut self, cx: &mut Context<Self>) -> bool {
        self.inspect_tokens.inspection = None;
        match self.inspect_tokens.widget.take() {
            Some(id) => {
                self.remove_widget(id, cx);
                true
            }
            None => false,
        }
    }

    /// What the inspect tokens widget shows, while it's open.
    pub fn token_inspection(&self) -> Option<&TokenInspection> {
        self.inspect_tokens.inspection.as_ref()
    }

    /// Monaco `editor.action.toggleHighContrast`: switches every editor to
    /// the high contrast theme of the current theme's kind (`hc-black` or
    /// `hc-light`), or back to the theme it replaced (`vs-dark` or `vs` if
    /// none). The current theme is the one from [`crate::set_theme`], else
    /// this editor's.
    pub fn toggle_high_contrast(&mut self, cx: &mut Context<Self>) {
        let current = crate::theme::global_theme(cx).unwrap_or_else(|| self.theme.clone());
        // `set_theme` updates every editor, this one too.
        cx.defer(move |cx| {
            let toggle = cx.default_global::<HighContrastToggle>();
            let dark = current.kind == crate::theme::ThemeKind::Dark;
            if current.high_contrast {
                let theme = toggle
                    .original
                    .take()
                    .unwrap_or_else(|| Arc::new(if dark { Theme::vs_dark() } else { Theme::vs() }));
                crate::theme::set_theme_arc(cx, theme);
            } else {
                toggle.original = Some(current);
                crate::theme::set_theme(cx, if dark { "hc-black" } else { "hc-light" });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_text_shows_whitespace() {
        assert_eq!(render_token_text("\ta b"), "→a·b");
    }

    #[test]
    fn font_styles() {
        let mut style = crate::theme::TokenStyle::default();
        assert_eq!(font_style_to_string(&style), "---");
        style.bold = true;
        style.italic = true;
        assert_eq!(font_style_to_string(&style), "italic bold ");
    }

    #[test]
    fn hex_colors() {
        assert_eq!(format_hex(gpui::rgb(0x569cd6).into()), "#569cd6");
    }

    #[test]
    fn tokens_cover_the_line() {
        let keyword = syntax::HighlightId::intern("keyword");
        let string = syntax::HighlightId::intern("string");
        // `let s = "x";`
        let tokens = line_tokens(12, &[(0..3, keyword), (8..11, string)]);
        assert_eq!(
            tokens,
            vec![(0, Some(keyword)), (3, None), (8, Some(string)), (11, None)]
        );
        assert_eq!(line_tokens(0, &[]), vec![(0, None)]);
    }
}
