//! Inline messages next to a position, such as "No definition found"
//! (Monaco's `contrib/message/browser/messageController.ts` and
//! `messageController.css`, MIT).

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement, PathBuilder, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Task, WeakEntity, Window, canvas, div, point, px,
};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::widgets::{ContentWidget, ContentWidgetPreference, OverflowRoomCell, WidgetId};

/// `fadeIn 150ms ease-out` and `fadeOut 100ms ease-out`.
const FADE_IN: Duration = Duration::from_millis(150);
const FADE_OUT: Duration = Duration::from_millis(100);
/// The `.anchor` triangle: 8px borders, so 16px wide and 8px tall, 2px
/// from the widget's left, which sits 6px left of the position.
const ANCHOR_SIZE: f32 = 8.;
const ANCHOR_LEFT: f32 = 2.;
const MARGIN_LEFT: f32 = -6.;

#[derive(Default)]
pub(crate) struct InlineMessage {
    widget: Option<WidgetId>,
    /// The view, to tell it the mouse left when it closes.
    view: Option<Entity<MessageView>>,
    /// The row it's shown at.
    row: u32,
    /// Rows the mouse may move within (Monaco: three lines around the
    /// position and the mouse's first row) before it closes.
    bounds: Option<Range<u32>>,
    /// The message's source text.
    text: SharedString,
    /// The mouse is over the message (Monaco `_mouseOverMessage`): it stays
    /// open when the editor loses focus.
    pub(crate) mouse_over: bool,
    /// A closed message fading out, removed when the fade ends.
    fading: Option<(WidgetId, Task<()>)>,
}

pub(crate) struct MessageView {
    editor: WeakEntity<CodeEditor>,
    content: AnyElementBuilder,
    background: Hsla,
    border: Hsla,
    border_width: Pixels,
    foreground: Hsla,
    font_size: Pixels,
    /// Where the editor publishes the room around the line.
    room: OverflowRoomCell,
    /// Shown below the line (Monaco's `.below` class, from `afterRender`):
    /// the arrow points up from the top.
    below: Rc<Cell<bool>>,
    /// Fading out (Monaco `MessageWidget.fadeOut`).
    closing: bool,
}

/// Builds the message's content each render: plain text, or rendered
/// markdown.
type AnyElementBuilder = Box<dyn Fn(&mut gpui::App) -> AnyElement>;

impl Render for MessageView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let below = self.below.get();
        let gap = px(ANCHOR_SIZE);
        let border = self.border;
        // Monaco `.monaco-editor-overlaymessage .message`: `2px 4px`
        // padding, 3px corners, `editorHoverWidget.*` with an
        // `inputValidation.infoBorder` border (2px in high contrast).
        let message = div()
            .px(px(4.))
            .py(px(2.))
            .rounded(px(3.))
            .bg(self.background)
            .border(self.border_width)
            .border_color(border)
            .text_color(self.foreground)
            .text_size(self.font_size)
            .child((self.content)(cx));
        // The `.anchor` triangle in the gap, pointing at the line.
        let anchor = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let left = bounds.left();
                let (base, tip) = if below {
                    (bounds.bottom(), bounds.top())
                } else {
                    (bounds.top(), bounds.bottom())
                };
                let mut path = PathBuilder::fill();
                path.move_to(point(left, base));
                path.line_to(point(left + gap * 2., base));
                path.line_to(point(left + gap, tip));
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, border);
                }
            },
        )
        .ml(px(ANCHOR_LEFT))
        .w(gap * 2.)
        .h(gap);
        // Which side it was placed on: the editor places it above or below
        // the line it's anchored to (Monaco's `afterRender(position)`).
        let room = self.room.clone();
        let placed = self.below.clone();
        let placement = canvas(
            move |bounds, window, _| {
                let Some(room) = room.get() else {
                    return;
                };
                // The line's top in the window.
                let line_top = room.above + crate::widgets::OVERFLOW_VERTICAL_PADDING;
                let below = bounds.top() >= line_top;
                if below != placed.get() {
                    placed.set(below);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .w_full()
        .h(px(0.));
        let editor = self.editor.clone();
        let root = div()
            .id("message")
            .relative()
            .ml(px(MARGIN_LEFT))
            .flex()
            .flex_col()
            .on_hover(move |hovered, _, cx| {
                let hovered = *hovered;
                editor
                    .update(cx, |editor, _| editor.message.mouse_over = hovered)
                    .ok();
            })
            .child(placement);
        // The arrow in the 8px gap between the box and the line.
        let root = if below {
            root.child(anchor).child(message)
        } else {
            root.child(message).child(anchor)
        };
        if self.closing {
            root.with_animation(
                "message-fade-out",
                Animation::new(FADE_OUT).with_easing(gpui::ease_out_quint()),
                |root, delta| root.opacity(1. - delta),
            )
            .into_any_element()
        } else {
            root.with_animation(
                "message-fade-in",
                Animation::new(FADE_IN).with_easing(gpui::ease_out_quint()),
                |root, delta| root.opacity(delta),
            )
            .into_any_element()
        }
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
        // A message still fading out goes at once.
        if let Some((id, _)) = self.message.fading.take() {
            self.remove_widget(id, cx);
        }
        // Monaco `showMessage` reads every message to screen readers.
        self.announce(text.clone(), cx);
        // Monaco `MessageWidget`: `revealLinesInCenterIfOutsideViewport`.
        self.reveal_lines_with_scroll_type(
            position.row,
            position.row,
            crate::view::reveal::RevealType::CenterIfOutsideViewport,
            crate::view::reveal::ScrollType::Smooth,
            cx,
        );
        let colors = &self.theme.colors;
        let editor = cx.entity().downgrade();
        let view = cx.new(|_| MessageView {
            editor,
            content,
            background: colors.hover_background,
            border: colors.message_border,
            border_width: if self.theme.high_contrast {
                px(2.)
            } else {
                px(1.)
            },
            foreground: colors.hover_foreground,
            font_size: self.options.font_size,
            room: OverflowRoomCell::default(),
            below: Rc::new(Cell::new(false)),
            closing: false,
        });
        let id = self.add_content_widget(
            ContentWidget {
                allow_editor_overflow: true,
                ..ContentWidget::new(
                    position,
                    vec![
                        ContentWidgetPreference::Above,
                        ContentWidgetPreference::Below,
                    ],
                    view.clone().into(),
                )
            },
            cx,
        );
        self.widgets.set_layer(id, crate::widgets::layer::MESSAGE);
        let room = self.widgets.room(id);
        view.update(cx, |view, _| view.room = room);
        self.message = InlineMessage {
            widget: Some(id),
            view: Some(view),
            row: position.row,
            bounds: None,
            text,
            mouse_over: false,
            fading: None,
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

    /// Closes the inline message, which fades out over 100ms (Monaco
    /// `closeMessage` and `MessageWidget.fadeOut`).
    pub fn close_message(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.message.widget.take() else {
            return;
        };
        self.message.mouse_over = false;
        if let Some((old, _)) = self.message.fading.take() {
            self.remove_widget(old, cx);
        }
        let Some(view) = self.message.view.take() else {
            self.remove_widget(id, cx);
            return;
        };
        view.update(cx, |view, cx| {
            view.closing = true;
            cx.notify();
        });
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FADE_OUT).await;
            this.update(cx, |this, cx| {
                if this.message.fading.as_ref().is_some_and(|(f, _)| *f == id) {
                    this.message.fading = None;
                    this.remove_widget(id, cx);
                }
            })
            .ok();
        });
        self.message.fading = Some((id, task));
    }

    /// Monaco closes the message when the editor text loses focus, unless
    /// the mouse is over it.
    pub(crate) fn message_blurred(&mut self, cx: &mut Context<Self>) {
        if !self.message.mouse_over {
            self.close_message(cx);
        }
    }

    /// Whether the mouse is over the message, for tests.
    #[doc(hidden)]
    pub fn set_message_mouse_over_for_tests(&mut self, over: bool) {
        self.message.mouse_over = over;
    }

    /// Whether the message shows below its line (with its arrow on top),
    /// for tests.
    #[doc(hidden)]
    pub fn message_below_for_tests(&self, cx: &gpui::App) -> Option<bool> {
        Some(self.message.view.as_ref()?.read(cx).below.get())
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
