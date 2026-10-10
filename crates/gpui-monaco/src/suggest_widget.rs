//! The suggest widget (Monaco's `suggestWidget`): the completion list with
//! match highlights, kind icons, details and the selected item's docs.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    Context, FontWeight, HighlightStyle, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Render, SharedString, Styled, StyledText, WeakEntity, Window, div, px,
    rgb,
};

use crate::completion::Session;
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_markdown};
use crate::providers::{CompletionItem, CompletionItemKind};
use crate::theme::Theme;

/// Rows visible at once (Monaco `suggest.maxVisibleSuggestions`).
const MAX_VISIBLE: usize = 12;

/// Rows moved by Page Up / Page Down in the list.
pub(crate) fn page_size() -> usize {
    MAX_VISIBLE - 1
}

/// What the widget shows; rebuilt from the session on each change.
pub struct SuggestWidgetState {
    rows: Vec<(CompletionItem, Vec<Range<usize>>)>,
    selected: usize,
}

impl SuggestWidgetState {
    pub(crate) fn from_session(session: &Session) -> Self {
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
        }
    }
}

/// The completion list, shown below or above the cursor while suggestions are
/// open.
pub struct SuggestWidget {
    editor: WeakEntity<CodeEditor>,
    state: SuggestWidgetState,
    theme: Arc<Theme>,
    style: MarkdownStyle,
    line_height: Pixels,
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

/// UTF-16 ranges in `text` to byte ranges.
fn utf16_ranges_to_bytes(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut boundaries = Vec::with_capacity(text.len() + 1);
    let mut units = 0;
    for (ix, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            boundaries.push(ix);
        }
        units += ch.len_utf16();
    }
    boundaries.push(text.len());
    let _ = units;
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
        line_height: Pixels,
    ) -> Self {
        let mut this = Self {
            editor,
            state,
            theme,
            style,
            line_height,
            scroll: 0,
        };
        this.keep_selected_visible();
        this
    }

    pub(crate) fn set_state(&mut self, state: SuggestWidgetState, cx: &mut Context<Self>) {
        self.state = state;
        self.keep_selected_visible();
        cx.notify();
    }

    fn keep_selected_visible(&mut self) {
        let selected = self.state.selected;
        if selected < self.scroll {
            self.scroll = selected;
        } else if selected >= self.scroll + MAX_VISIBLE {
            self.scroll = selected + 1 - MAX_VISIBLE;
        }
        self.scroll = self
            .scroll
            .min(self.state.rows.len().saturating_sub(MAX_VISIBLE));
    }
}

impl Render for SuggestWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors.clone();
        let registry = crate::model::language_registry(cx);
        let match_color: Hsla = rgb(0x2aaaff).into();
        let selected_bg: Hsla = rgb(0x04395e).into();
        let row_height = self.line_height + px(2.);
        let end = (self.scroll + MAX_VISIBLE).min(self.state.rows.len());
        let mut list = div().flex().flex_col();
        for ix in self.scroll..end {
            let (item, highlights) = &self.state.rows[ix];
            let (icon, icon_color) = kind_icon(item.kind);
            let label: SharedString = item.label.clone();
            // Monaco strikes deprecated items through, dimmed, without match
            // highlights.
            let deprecated = item
                .tags
                .contains(&crate::providers::CompletionItemTag::Deprecated);
            let label_highlights: Vec<(Range<usize>, HighlightStyle)> = if deprecated {
                vec![(
                    0..label.len(),
                    HighlightStyle {
                        strikethrough: Some(gpui::StrikethroughStyle {
                            thickness: px(1.),
                            color: None,
                        }),
                        fade_out: Some(0.34),
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
                                color: Some(match_color),
                                font_weight: Some(FontWeight::BOLD),
                                ..Default::default()
                            },
                        )
                    })
                    .collect()
            };
            let is_selected = ix == self.state.selected;
            let editor = self.editor.clone();
            list = list.child(
                div()
                    .id(("suggestion", ix))
                    .h(row_height)
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(is_selected, |this| this.bg(selected_bg))
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
                    .child(div().w(px(16.)).text_color(icon_color).child(icon))
                    .child(
                        div()
                            .flex_1()
                            .child(StyledText::new(label).with_highlights(label_highlights)),
                    )
                    .when_some(
                        item.detail.clone().filter(|_| is_selected),
                        |this, detail| {
                            this.child(
                                div()
                                    .text_color(colors.widget_foreground.opacity(0.6))
                                    .child(detail),
                            )
                        },
                    ),
            );
        }
        let docs = self
            .state
            .rows
            .get(self.state.selected)
            .and_then(|(item, _)| item.documentation.clone());
        div()
            .flex()
            .flex_col()
            .w(px(430.))
            .bg(colors.widget_background)
            .border_1()
            .border_color(colors.widget_border)
            .text_color(colors.widget_foreground)
            .text_size(self.style.font_size)
            .font_family(self.style.code_font.clone())
            .child(list)
            .when_some(docs, |this, docs| {
                this.child(
                    div()
                        .border_t_1()
                        .border_color(colors.widget_border)
                        .p(px(6.))
                        .max_h(px(160.))
                        .child(render_markdown(
                            &docs.0,
                            &self.style,
                            &self.theme,
                            &registry,
                        )),
                )
            })
    }
}

use gpui::prelude::FluentBuilder as _;
