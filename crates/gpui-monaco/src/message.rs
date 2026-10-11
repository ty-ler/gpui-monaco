//! Inline messages next to a position, such as "No definition found"
//! (Monaco's `contrib/message/browser/messageController.ts`, MIT).

use std::ops::Range;

use gpui::{
    AnyElement, AppContext as _, Context, Hsla, IntoElement, ParentElement, Pixels, Render,
    SharedString, Styled, Window, div, px,
};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

#[derive(Default)]
pub(crate) struct InlineMessage {
    widget: Option<WidgetId>,
    /// The row it's shown at.
    row: u32,
    /// Rows the mouse may move within (Monaco: three lines around the
    /// position and the mouse's first row) before it closes.
    bounds: Option<Range<u32>>,
    /// The message's source text.
    text: SharedString,
}

struct MessageView {
    content: AnyElementBuilder,
    background: Hsla,
    border: Hsla,
    foreground: Hsla,
    font_size: Pixels,
}

/// Builds the message's content each render: plain text, or rendered
/// markdown.
type AnyElementBuilder = Box<dyn Fn(&mut gpui::App) -> AnyElement>;

impl Render for MessageView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Monaco `.monaco-editor-overlaymessage .message`.
        div()
            .px(px(4.))
            .py(px(2.))
            .rounded(px(3.))
            .bg(self.background)
            .border_1()
            .border_color(self.border)
            .text_color(self.foreground)
            .text_size(self.font_size)
            .child((self.content)(cx))
    }
}

impl CodeEditor {
    /// Shows `message` above (or below) `position` until the cursor moves,
    /// the editor loses focus or the mouse moves away (Monaco
    /// `MessageController.showMessage`).
    pub fn show_message(
        &mut self,
        message: impl Into<SharedString>,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        let text: SharedString = message.into();
        let content = text.clone();
        self.show_message_content(
            text,
            Box::new(move |_| content.clone().into_any_element()),
            position,
            cx,
        );
    }

    /// [`show_message`](Self::show_message) with markdown (Monaco's
    /// `IMarkdownString` messages). Links open on click.
    pub fn show_markdown_message(
        &mut self,
        markdown: impl Into<crate::providers::MarkdownString>,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        let markdown: crate::providers::MarkdownString = markdown.into();
        let style = self.markdown_style();
        let theme = self.theme.clone();
        self.show_message_content(
            markdown.value.clone(),
            Box::new(move |cx| crate::markdown::render_markdown(&markdown, &style, &theme, cx)),
            position,
            cx,
        );
    }

    fn show_message_content(
        &mut self,
        text: SharedString,
        content: AnyElementBuilder,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        self.close_message(cx);
        // Monaco `showMessage` reads every message to screen readers.
        self.announce(text.clone(), cx);
        let colors = &self.theme.colors;
        let view = cx.new(|_| MessageView {
            content,
            background: colors.message_background,
            border: colors.message_border,
            foreground: colors.widget_foreground,
            font_size: self.options.font_size,
        });
        let id = self.add_content_widget(
            ContentWidget {
                position,
                preference: vec![
                    ContentWidgetPreference::Above,
                    ContentWidgetPreference::Below,
                ],
                view: view.into(),
                allow_editor_overflow: true,
            },
            cx,
        );
        self.widgets.set_layer(id, crate::widgets::layer::MESSAGE);
        self.message = InlineMessage {
            widget: Some(id),
            row: position.row,
            bounds: None,
            text,
        };
    }

    /// Monaco `ReadOnlyMessageController`: an edit was refused, so say why
    /// at the cursor.
    pub(crate) fn read_only_edit_attempt(&mut self, cx: &mut Context<Self>) {
        cx.emit(crate::editor::CodeEditorEvent::ReadOnlyEditAttempt);
        let message = self.options.read_only_message.clone().unwrap_or_else(|| {
            if self.options.single_line {
                "Cannot edit in read-only input".into()
            } else {
                "Cannot edit in read-only editor".into()
            }
        });
        let position = self.cursors.primary().position;
        self.show_markdown_message(message, position, cx);
    }

    /// The visible message's text (markdown source for markdown ones).
    pub fn message_text(&self) -> Option<SharedString> {
        self.message
            .widget
            .is_some()
            .then(|| self.message.text.clone())
    }

    /// Whether the `placeholder` shows: set, and the model is empty.
    pub fn is_placeholder_visible(&self) -> bool {
        self.options.placeholder.is_some() && self.snapshot.is_empty()
    }

    /// Whether an inline message is showing.
    pub fn is_message_visible(&self) -> bool {
        self.message.widget.is_some()
    }

    /// Closes the inline message.
    pub fn close_message(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.message.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Closes the message once the mouse leaves the rows around it.
    pub(crate) fn message_mouse_move(&mut self, row: u32, cx: &mut Context<Self>) {
        if self.message.widget.is_none() {
            return;
        }
        match &self.message.bounds {
            None => {
                let message_row = self.message.row;
                self.message.bounds = Some(message_row.saturating_sub(3)..row + 4);
            }
            Some(bounds) if !bounds.contains(&row) => self.close_message(cx),
            Some(_) => {}
        }
    }
}
