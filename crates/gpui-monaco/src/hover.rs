//! Hover (Monaco's `contrib/hover`): after the mouse rests over text, shows
//! decoration hover messages (e.g. diagnostics) and hover provider results in
//! a content widget.

use std::ops::Range;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use text::PointUtf16;

use crate::cursor;
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_markdown};
use crate::providers::{MarkdownString, hover_providers};
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

/// Monaco `hover.delay`.
pub const HOVER_DELAY: Duration = Duration::from_millis(300);

#[derive(Default)]
pub(crate) struct HoverState {
    task: Option<Task<()>>,
    widget: Option<(Entity<HoverWidget>, WidgetId)>,
    /// The range the visible (or pending) hover is about.
    range: Option<Range<PointUtf16>>,
}

/// The hover card.
pub struct HoverWidget {
    contents: Vec<MarkdownString>,
    style: MarkdownStyle,
    theme: std::sync::Arc<crate::Theme>,
}

impl Render for HoverWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;
        let registry = crate::model::language_registry(cx);
        let mut sections = Vec::new();
        for (ix, content) in self.contents.iter().enumerate() {
            if ix > 0 {
                sections.push(div().h(px(1.)).bg(colors.widget_border).into_any_element());
            }
            sections.push(render_markdown(
                &content.0,
                &self.style,
                &self.theme,
                &registry,
            ));
        }
        div()
            .id("hover")
            .max_w(px(500.))
            .max_h(px(250.))
            .overflow_y_scroll()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .bg(colors.widget_background)
            .border_1()
            .border_color(colors.widget_border)
            .text_size(self.style.font_size)
            .children(sections)
    }
}

impl CodeEditor {
    pub(crate) fn markdown_style(&self) -> MarkdownStyle {
        let colors = &self.theme.colors;
        MarkdownStyle {
            foreground: colors.widget_foreground,
            link: colors.input_option_active_border,
            code_background: colors.input_background,
            code_font: self.options.font_family.clone(),
            font_size: self.options.font_size,
        }
    }

    pub fn is_hover_visible(&self) -> bool {
        self.hover.widget.is_some()
    }

    /// Hides the hover and cancels a pending one.
    pub fn hide_hover(&mut self, cx: &mut Context<Self>) {
        self.hover.task = None;
        self.hover.range = None;
        if let Some((_, id)) = self.hover.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Shows the hover for the cursor position right away (Monaco
    /// `editor.action.showHover`, Cmd/Ctrl+K Cmd/Ctrl+I).
    pub fn show_hover_at_cursor(&mut self, cx: &mut Context<Self>) {
        let position = self.cursors.primary().position;
        self.hide_hover(cx);
        let config = self.cursor_config(cx);
        let range =
            cursor::word_at(&config, self.snapshot.rope(), position).unwrap_or(position..position);
        let (mut contents, tasks) = self.hover_requests(position, cx);
        self.hover.range = Some(range.clone());
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            let mut hover_range = None;
            for task in tasks {
                if let Ok(Some(hover)) = task.await {
                    contents.extend(hover.contents);
                    hover_range = hover_range.or(hover.range);
                }
            }
            this.update(cx, |this, cx| {
                if !contents.is_empty() {
                    this.show_hover(hover_range.unwrap_or(range), contents, cx);
                }
            })
            .ok();
        }));
    }

    /// Called on mouse moves over the text with the position under the
    /// mouse (`None` outside text).
    pub(crate) fn hover_mouse_move(
        &mut self,
        position: Option<PointUtf16>,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = position else {
            if self.hover.widget.is_some() || self.hover.task.is_some() {
                self.hide_hover(cx);
            }
            return;
        };
        if let Some(range) = &self.hover.range
            && range.start <= position
            && position <= range.end
        {
            return;
        }
        self.hide_hover(cx);
        let config = self.cursor_config(cx);
        let range =
            cursor::word_at(&config, self.snapshot.rope(), position).unwrap_or(position..position);
        self.hover.range = Some(range.clone());
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            let Ok(requests) = this.update(cx, |this, cx| this.hover_requests(position, cx)) else {
                return;
            };
            let (mut contents, tasks) = requests;
            let mut hover_range = None;
            for task in tasks {
                if let Ok(Some(hover)) = task.await {
                    contents.extend(hover.contents);
                    hover_range = hover_range.or(hover.range);
                }
            }
            this.update(cx, |this, cx| {
                if contents.is_empty() {
                    return;
                }
                let range = hover_range.unwrap_or(range);
                this.show_hover(range, contents, cx);
            })
            .ok();
        }));
    }

    /// Decoration hover messages at `position` plus provider requests.
    #[allow(clippy::type_complexity)]
    fn hover_requests(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> (
        Vec<MarkdownString>,
        Vec<Task<anyhow::Result<Option<crate::providers::Hover>>>>,
    ) {
        let offset = self.snapshot.point_utf16_to_offset(position);
        let editor_id = Some(cx.entity_id());
        let messages: Vec<MarkdownString> = self.model().update(cx, |model, _| {
            model
                .decorations_in_range(offset..offset, editor_id)
                .into_iter()
                .filter(|d| d.range.start <= offset && offset <= d.range.end)
                .filter_map(|d| d.options.hover_message.clone())
                .map(MarkdownString)
                .collect()
        });
        let model = self.model().read(cx);
        let context = model.provider_context();
        let language = model.language_id().map(|l| l.to_string());
        let providers = hover_providers(cx, language.as_deref());
        let tasks = providers
            .into_iter()
            .map(|provider| provider.provide_hover(&context, position, cx))
            .collect();
        (messages, tasks)
    }

    fn show_hover(
        &mut self,
        range: Range<PointUtf16>,
        contents: Vec<MarkdownString>,
        cx: &mut Context<Self>,
    ) {
        let style = self.markdown_style();
        let theme = self.theme.clone();
        let widget = cx.new(|_| HoverWidget {
            contents,
            style,
            theme,
        });
        let id = self.add_content_widget(
            ContentWidget {
                position: range.start,
                preference: vec![
                    ContentWidgetPreference::Above,
                    ContentWidgetPreference::Below,
                ],
                view: widget.clone().into(),
            },
            cx,
        );
        self.hover.range = Some(range);
        self.hover.widget = Some((widget, id));
    }
}
