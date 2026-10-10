//! Inline messages next to a position, such as "No definition found"
//! (Monaco's `contrib/message/browser/messageController.ts`, MIT).

use std::ops::Range;

use gpui::{
    AppContext as _, Context, Hsla, IntoElement, ParentElement, Pixels, Render, SharedString,
    Styled, Window, div, px,
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
}

struct MessageView {
    text: SharedString,
    background: Hsla,
    border: Hsla,
    foreground: Hsla,
    font_size: Pixels,
}

impl Render for MessageView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(3.))
            .bg(self.background)
            .border_1()
            .border_color(self.border)
            .text_color(self.foreground)
            .text_size(self.font_size)
            .child(self.text.clone())
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
        self.close_message(cx);
        let colors = &self.theme.colors;
        let view = cx.new(|_| MessageView {
            text: message.into(),
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
            },
            cx,
        );
        self.message = InlineMessage {
            widget: Some(id),
            row: position.row,
            bounds: None,
        };
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
