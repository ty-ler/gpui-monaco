//! The suggest widget (Monaco's `suggestWidget`, `suggestWidgetRenderer`,
//! `suggestWidgetDetails` and `suggestWidgetStatus`): the completion list
//! with match highlights, kind icons and label details, the "Loading..."
//! and "No suggestions." messages, the details panel (toggled with
//! Ctrl+Space, beside the list when it fits, else below it), the status bar
//! and resizing.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, DragMoveEvent, FontWeight, Global, HighlightStyle, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    Size, StatefulInteractiveElement, Styled, StyledText, WeakEntity, Window, div, px, rgb, size,
};

use crate::completion::Session;
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_markdown};
use crate::options::{SuggestInsertMode, SuggestOptions};
use crate::providers::{CompletionItem, CompletionItemKind, CompletionItemTag};
use crate::theme::Theme;

/// Rows shown at the default height (hardcoded in Monaco).
const DEFAULT_ROWS: f32 = 12.;
/// Monaco's default widget width.
const DEFAULT_WIDTH: f32 = 430.;
/// Monaco's minimum widget width.
const MIN_WIDTH: f32 = 220.;
/// The details panel's default width.
const DETAILS_WIDTH: f32 = 330.;
/// Monaco's `horizontalPadding` around the widgets.
const HORIZONTAL_PADDING: f32 = 14.;

/// The message shown while an explicit request is computing.
pub const LOADING_MESSAGE: &str = "Loading...";
/// The message shown when an explicit request found nothing.
pub const NO_SUGGESTIONS_MESSAGE: &str = "No suggestions.";

/// What the widget shows; rebuilt from the session on each change.
pub struct SuggestWidgetState {
    rows: Vec<(CompletionItem, Vec<Range<usize>>)>,
    selected: usize,
    /// "Loading..." or "No suggestions." instead of rows.
    message: Option<&'static str>,
    /// Monaco's `Frozen` state: nothing matches what was typed, so the
    /// last matches stay, without bold highlights.
    frozen: bool,
}

impl SuggestWidgetState {
    pub(crate) fn from_session(session: &Session) -> Self {
        if session.model.filtered().is_empty() {
            return Self::message(NO_SUGGESTIONS_MESSAGE);
        }
        let rows = session
            .model
            .filtered()
            .iter()
            .map(|scored| {
                (
                    session.model.items()[scored.index].clone(),
                    scored.highlights.clone(),
                )
            })
            .collect();
        Self {
            rows,
            selected: session.selected,
            message: None,
            frozen: session.frozen,
        }
    }

    pub(crate) fn message(message: &'static str) -> Self {
        Self {
            rows: Vec::new(),
            selected: 0,
            message: Some(message),
            frozen: false,
        }
    }
}

/// Monaco's stored suggest UI state: whether details are expanded
/// (`expandSuggestionDocs`) and the size the user resized the list to
/// (`suggestWidget.size`). Kept for the app's lifetime.
#[derive(Default)]
pub(crate) struct SuggestUi {
    pub(crate) details_expanded: bool,
    pub(crate) size: Option<Size<Pixels>>,
}

impl Global for SuggestUi {}

/// Whether the details panel is expanded.
pub(crate) fn details_expanded(cx: &App) -> bool {
    cx.try_global::<SuggestUi>()
        .is_some_and(|ui| ui.details_expanded)
}

/// Monaco `canExpandCompletionItem`: docs, or a detail other than the
/// label.
pub(crate) fn can_expand(item: &CompletionItem) -> bool {
    item.documentation.is_some()
        || item
            .detail
            .as_ref()
            .is_some_and(|detail| *detail != item.label)
}

/// Sizes that depend on the editor.
pub(crate) struct SuggestWidgetMetrics {
    /// Row height (`suggest.line_height`, or the editor's).
    pub(crate) item_height: Pixels,
    /// `suggest.font_size`, or the editor's.
    pub(crate) font_size: Pixels,
    /// Room right of the widget's left edge, for placing details beside it.
    pub(crate) available_width: Option<Pixels>,
}

/// A resize sash being dragged.
#[derive(Clone, Copy)]
enum Sash {
    East,
    South,
}

/// The completion list, shown below or above the cursor while suggestions are
/// open.
pub struct SuggestWidget {
    editor: WeakEntity<CodeEditor>,
    state: SuggestWidgetState,
    theme: Arc<Theme>,
    style: MarkdownStyle,
    options: SuggestOptions,
    metrics: SuggestWidgetMetrics,
    /// First visible row.
    scroll: usize,
}

/// A short glyph and color per kind, standing in for Monaco's codicons.
fn kind_icon(kind: CompletionItemKind) -> (&'static str, Hsla) {
    use CompletionItemKind::*;
    let purple: Hsla = rgb(0xb180d7).into();
    let blue: Hsla = rgb(0x75beff).into();
    let orange: Hsla = rgb(0xee9d28).into();
    let gray: Hsla = rgb(0xc5c5c5).into();
    match kind {
        Method | Function | Constructor => ("ƒ", purple),
        Field | Variable | Property | Value | Reference | TypeParameter => ("𝑥", blue),
        Class | Struct | Interface | Enum | Module => ("◆", orange),
        EnumMember | Constant => ("π", blue),
        Keyword | Operator => ("⌘", gray),
        Snippet => ("{}", gray),
        File | Folder => ("▤", gray),
        Event | Unit | Color | Text => ("ab", gray),
    }
}

/// Monaco's `ColorExtractor` pattern: hex, `rgb[a]()` and `hsl[a]()`.
const COLOR_PATTERN: &str = r"(#([\da-fA-F]{3}){1,2}|(rgb|hsl)a\(\s*(\d{1,3}%?\s*,\s*){3}(1|0?\.\d+)\)|(rgb|hsl)\(\s*\d{1,3}%?(\s*,\s*\d{1,3}%?){2}\s*\))";

/// The color a `Color` item shows as a swatch (Monaco `ColorExtractor`):
/// its label or detail if either is exactly a color, else a color at the
/// start or end of its docs.
pub(crate) fn completion_item_color(item: &CompletionItem) -> Option<Hsla> {
    use std::sync::OnceLock;
    static STRICT: OnceLock<regex::Regex> = OnceLock::new();
    static RELAXED: OnceLock<regex::Regex> = OnceLock::new();
    let strict =
        STRICT.get_or_init(|| regex::Regex::new(&format!("(?i)^{COLOR_PATTERN}$")).expect("valid"));
    let relaxed = RELAXED.get_or_init(|| regex::Regex::new(COLOR_PATTERN).expect("valid"));
    if strict.is_match(&item.label) {
        return parse_css_color(&item.label);
    }
    if let Some(detail) = item.detail.as_ref().filter(|d| strict.is_match(d)) {
        return parse_css_color(detail);
    }
    let docs = &item.documentation.as_ref()?.0;
    let found = relaxed.find(docs)?;
    (found.start() == 0 || found.end() == docs.len())
        .then(|| parse_css_color(found.as_str()))
        .flatten()
}

/// Parses `#rgb`, `#rrggbb`, `rgb[a](…)` and `hsl[a](…)`.
fn parse_css_color(text: &str) -> Option<Hsla> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        let hex: String = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect()
        } else {
            hex.to_string()
        };
        let value = u32::from_str_radix(&hex, 16).ok()?;
        return Some(rgb(value).into());
    }
    let open = text.find('(')?;
    let function = text[..open].to_ascii_lowercase();
    let args: Vec<&str> = text[open + 1..text.len() - 1]
        .split(',')
        .map(str::trim)
        .collect();
    let number = |s: &str, scale: f32| -> Option<f32> {
        match s.strip_suffix('%') {
            Some(percent) => Some(percent.parse::<f32>().ok()? / 100.),
            None => Some(s.parse::<f32>().ok()? / scale),
        }
    };
    let alpha = args.get(3).map_or(Some(1.), |a| a.parse::<f32>().ok())?;
    if function.starts_with("rgb") {
        let [r, g, b] = [args[0], args[1], args[2]].map(|c| number(c, 255.));
        Some(
            gpui::Rgba {
                r: r?,
                g: g?,
                b: b?,
                a: alpha,
            }
            .into(),
        )
    } else {
        let h = args[0].trim_end_matches('%').parse::<f32>().ok()? / 360.;
        Some(gpui::hsla(
            h,
            number(args[1], 100.)?,
            number(args[2], 100.)?,
            alpha,
        ))
    }
}

/// UTF-16 ranges in `text` to byte ranges.
fn utf16_ranges_to_bytes(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut boundaries = Vec::with_capacity(text.len() + 1);
    for (ix, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            boundaries.push(ix);
        }
    }
    boundaries.push(text.len());
    ranges
        .iter()
        .filter_map(|r| {
            let start = *boundaries.get(r.start)?;
            let end = *boundaries.get(r.end.min(boundaries.len() - 1))?;
            (start < end).then_some(start..end)
        })
        .collect()
}

impl SuggestWidget {
    pub(crate) fn new(
        editor: WeakEntity<CodeEditor>,
        state: SuggestWidgetState,
        theme: Arc<Theme>,
        style: MarkdownStyle,
        options: SuggestOptions,
        metrics: SuggestWidgetMetrics,
        cx: &App,
    ) -> Self {
        let mut this = Self {
            editor,
            state,
            theme,
            style,
            options,
            metrics,
            scroll: 0,
        };
        this.keep_selected_visible(cx);
        this
    }

    pub(crate) fn set_state(
        &mut self,
        state: SuggestWidgetState,
        metrics: SuggestWidgetMetrics,
        cx: &mut Context<Self>,
    ) {
        self.state = state;
        self.metrics = metrics;
        self.keep_selected_visible(cx);
        cx.notify();
    }

    pub(crate) fn message(&self) -> Option<&'static str> {
        self.state.message
    }

    fn status_bar_height(&self) -> Pixels {
        if self.options.show_status_bar && self.state.message.is_none() {
            self.metrics.item_height
        } else {
            px(0.)
        }
    }

    /// The list's size: the resized one, or Monaco's default of 12 rows,
    /// shrunk to the rows there are.
    fn list_size(&self, cx: &App) -> Size<Pixels> {
        let item_height = self.metrics.item_height;
        let status = self.status_bar_height();
        let stored = cx.try_global::<SuggestUi>().and_then(|ui| ui.size);
        let default = size(px(DEFAULT_WIDTH), status + item_height * DEFAULT_ROWS);
        let wanted = stored.unwrap_or(default);
        let full = status + item_height * self.state.rows.len().max(1) as f32;
        let min = status + item_height;
        size(
            wanted.width.max(px(MIN_WIDTH)),
            wanted.height.min(full).max(min),
        )
    }

    /// Rows that fit in the list.
    pub(crate) fn visible_rows(&self, cx: &App) -> usize {
        let height = self.list_size(cx).height - self.status_bar_height();
        ((height / self.metrics.item_height).floor() as usize).max(1)
    }

    fn keep_selected_visible(&mut self, cx: &App) {
        let visible = self.visible_rows(cx);
        let selected = self.state.selected;
        if selected < self.scroll {
            self.scroll = selected;
        } else if selected >= self.scroll + visible {
            self.scroll = selected + 1 - visible;
        }
        self.scroll = self
            .scroll
            .min(self.state.rows.len().saturating_sub(visible));
    }

    fn render_row(&self, ix: usize, show_details: bool) -> impl IntoElement + use<> {
        let colors = &self.theme.colors;
        let (item, highlights) = &self.state.rows[ix];
        let is_selected = ix == self.state.selected;
        let item_height = self.metrics.item_height;
        let label: SharedString = item.label.clone();
        // Monaco strikes deprecated items through, dimmed, without match
        // highlights.
        let deprecated = item.tags.contains(&CompletionItemTag::Deprecated);
        let highlight_color = if is_selected {
            colors.suggest_focus_highlight_foreground
        } else {
            colors.suggest_highlight_foreground
        };
        let label_highlights: Vec<(Range<usize>, HighlightStyle)> = if deprecated {
            vec![(
                0..label.len(),
                HighlightStyle {
                    strikethrough: Some(gpui::StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..Default::default()
                },
            )]
        } else {
            utf16_ranges_to_bytes(&label, highlights)
                .into_iter()
                .map(|range| {
                    (
                        range,
                        HighlightStyle {
                            color: Some(highlight_color),
                            font_weight: (!self.state.frozen).then_some(FontWeight::BOLD),
                            ..Default::default()
                        },
                    )
                })
                .collect()
        };
        // The right side: a label description on every row, or the detail
        // on the focused row while the details panel is closed.
        let right_text = match &item.label_description {
            Some(description) => Some(description.clone()),
            None => item
                .detail
                .clone()
                .filter(|_| is_selected && !show_details)
                .map(|d| SharedString::from(d.replace(['\n', '\r'], " "))),
        }
        .filter(|_| self.options.show_inline_details);
        let read_more =
            is_selected && !show_details && !self.options.show_status_bar && can_expand(item);
        let icon = self.options.show_icons.then(|| {
            let mut icon = div()
                .w(item_height)
                .flex_none()
                .flex()
                .justify_center()
                .items_center();
            if item.kind == CompletionItemKind::Color
                && let Some(color) = completion_item_color(item)
            {
                icon = icon.child(
                    div()
                        .size(px(9.))
                        .border_1()
                        .border_color(gpui::black())
                        .bg(color),
                );
            } else {
                let (glyph, glyph_color) = kind_icon(item.kind);
                icon = icon.text_color(glyph_color).child(glyph);
            }
            icon
        });
        let editor = self.editor.clone();
        let editor_for_more = self.editor.clone();
        div()
            .id(("suggestion", ix))
            .group("suggest-row")
            .h(item_height)
            .pl(px(2.))
            .pr(px(10.))
            .flex()
            .items_center()
            .overflow_hidden()
            .when(is_selected, |this| {
                this.bg(colors.suggest_selected_background)
                    .text_color(colors.suggest_selected_foreground)
            })
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |editor, cx| {
                        editor.select_suggestion(ix, cx);
                        editor.accept_suggestion(cx);
                    });
                }
            })
            .children(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .when(deprecated, |this| this.opacity(0.66))
                    .child(StyledText::new(label).with_highlights(label_highlights))
                    .when_some(item.label_detail.clone(), |this, detail| {
                        this.child(div().opacity(0.6).child(detail))
                    }),
            )
            .when_some(right_text, |this, text| {
                this.child(
                    div()
                        .flex_shrink(4.)
                        .max_w(gpui::relative(0.7))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .ml(self.metrics.font_size * 0.85 * 1.1)
                        .text_size(self.metrics.font_size * 0.85)
                        .when(!is_selected, |this| this.opacity(0.7))
                        .child(text),
                )
            })
            .when(read_more, |this| {
                this.child(
                    div()
                        .id(("read-more", ix))
                        .ml(px(4.))
                        .w(px(18.))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .opacity(0.)
                        .group_hover("suggest-row", |style| style.opacity(1.))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor_for_more.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.toggle_suggestion_details(cx);
                                });
                            }
                        })
                        .child("›"),
                )
            })
    }

    /// Whether the details panel shows: expanded, and the focused item has
    /// something to show.
    pub(crate) fn details_shown(&self, cx: &App) -> bool {
        self.state.message.is_none()
            && details_expanded(cx)
            && self
                .state
                .rows
                .get(self.state.selected)
                .is_some_and(|(item, _)| can_expand(item))
    }

    fn render_status_bar(&self, show_details: bool) -> impl IntoElement + use<> {
        let colors = &self.theme.colors;
        let mac = cfg!(target_os = "macos");
        let (primary, alternative) = match self.options.insert_mode {
            SuggestInsertMode::Insert => ("Insert", "Replace"),
            SuggestInsertMode::Replace => ("Replace", "Insert"),
        };
        let focused = self
            .state
            .rows
            .get(self.state.selected)
            .map(|(item, _)| item);
        // Monaco shows the other mode only when the item's insert and replace
        // ranges differ.
        let has_alternative = focused.is_some_and(|item| {
            item.replace_range
                .as_ref()
                .is_some_and(|replace| item.range.as_ref() != Some(replace))
        });
        let mut left = format!("{primary} ({})", if mac { "⏎" } else { "Enter" });
        if has_alternative {
            left.push_str(&format!(
                ", {alternative} ({})",
                if mac { "⇧⏎" } else { "Shift+Enter" }
            ));
        }
        let right = focused.filter(|item| can_expand(item)).map(|_| {
            let key = if mac { "⌃Space" } else { "Ctrl+Space" };
            if show_details {
                format!("Show Less ({key})")
            } else {
                format!("Show More ({key})")
            }
        });
        div()
            .h(self.metrics.item_height)
            .px(px(4.))
            .flex()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(colors.suggest_border)
            .text_size(self.metrics.font_size * 0.8)
            .text_color(colors.suggest_foreground.opacity(0.5))
            .child(left)
            .children(right)
    }

    fn render_details(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let colors = &self.theme.colors;
        let (item, _) = self.state.rows.get(self.state.selected)?;
        let registry = crate::model::language_registry(cx);
        let detail = item
            .detail
            .clone()
            .filter(|detail| *detail != item.label)
            .map(|detail| -> SharedString {
                // Monaco caps the detail at 100000 characters.
                if detail.len() > 100_000 {
                    let mut end = 100_000;
                    while !detail.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}…", &detail[..end]).into()
                } else {
                    detail
                }
            });
        let docs = item.documentation.clone();
        let editor = self.editor.clone();
        Some(
            div()
                .id("suggest-details")
                .w_full()
                .relative()
                .bg(colors.suggest_background)
                .border_1()
                .border_color(colors.suggest_border)
                .text_color(colors.suggest_foreground)
                .overflow_y_scroll()
                .child(
                    div()
                        .id("suggest-details-close")
                        .absolute()
                        .top(px(2.))
                        .right(px(2.))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.toggle_suggestion_details(cx);
                                });
                            }
                        })
                        .child("×"),
                )
                .when_some(detail, |this, detail| {
                    this.child(
                        div()
                            .opacity(0.7)
                            .pt(px(4.))
                            .pb(px(if docs.is_some() { 12. } else { 4. }))
                            .pl(px(5.))
                            .mr(px(24.))
                            .child(detail),
                    )
                })
                .when_some(docs, |this, docs| {
                    this.child(div().px(px(5.)).py(px(4.)).child(render_markdown(
                        &docs.0,
                        &self.style,
                        &self.theme,
                        &registry,
                    )))
                }),
        )
    }

    /// Resizes the list as a sash is dragged (Monaco persists the size).
    fn resize(&mut self, sash: Sash, event: &DragMoveEvent<Sash>, cx: &mut Context<Self>) {
        let bounds = event.bounds;
        let position = event.event.position;
        let current = self.list_size(cx);
        let size = match sash {
            Sash::East => size(
                (position.x - bounds.left()).max(px(MIN_WIDTH)),
                current.height,
            ),
            Sash::South => size(
                current.width,
                (position.y - bounds.top())
                    .max(self.metrics.item_height + self.status_bar_height()),
            ),
        };
        cx.default_global::<SuggestUi>().size = Some(size);
        self.keep_selected_visible(cx);
        cx.notify();
    }
}

impl Render for SuggestWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors.clone();
        let item_height = self.metrics.item_height;
        let base = div()
            .bg(colors.suggest_background)
            .border_1()
            .border_color(colors.suggest_border)
            .text_color(colors.suggest_foreground)
            .text_size(self.metrics.font_size)
            .line_height(item_height)
            .font_family(self.style.code_font.clone());
        if let Some(message) = self.state.message {
            // Monaco: half the default width, one row.
            return div().child(
                base.w(px(DEFAULT_WIDTH / 2.))
                    .h(item_height + px(2.))
                    .pl(px(22.))
                    .flex()
                    .items_center()
                    .child(message),
            );
        }
        let list_size = self.list_size(cx);
        let visible = self.visible_rows(cx);
        let show_details = self.details_shown(cx);
        let end = (self.scroll + visible).min(self.state.rows.len());
        let rows: Vec<_> = (self.scroll..end)
            .map(|ix| self.render_row(ix, show_details).into_any_element())
            .collect();
        let sash = |id: &'static str, sash: Sash| {
            div()
                .id(id)
                .absolute()
                .on_drag(sash, |_, _, _, cx| cx.new(|_| gpui::Empty))
        };
        let list = base
            .id("suggest-list")
            .relative()
            .flex_none()
            .w(list_size.width)
            .h(list_size.height + px(2.))
            .flex()
            .flex_col()
            .child(div().flex_1().overflow_hidden().children(rows))
            .when(self.options.show_status_bar, |this| {
                this.child(self.render_status_bar(show_details))
            })
            .child(
                sash("suggest-sash-east", Sash::East)
                    .top_0()
                    .bottom_0()
                    .right(px(-2.))
                    .w(px(4.))
                    .cursor_col_resize(),
            )
            .child(
                sash("suggest-sash-south", Sash::South)
                    .left_0()
                    .right_0()
                    .bottom(px(-2.))
                    .h(px(4.))
                    .cursor_row_resize(),
            )
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<Sash>, _, cx| {
                let sash = *event.drag(cx);
                this.resize(sash, event, cx);
            }));
        // Monaco places the details east of the list, else west, else
        // below; here east when it fits, else below.
        let details = show_details.then(|| self.render_details(cx)).flatten();
        let beside = self.metrics.available_width.is_none_or(|available| {
            list_size.width + px(DETAILS_WIDTH + HORIZONTAL_PADDING) <= available
        });
        div()
            .flex()
            .when(!beside, |this| this.flex_col())
            .items_start()
            .child(list)
            .when_some(details, |this, details| {
                this.child(if beside {
                    div()
                        .w(px(DETAILS_WIDTH))
                        .max_h(list_size.height + px(2.))
                        .ml(px(-1.))
                        .flex()
                        .child(details)
                } else {
                    div()
                        .w(list_size.width)
                        .max_h(list_size.height + px(2.))
                        .mt(px(-1.))
                        .flex()
                        .child(details)
                })
            })
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_css_colors() {
        let hsla = |c: Option<Hsla>| c.map(gpui::Rgba::from);
        let rgba = |r: f32, g: f32, b: f32, a: f32| Some(gpui::Rgba { r, g, b, a });
        assert_eq!(hsla(parse_css_color("#f00")), rgba(1., 0., 0., 1.));
        assert_eq!(hsla(parse_css_color("#0000ff")), rgba(0., 0., 1., 1.));
        assert_eq!(
            hsla(parse_css_color("rgba(255, 0, 0, 0.5)")),
            rgba(1., 0., 0., 0.5)
        );
        assert_eq!(
            hsla(parse_css_color("rgb(0%, 100%, 0%)")),
            rgba(0., 1., 0., 1.)
        );
        let hsl = parse_css_color("hsl(120, 100%, 50%)").unwrap();
        assert!((hsl.h - 1. / 3.).abs() < 1e-4 && hsl.s == 1. && hsl.l == 0.5);
    }

    #[test]
    fn extracts_completion_item_colors() {
        let item = |label: &str, detail: Option<&str>, docs: Option<&str>| CompletionItem {
            detail: detail.map(|d| d.to_string().into()),
            documentation: docs.map(Into::into),
            ..CompletionItem::new(label.to_string(), CompletionItemKind::Color)
        };
        assert!(completion_item_color(&item("#FFF", None, None)).is_some());
        assert!(completion_item_color(&item("red", Some("#ff0000"), None)).is_some());
        // Docs: only at the start or end.
        assert!(completion_item_color(&item("red", None, Some("The color #ff0000"))).is_some());
        assert!(completion_item_color(&item("red", None, Some("a #ff0000 b"))).is_none());
        // Not exactly a color.
        assert!(completion_item_color(&item("#ff0000 x", None, None)).is_none());
    }
}
