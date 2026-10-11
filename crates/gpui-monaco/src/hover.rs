//! Hover (Monaco's `contrib/hover`): after the mouse rests over text, shows
//! decoration hover messages (e.g. diagnostics) and hover provider results in
//! a content widget.

use std::ops::Range;
use std::time::Duration;

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, Modifiers, ParentElement, Pixels, Render, ScrollHandle, Size,
    StatefulInteractiveElement, Styled, Task, Window, canvas, div, point, px, size,
};
use text::PointUtf16;

use crate::actions::{
    GoToBottomHover, GoToTopHover, HideHover, PageDownHover, PageUpHover, ScrollDownHover,
    ScrollLeftHover, ScrollRightHover, ScrollUpHover,
};
use crate::codicons::{Codicon, icon as codicon};
use crate::cursor;
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_markdown};
use crate::options::{HoverEnabled, MultiCursorModifier};
use crate::providers::{
    Hover, HoverContext, HoverProvider, HoverVerbosityRequest, MarkdownString, hover_providers,
};
use crate::widgets::{ContentWidgetPreference, WidgetId};

/// Monaco's default `hover.delay`.
pub const HOVER_DELAY: Duration = Duration::from_millis(300);

/// The key context of a focused hover (Monaco `hoverFocused`).
pub const HOVER_KEY_CONTEXT: &str = "HoverWidget";

#[derive(Default)]
pub(crate) struct HoverState {
    task: Option<Task<()>>,
    widget: Option<(Entity<HoverWidget>, WidgetId)>,
    /// The range the visible (or pending) hover is about.
    range: Option<Range<PointUtf16>>,
    /// The gutter row a glyph margin hover is about.
    gutter_row: Option<u32>,
    /// Shown from the keyboard (Monaco `isVisibleFromKeyboard`).
    from_keyboard: bool,
    /// Monaco's `hidingDelay` grace period before reacting to the mouse.
    hiding: Option<Task<()>>,
    /// The last mouse target and modifiers, for `onKeyboardModifier`.
    last_mouse: Option<(HoverTarget, Modifiers)>,
    /// The size the card was last resized to (Monaco
    /// `ContentHoverWidget._lastDimensions`): later cards may grow to it.
    last_size: Rc<Cell<Option<Size<Pixels>>>>,
    /// The inlay hint label part it's about (Monaco `InlayHintsHover`).
    inlay: Option<crate::inlay_hints::InlayHintPart>,
}

/// A provider's hover, kept to ask it for more or less detail (Monaco
/// `HoverSource`).
#[derive(Clone)]
pub(crate) struct HoverPartSource {
    provider: Arc<dyn HoverProvider>,
    hover: Hover,
    position: PointUtf16,
}

impl HoverPartSource {
    /// Monaco `HoverSource.supportsVerbosityAction`.
    fn supports(&self, increase: bool) -> bool {
        if increase {
            self.hover.can_increase_verbosity
        } else {
            self.hover.can_decrease_verbosity
        }
    }
}

/// A markdown hover part (Monaco `MarkdownHover`): a decoration's hover
/// message, a provider's hover, or a long line warning.
#[derive(Clone)]
pub(crate) struct MarkdownHoverPart {
    contents: Vec<MarkdownString>,
    source: Option<HoverPartSource>,
    /// A long line warning, with Monaco's "Don't Show Again" link.
    long_line_warning: bool,
}

impl MarkdownHoverPart {
    fn new(contents: Vec<MarkdownString>) -> Self {
        Self {
            contents,
            source: None,
            long_line_warning: false,
        }
    }
}

/// Monaco's message when rendering stops on a long line.
const STOPPED_RENDERING: &str = "Rendering paused for long line for performance reasons. This can be configured via `stop_rendering_line_after`.";
/// Monaco's message when tokenization skips a long line.
const TOO_MANY_CHARACTERS: &str = "Tokenization is skipped for long lines for performance reasons. This can be configured via `max_tokenization_line_length`.";

/// Monaco's minimum hover width when resizing.
const MIN_HOVER_WIDTH: f32 = 150.;

/// What the mouse is over, for hovers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HoverTarget {
    /// Text at a position.
    Text(PointUtf16),
    /// The glyph margin or line numbers of a row (Monaco's glyph hover).
    Gutter(u32),
    /// Nothing that hovers.
    None,
}

/// What a hover action link runs.
type HoverAction = Box<dyn Fn(&mut CodeEditor, &mut Context<CodeEditor>)>;

/// The hover card.
pub struct HoverWidget {
    /// Diagnostics at the position (Monaco's marker hover part).
    markers: Vec<crate::Marker>,
    /// Whether code actions can offer a quick fix.
    quick_fix: bool,
    editor: gpui::WeakEntity<CodeEditor>,
    /// Markdown parts: decoration messages, provider hovers, warnings.
    parts: Vec<MarkdownHoverPart>,
    /// The part last clicked, for the verbosity actions (Monaco
    /// `focusedHoverPartIndex`).
    focused_part: Option<usize>,
    /// Verbosity requests in flight per part, with their accumulated
    /// delta (Monaco `_ongoingHoverOperations`).
    verbosity_requests: HashMap<usize, (i32, Task<()>)>,
    /// Where the card is anchored, to tell which side it's on.
    anchor: PointUtf16,
    /// The size the user resized the card to.
    size: Option<Size<Pixels>>,
    /// The largest size before a resize (Monaco `_updateMaxDimensions`).
    max_size: Size<Pixels>,
    /// A resize drag: (mouse start, size at start, card above its line).
    resizing: Option<(gpui::Point<Pixels>, Size<Pixels>, bool)>,
    last_size: Rc<Cell<Option<Size<Pixels>>>>,
    /// Monaco's color picker hover part.
    color_picker: Option<Entity<crate::ColorPickerWidget>>,
    /// Monaco's unicode highlighter hover parts.
    unicode: Vec<crate::unicode_highlighter::UnicodeHover>,
    /// An inlay hint label part's command, offered as a link with the
    /// keys that run it (Monaco's "Execute Command").
    command: Option<(crate::providers::Command, String)>,
    /// The inline suggestion toolbar, over ghost text with
    /// `show_toolbar: OnHover` (Monaco `InlineCompletionsHoverParticipant`).
    inline_toolbar: Option<Entity<crate::inline_completions::toolbar::InlineToolbar>>,
    style: MarkdownStyle,
    theme: Arc<crate::Theme>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The editor's line height: one scroll step (Monaco `scrollUp`).
    line_height: Pixels,
    /// Where the card was painted, so the editor knows the mouse is on it.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The room it has in the window (it overflows the editor).
    room: crate::widgets::OverflowRoomCell,
}

impl Focusable for HoverWidget {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl HoverWidget {
    /// Monaco `ContentHoverWidget.scrollUp` and friends: scrolls by `delta`
    /// (positive scrolls down), or to the top or bottom.
    fn scroll_by(&mut self, delta: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let offset = self.scroll.offset();
        let max = self.scroll.max_offset();
        // Scroll offsets are negative, down to `-max`.
        let x = (offset.x - delta.x).clamp(-max.x, px(0.));
        let y = (offset.y - delta.y).clamp(-max.y, px(0.));
        self.scroll.set_offset(point(x, y));
        cx.notify();
    }

    fn page(&self) -> Pixels {
        self.scroll.bounds().size.height
    }
}

/// What asked for a hover (Monaco `HoverStartSource`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HoverSource {
    Mouse,
    Click,
    Keyboard,
}

/// Hover provider requests, with their providers for verbosity requests.
type ProviderHoverTasks = Vec<(Arc<dyn HoverProvider>, Task<anyhow::Result<Option<Hover>>>)>;

/// Monaco `isValidHover`: some non-empty contents.
fn is_valid_hover(hover: &Hover) -> bool {
    hover.contents.iter().any(|c| !c.value.trim().is_empty())
}

/// A pending color picker hover part.
type PickerTask = Option<Task<Option<Entity<crate::ColorPickerWidget>>>>;

impl Render for HoverWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;
        let mut sections = Vec::new();
        for marker in &self.markers {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.hover_border).into_any_element());
            }
            let copy = match marker.source_label() {
                Some(label) => format!("{} {label}", marker.message),
                None => marker.message.to_string(),
            };
            let row = self.render_marker(marker);
            sections.push(self.copy_row(("marker-row", sections.len()), row, copy, cx));
        }
        if let Some(picker) = &self.color_picker {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.hover_border).into_any_element());
            }
            sections.push(picker.clone().into_any_element());
        }
        for (ix, hover) in self.unicode.iter().enumerate() {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.hover_border).into_any_element());
            }
            // The reason, then Monaco's "Adjust settings" link.
            let editor = self.editor.clone();
            let part = hover.clone();
            let row = div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().w(px(480.)).child(render_markdown(
                    &MarkdownString::new(hover.reason.clone()),
                    &self.style,
                    &self.theme,
                    cx,
                )))
                .child(
                    div()
                        .id(("unicode-adjust", ix))
                        .cursor_pointer()
                        .text_color(self.style.link)
                        .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor.upgrade() {
                                let part = part.clone();
                                editor.update(cx, |editor, cx| {
                                    editor.hide_hover(cx);
                                    editor.show_unicode_exclude_options(part, window, cx);
                                });
                            }
                        })
                        .child("Adjust settings"),
                )
                .into_any_element();
            let copy = hover.reason.to_string();
            sections.push(self.copy_row(("unicode-row", ix), row, copy, cx));
        }
        for ix in 0..self.parts.len() {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.hover_border).into_any_element());
            }
            let row = self.render_markdown_part(ix, cx);
            let copy = self.parts[ix]
                .contents
                .iter()
                .map(|c| c.value.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(self.copy_row(("markdown-row", ix), row, copy, cx));
        }
        if let Some(toolbar) = &self.inline_toolbar {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.hover_border).into_any_element());
            }
            sections.push(toolbar.clone().into_any_element());
        }
        if let Some((command, keys)) = &self.command {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.hover_border).into_any_element());
            }
            let editor = self.editor.clone();
            let run = command.clone();
            sections.push(
                div()
                    .flex()
                    .child(
                        div()
                            .id("inlay-command")
                            .cursor_pointer()
                            .text_color(self.style.link)
                            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                if let Some(editor) = editor.upgrade() {
                                    editor.update(cx, |editor, cx| {
                                        editor.hide_hover(cx);
                                        editor.run_inlay_hint_command(&run, cx);
                                    });
                                }
                            })
                            .child("Execute Command"),
                    )
                    .child(keys.clone())
                    .into_any_element(),
            );
        }
        let bounds = self.bounds.clone();
        let line = self.line_height;
        let resize = self.render_resize_handle(cx);
        // Monaco `ResizableContentWidget`: the card is resizable; once
        // resized it keeps that size, else it fits its content up to the
        // maximum.
        // Monaco `_findMaximumRenderingHeight`: no taller than the room
        // above or below the line in the window.
        let card = match self.size {
            Some(size) => div().w(size.width).h(size.height),
            None => div()
                .max_w(self.room.cap_width(self.max_size.width, px(100.)))
                .max_h(self.room.cap_height(self.max_size.height, line * 2.)),
        };
        let card = card
            .id("hover-card")
            .track_scroll(&self.scroll)
            .overflow_scroll()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .bg(colors.hover_background)
            .text_color(colors.hover_foreground)
            .border_1()
            .border_color(if self.focus_handle.is_focused(_window) {
                colors.focus_border
            } else {
                colors.hover_border
            })
            .children(sections);
        div()
            .id("hover")
            .key_context(HOVER_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .text_size(self.style.font_size)
            // The editor's line height (Monaco sets `line-height` to it over
            // the font size), not GPUI's default.
            .line_height(gpui::relative(self.style.line_height))
            .on_action(cx.listener(move |this, _: &ScrollUpHover, _, cx| {
                this.scroll_by(point(px(0.), -line), cx)
            }))
            .on_action(cx.listener(move |this, _: &ScrollDownHover, _, cx| {
                this.scroll_by(point(px(0.), line), cx)
            }))
            .on_action(cx.listener(move |this, _: &ScrollLeftHover, _, cx| {
                this.scroll_by(point(-line, px(0.)), cx)
            }))
            .on_action(cx.listener(move |this, _: &ScrollRightHover, _, cx| {
                this.scroll_by(point(line, px(0.)), cx)
            }))
            .on_action(cx.listener(|this, _: &PageUpHover, _, cx| {
                let page = this.page();
                this.scroll_by(point(px(0.), -page), cx)
            }))
            .on_action(cx.listener(|this, _: &PageDownHover, _, cx| {
                let page = this.page();
                this.scroll_by(point(px(0.), page), cx)
            }))
            .on_action(cx.listener(|this, _: &GoToTopHover, _, cx| {
                this.scroll_by(point(px(0.), px(-1e9)), cx)
            }))
            .on_action(cx.listener(|this, _: &GoToBottomHover, _, cx| {
                this.scroll_by(point(px(0.), px(1e9)), cx)
            }))
            .on_action(cx.listener(|this, _: &HideHover, window, cx| {
                // Monaco: Escape in the focused hover closes it and returns
                // focus to the editor.
                if let Some(editor) = this.editor.upgrade() {
                    editor.update(cx, |editor, cx| {
                        editor.hide_hover(cx);
                        window.focus(&editor.focus_handle, cx);
                    });
                }
            }))
            .child(
                canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(card)
            .child(resize)
    }
}

impl HoverWidget {
    /// Monaco `HoverCopyButton`: a hover row with a copy button that shows
    /// while the mouse is over the row.
    fn copy_row(
        &self,
        id: (&'static str, usize),
        row: gpui::AnyElement,
        text: String,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = &self.theme.colors;
        let group: gpui::SharedString = format!("{}-{}", id.0, id.1).into();
        div()
            .group(group.clone())
            .relative()
            .pr(px(20.))
            .child(row)
            .child(
                div()
                    .id(id)
                    .absolute()
                    .top(px(0.))
                    .right(px(0.))
                    .px(px(4.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .cursor_pointer()
                    .opacity(0.)
                    .group_hover(group, |style| style.opacity(1.))
                    .hover(|style| colors.toolbar_hover(style))
                    .tooltip(crate::tooltip::tooltip("Copy", colors))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            if !text.is_empty() {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            }
                        }),
                    )
                    // `.hover-copy-button`: a 16px `copy` codicon.
                    .child(codicon(Codicon::Copy)),
            )
            .into_any_element()
    }

    /// Monaco `MarkdownHoverParticipant`: the part's markdown, with
    /// verbosity controls when its provider supports them and the long
    /// line warning's "Don't Show Again".
    fn render_markdown_part(&self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = &self.theme.colors;
        let part = &self.parts[ix];
        let link = self.style.link;
        let mut contents = div().flex_1().min_w_0().flex().flex_col().gap(px(6.));
        for content in &part.contents {
            contents = contents.child(render_markdown(content, &self.style, &self.theme, cx));
        }
        if part.long_line_warning {
            let editor = self.editor.clone();
            contents = contents.child(
                div()
                    .id(("long-line-warning", ix))
                    .cursor_pointer()
                    .text_color(link)
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        if let Some(editor) = editor.upgrade() {
                            editor.update(cx, |editor, cx| editor.hide_long_line_warning_hover(cx));
                        }
                    })
                    .child("Don't Show Again"),
            );
        }
        // Monaco `_renderHoverExpansionAction`: + and - beside the part.
        let verbosity = part
            .source
            .as_ref()
            .filter(|source| source.supports(true) || source.supports(false))
            .map(|source| {
                let action = |increase: bool| {
                    let enabled = source.supports(increase);
                    let editor = self.editor.clone();
                    let label = if increase {
                        "Increase Hover Verbosity"
                    } else {
                        "Decrease Hover Verbosity"
                    };
                    div()
                        .id((
                            if increase {
                                "verbosity-up"
                            } else {
                                "verbosity-down"
                            },
                            ix,
                        ))
                        .when(enabled, |this| this.cursor_pointer().text_color(link))
                        .when(!enabled, |this| this.opacity(0.6))
                        .tooltip(crate::tooltip::tooltip(label, colors))
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if !enabled {
                                return;
                            }
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.update_hover_verbosity_level(increase, Some(ix), cx)
                                });
                            }
                        })
                        // `add` and `remove` codicons at 11px.
                        .child(
                            codicon(if increase {
                                Codicon::Add
                            } else {
                                Codicon::Remove
                            })
                            .size(px(11.)),
                        )
                };
                div()
                    .flex_none()
                    .w(px(22.))
                    .px(px(5.))
                    .border_r_1()
                    .border_color(colors.hover_border)
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(action(true))
                    .child(action(false))
            });
        div()
            .id(("markdown-part", ix))
            .flex()
            .gap(px(4.))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, _| this.focused_part = Some(ix)),
            )
            .children(verbosity)
            .child(contents)
            .into_any_element()
    }

    /// Monaco `ResizableHTMLElement`'s sashes: a corner handle on the side
    /// away from the line (bottom right below it, top right above it).
    fn render_resize_handle(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entity = cx.entity().downgrade();
        let resizing = self.resizing;
        let min_height = self.line_height + px(8.);
        let editor = self.editor.clone();
        let above = self.shown_above(cx);
        div()
            .id("hover-resize")
            .absolute()
            .right(px(-2.))
            .when(above, |this| this.top(px(-2.)))
            .when(!above, |this| this.bottom(px(-2.)))
            .size(px(8.))
            .cursor(if above {
                gpui::CursorStyle::ResizeUpRightDownLeft
            } else {
                gpui::CursorStyle::ResizeUpLeftDownRight
            })
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(bounds) = this.bounds.get() {
                        this.resizing = Some((event.position, bounds.size, above));
                        cx.notify();
                    }
                }),
            )
            .child(canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let Some((start, start_size, above)) = resizing else {
                        return;
                    };
                    let moved = entity.clone();
                    let editor = editor.clone();
                    window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                        if phase != gpui::DispatchPhase::Capture {
                            return;
                        }
                        let Some(this) = moved.upgrade() else {
                            return;
                        };
                        let editor_size = editor
                            .upgrade()
                            .and_then(|editor| editor.read(cx).layout_size());
                        this.update(cx, |this, cx| {
                            let delta = event.position - start;
                            let dy = if above { -delta.y } else { delta.y };
                            let mut width = (start_size.width + delta.x).max(px(MIN_HOVER_WIDTH));
                            let mut height = (start_size.height + dy).max(min_height);
                            if let Some(max) = editor_size {
                                width = width.min(max.width.max(px(MIN_HOVER_WIDTH)));
                                height = height.min(max.height.max(min_height));
                            }
                            let new_size = size(width, height);
                            this.size = Some(new_size);
                            this.last_size.set(Some(new_size));
                            cx.notify();
                        });
                    });
                    let released = entity.clone();
                    window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                        if phase == gpui::DispatchPhase::Capture
                            && let Some(this) = released.upgrade()
                        {
                            this.update(cx, |this, cx| {
                                this.resizing = None;
                                cx.notify();
                            });
                        }
                    });
                },
            ))
            .into_any_element()
    }

    /// Whether the card was laid out above its anchor's line.
    fn shown_above(&self, cx: &gpui::App) -> bool {
        let (Some(bounds), Some(editor)) = (self.bounds.get(), self.editor.upgrade()) else {
            return false;
        };
        editor
            .read(cx)
            .scrolled_visible_position(self.anchor)
            .is_some_and(|line| bounds.bottom() <= line.y + px(1.))
    }

    /// Monaco `MarkerHoverParticipant`: the message with its source and
    /// code, related information links, and the problem's actions.
    fn render_marker(&self, marker: &crate::Marker) -> gpui::AnyElement {
        let colors = &self.theme.colors;
        let link = self.style.link;
        // The message, then its source and code, dimmed.
        let mut column = div().flex().flex_col().gap(px(2.)).child(
            div()
                .flex()
                .gap(px(4.))
                .child(marker.message.clone())
                .children(
                    marker
                        .source_label()
                        .map(|label| div().opacity(0.6).child(label)),
                ),
        );
        for (ix, related) in marker.related_information.iter().enumerate() {
            let editor = self.editor.clone();
            let location = related.location();
            column = column.child(
                div()
                    .id(("related", ix))
                    .cursor_pointer()
                    .text_color(link)
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        if let Some(editor) = editor.upgrade() {
                            editor.update(cx, |editor, cx| {
                                editor.hide_hover(cx);
                                editor.open_location(&location, true, cx);
                            });
                        }
                    })
                    .child(related.label()),
            );
        }
        let action = |id: &'static str, label: String, run: HoverAction| {
            let editor = self.editor.clone();
            div()
                .id(id)
                .cursor_pointer()
                .text_color(link)
                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| {
                            editor.hide_hover(cx);
                            run(editor, cx);
                        });
                    }
                })
                .child(label)
        };
        let mac = cfg!(target_os = "macos");
        let view_marker = marker.clone();
        let fix_at = marker.range.start;
        // Monaco's status bar row (`editorHoverWidget.statusBarBackground`).
        let actions = div()
            .mt(px(4.))
            .pt(px(4.))
            .border_t_1()
            .border_color(colors.hover_border)
            .bg(colors.hover_status_bar_background)
            .flex()
            .gap(px(12.))
            .child(action(
                "view-problem",
                format!("View Problem ({})", if mac { "⌥F8" } else { "Alt+F8" }),
                Box::new(move |editor, cx| editor.show_marker(&view_marker, cx)),
            ))
            .when(self.quick_fix, |this| {
                this.child(action(
                    "quick-fix",
                    format!("Quick Fix... ({})", if mac { "⌘." } else { "Ctrl+." }),
                    Box::new(move |editor, cx| {
                        editor.set_selections(&[cursor::Selection::collapsed(fix_at)], cx);
                        editor.quick_fix(cx);
                    }),
                ))
            });
        column.child(actions).into_any_element()
    }
}

use gpui::prelude::FluentBuilder as _;

impl CodeEditor {
    pub(crate) fn markdown_style(&self) -> MarkdownStyle {
        let colors = &self.theme.colors;
        let font_size = self.options.font_size;
        MarkdownStyle {
            foreground: colors.hover_foreground,
            link: colors.text_link_foreground,
            code_background: colors.text_code_block_background,
            code_font: self.options.font_family.clone(),
            font_size,
            // Monaco `ContentHoverWidget._updateFont` and
            // `ParameterHintsWidget.updateFont`: the editor's line height
            // over its font size.
            line_height: f32::from(self.options.resolved_line_height()) / f32::from(font_size),
            separator: colors.text_separator_foreground,
            quote_background: colors.text_block_quote_background,
            quote_border: colors.text_block_quote_border,
        }
    }

    /// The unicode highlighter's hover parts at `position`.
    fn unicode_hovers_at(
        &self,
        position: PointUtf16,
        cx: &gpui::App,
    ) -> Vec<crate::unicode_highlighter::UnicodeHover> {
        let offset = self.snapshot.point_utf16_to_offset(position);
        self.unicode_hovers(offset, cx)
    }

    /// The unicode hover parts' reasons in the visible hover card.
    #[doc(hidden)]
    pub fn hover_unicode_reasons_for_tests(&self, cx: &gpui::App) -> Vec<gpui::SharedString> {
        self.hover
            .widget
            .as_ref()
            .map(|(widget, _)| {
                widget
                    .read(cx)
                    .unicode
                    .iter()
                    .map(|u| u.reason.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The diagnostics in the visible hover card.
    #[doc(hidden)]
    pub fn hover_markers_for_tests(&self, cx: &gpui::App) -> Vec<crate::Marker> {
        self.hover
            .widget
            .as_ref()
            .map(|(widget, _)| widget.read(cx).markers.clone())
            .unwrap_or_default()
    }

    /// The markdown parts' contents in the visible hover card.
    #[doc(hidden)]
    pub fn hover_parts_for_tests(&self, cx: &gpui::App) -> Vec<Vec<gpui::SharedString>> {
        self.hover
            .widget
            .as_ref()
            .map(|(widget, _)| {
                widget
                    .read(cx)
                    .parts
                    .iter()
                    .map(|part| part.contents.iter().map(|c| c.value.clone()).collect())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Monaco `MarkdownHoverParticipant.computeSync`'s long line warnings:
    /// past `stop_rendering_line_after` on a line that long, else on a line
    /// too long to tokenize.
    fn long_line_warnings(&self, position: PointUtf16, cx: &gpui::App) -> Vec<MarkdownHoverPart> {
        if !self.options.hover.show_long_line_warning {
            return Vec::new();
        }
        let rope = self.snapshot.rope();
        if position.row >= rope.line_count() {
            return Vec::new();
        }
        let length = rope.line_len_utf16(position.row);
        let column = position.column + 1;
        let message = match self.options.stop_rendering_line_after {
            Some(stop) if length > stop && column >= stop => STOPPED_RENDERING,
            _ if length >= self.model().read(cx).options().max_tokenization_line_length => {
                TOO_MANY_CHARACTERS
            }
            _ => return Vec::new(),
        };
        vec![MarkdownHoverPart {
            contents: vec![MarkdownString::new(message)],
            source: None,
            long_line_warning: true,
        }]
    }

    /// Monaco `editor.action.hideLongLineWarningHover` (the warning's
    /// "Don't Show Again"): turns off `hover.show_long_line_warning`.
    pub fn hide_long_line_warning_hover(&mut self, cx: &mut Context<Self>) {
        self.hide_hover(cx);
        let mut options = self.options.clone();
        options.hover.show_long_line_warning = false;
        self.update_options(options, cx);
    }

    /// Monaco `editor.action.increaseHoverVerbosityLevel` and
    /// `decreaseHoverVerbosityLevel`: asks the provider of hover part
    /// `index` (else the clicked part, else the first that supports it) for
    /// more or less detail and replaces the part with the answer. Repeated
    /// requests before an answer add up.
    pub fn update_hover_verbosity_level(
        &mut self,
        increase: bool,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some((widget, _)) = &self.hover.widget else {
            return;
        };
        let widget = widget.clone();
        let (index, source, delta) = {
            let hover = widget.read(cx);
            let index = index.or(hover.focused_part).or_else(|| {
                hover.parts.iter().position(|part| {
                    part.source
                        .as_ref()
                        .is_some_and(|source| source.supports(increase))
                })
            });
            let Some(index) = index else {
                return;
            };
            let Some(source) = hover
                .parts
                .get(index)
                .and_then(|part| part.source.clone())
                .filter(|source| source.supports(increase))
            else {
                return;
            };
            let mut delta = if increase { 1 } else { -1 };
            if let Some((pending, _)) = hover.verbosity_requests.get(&index) {
                delta += pending;
            }
            (index, source, delta)
        };
        let context = self.model().read(cx).provider_context();
        let hover_context = HoverContext {
            verbosity_request: Some(HoverVerbosityRequest {
                verbosity_delta: delta,
                previous_hover: source.hover.clone(),
            }),
        };
        let request = source.provider.provide_hover_with_context(
            &context,
            source.position,
            &hover_context,
            cx,
        );
        let weak = widget.downgrade();
        let task = cx.spawn(async move |_, cx| {
            let hover = request.await;
            weak.update(cx, |widget, cx| {
                widget.verbosity_requests.remove(&index);
                if let Ok(Some(hover)) = hover
                    && let Some(part) = widget.parts.get_mut(index)
                {
                    part.contents = hover.contents.clone();
                    part.source = Some(HoverPartSource {
                        provider: source.provider,
                        hover,
                        position: source.position,
                    });
                    cx.notify();
                }
            })
            .ok();
        });
        widget.update(cx, |widget, _| {
            widget.verbosity_requests.insert(index, (delta, task));
        });
    }

    /// Monaco `InlineCompletionsHoverParticipant.computeSync`: with
    /// `show_toolbar: OnHover`, the toolbar when `position` is where ghost
    /// text is inserted; it asks explicitly for all completions.
    fn inline_toolbar_hover(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Option<Entity<crate::inline_completions::toolbar::InlineToolbar>> {
        use crate::options::InlineSuggestShowToolbar;
        if self.options.inline_suggest.show_toolbar != InlineSuggestShowToolbar::OnHover
            || !self.inline_ghost_text_at(position, cx)
        {
            return None;
        }
        self.trigger_inline_suggest(cx);
        let editor = cx.entity();
        let theme = self.theme.clone();
        Some(cx.new(|cx| {
            crate::inline_completions::toolbar::InlineToolbar::new(&editor, theme, false, cx)
        }))
    }

    fn set_hover_inline_toolbar(
        &mut self,
        toolbar: Option<Entity<crate::inline_completions::toolbar::InlineToolbar>>,
        cx: &mut Context<Self>,
    ) {
        if let Some((widget, _)) = &self.hover.widget {
            widget.update(cx, |widget, cx| {
                widget.inline_toolbar = toolbar;
                cx.notify();
            });
        }
    }

    /// Whether the visible hover has the inline suggestion toolbar.
    #[doc(hidden)]
    pub fn hover_has_inline_toolbar_for_tests(&self, cx: &gpui::App) -> bool {
        self.hover
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).inline_toolbar.is_some())
    }

    /// Whether the hover card is being resized (Monaco `isResizing`).
    fn is_hover_resizing(&self, cx: &gpui::App) -> bool {
        self.hover
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).resizing.is_some())
    }

    /// Whether a hover card is showing.
    pub fn is_hover_visible(&self) -> bool {
        self.hover.widget.is_some()
    }

    /// Hides the hover and cancels a pending one.
    pub fn hide_hover(&mut self, cx: &mut Context<Self>) {
        self.hover.task = None;
        self.hover.hiding = None;
        self.hover.range = None;
        self.hover.gutter_row = None;
        self.hover.from_keyboard = false;
        self.hover.inlay = None;
        if let Some((_, id)) = self.hover.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Monaco `editor.action.showHover` ("Show or Focus Hover", Cmd/Ctrl+K
    /// Cmd/Ctrl+I): focuses a visible hover, or shows one at the cursor.
    pub fn show_or_focus_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((widget, _)) = &self.hover.widget {
            let handle = widget.read(cx).focus_handle.clone();
            window.focus(&handle, cx);
            return;
        }
        self.show_hover_at_cursor(cx);
    }

    /// Focuses the visible hover; returns whether one shows.
    pub(crate) fn focus_hover(&self, window: &mut Window, cx: &mut gpui::App) -> bool {
        let Some((widget, _)) = &self.hover.widget else {
            return false;
        };
        let handle = widget.read(cx).focus_handle.clone();
        window.focus(&handle, cx);
        true
    }

    /// Where the hover card was last painted.
    pub(crate) fn hover_card_bounds(&self, cx: &gpui::App) -> Option<Bounds<Pixels>> {
        let (widget, _) = self.hover.widget.as_ref()?;
        widget.read(cx).bounds.get()
    }

    /// Where the hover card was last painted, for tests.
    #[doc(hidden)]
    pub fn hover_card_bounds_for_tests(&self, cx: &gpui::App) -> Option<Bounds<Pixels>> {
        self.hover_card_bounds(cx)
    }

    /// The hover card's scroll offset, for tests.
    #[doc(hidden)]
    pub fn hover_scroll_offset_for_tests(&self, cx: &gpui::App) -> Option<gpui::Point<Pixels>> {
        let (widget, _) = self.hover.widget.as_ref()?;
        Some(widget.read(cx).scroll.offset())
    }

    /// Whether the visible hover has keyboard focus (Monaco `hoverFocused`).
    pub fn is_hover_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.hover
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).focus_handle.is_focused(window))
    }

    /// Shows the hover for the cursor position right away (Monaco
    /// `editor.action.showHover`, Cmd/Ctrl+K Cmd/Ctrl+I).
    pub fn show_hover_at_cursor(&mut self, cx: &mut Context<Self>) {
        // Widget inputs are plain inputs in Monaco.
        if self.widget_input {
            return;
        }
        let position = self.cursors.primary().position;
        self.hide_hover(cx);
        self.hover.from_keyboard = true;
        let config = self.cursor_config(cx);
        let range =
            cursor::word_at(&config, self.snapshot.rope(), position).unwrap_or(position..position);
        let (markers, mut parts, tasks) = self.hover_requests(position, cx);
        let picker = self.color_picker_request(position, HoverSource::Keyboard, cx);
        self.hover.range = Some(range.clone());
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            let picker = match picker {
                Some(task) => task.await,
                None => None,
            };
            let mut hover_range = None;
            for (provider, task) in tasks {
                if let Ok(Some(hover)) = task.await
                    && is_valid_hover(&hover)
                {
                    hover_range = hover_range.or(hover.range.clone());
                    parts.push(MarkdownHoverPart {
                        contents: hover.contents.clone(),
                        source: Some(HoverPartSource {
                            provider,
                            hover,
                            position,
                        }),
                        long_line_warning: false,
                    });
                }
            }
            this.update(cx, |this, cx| {
                let unicode = this.unicode_hovers_at(position, cx);
                let toolbar = this.inline_toolbar_hover(position, cx);
                if !parts.is_empty()
                    || !markers.is_empty()
                    || picker.is_some()
                    || !unicode.is_empty()
                    || toolbar.is_some()
                {
                    let range = hover_range.unwrap_or(range);
                    this.show_hover(range, markers, parts, picker, unicode, cx);
                    this.set_hover_inline_toolbar(toolbar, cx);
                }
            })
            .ok();
        }));
    }

    /// Monaco `ContentHoverController._onEditorMouseMove`: on mouse moves
    /// over the editor (not over other widgets) with what's under the
    /// mouse, whether it's over the hover card, and whether the card has
    /// focus.
    pub(crate) fn hover_mouse_move(
        &mut self,
        target: HoverTarget,
        over_hover: bool,
        hover_focused: bool,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        self.hover.last_mouse = Some((target, modifiers));
        // Off an inlay hint's part.
        if self.hover.inlay.is_some() && !over_hover && !hover_focused {
            self.hide_hover(cx);
        }
        let options = self.options.hover;
        let visible = self.hover.widget.is_some();
        // Monaco `_shouldKeepCurrentHover`.
        let keep = visible
            && (hover_focused
                || self.is_hover_resizing(cx)
                || (options.sticky && self.hover.from_keyboard)
                || (options.sticky && over_hover)
                || (over_hover && self.hover_color_picker(cx).is_some()));
        if keep {
            self.hover.hiding = None;
            return;
        }
        // Monaco `_shouldRescheduleHoverComputation`: a grace period for
        // the mouse to reach a sticky hover.
        if visible
            && options.sticky
            && !options.hiding_delay.is_zero()
            && !self.mouse_in_hover(target)
        {
            if self.hover.hiding.is_none() {
                let delay = options.hiding_delay;
                self.hover.hiding = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(delay).await;
                    this.update(cx, |this, cx| {
                        this.hover.hiding = None;
                        if let Some((target, modifiers)) = this.hover.last_mouse {
                            this.react_to_hover_mouse(target, over_hover, modifiers, cx);
                        }
                    })
                    .ok();
                }));
            }
            return;
        }
        self.hover.hiding = None;
        self.react_to_hover_mouse(target, over_hover, modifiers, cx);
    }

    /// Whether `target` is in what the hover is about, so it stays.
    fn mouse_in_hover(&self, target: HoverTarget) -> bool {
        match target {
            HoverTarget::Text(position) => self
                .hover
                .range
                .as_ref()
                .is_some_and(|range| range.start <= position && position <= range.end),
            HoverTarget::Gutter(row) => self.hover.gutter_row == Some(row),
            HoverTarget::None => false,
        }
    }

    /// Monaco `shouldShowHover`: `hover.enabled`, with `onKeyboardModifier`
    /// the modifier opposite the multi-cursor one.
    fn hover_enabled_for(&self, modifiers: &Modifiers) -> bool {
        match self.options.hover.enabled {
            HoverEnabled::On => true,
            HoverEnabled::Off => false,
            HoverEnabled::OnKeyboardModifier => match self.options.multi_cursor_modifier {
                MultiCursorModifier::Alt => modifiers.control || modifiers.platform,
                MultiCursorModifier::CtrlCmd => modifiers.alt,
            },
        }
    }

    /// Monaco `_onKeyDown` for `onKeyboardModifier`: pressing the modifier
    /// shows the hover under the mouse.
    pub(crate) fn hover_modifiers_changed(
        &mut self,
        modifiers: &Modifiers,
        cx: &mut Context<Self>,
    ) {
        if self.options.hover.enabled != HoverEnabled::OnKeyboardModifier
            || self.hover.widget.is_some()
            || !self.hover_enabled_for(modifiers)
        {
            return;
        }
        if let Some((target, _)) = self.hover.last_mouse {
            self.react_to_hover_mouse(target, false, *modifiers, cx);
        }
    }

    /// Monaco `_reactToEditorMouseMove`: keeps the hover if the mouse is
    /// still on its range, starts a new one after `hover.delay`, or hides.
    fn react_to_hover_mouse(
        &mut self,
        target: HoverTarget,
        over_hover: bool,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if over_hover && self.hover.widget.is_some() && self.options.hover.sticky {
            return;
        }
        if !self.hover_enabled_for(&modifiers) {
            self.hide_hover(cx);
            return;
        }
        if self.mouse_in_hover(target) {
            return;
        }
        self.hide_hover(cx);
        let delay = self.options.hover.delay;
        match target {
            HoverTarget::None => {}
            HoverTarget::Gutter(row) => self.schedule_gutter_hover(row, delay, cx),
            HoverTarget::Text(position) => self.schedule_text_hover(position, delay, cx),
        }
    }

    fn schedule_text_hover(
        &mut self,
        position: PointUtf16,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        let config = self.cursor_config(cx);
        let range =
            cursor::word_at(&config, self.snapshot.rope(), position).unwrap_or(position..position);
        self.hover.range = Some(range.clone());
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(requests) = this.update(cx, |this, cx| {
                let requests = this.hover_requests(position, cx);
                let picker = this.color_picker_request(position, HoverSource::Mouse, cx);
                (requests, picker)
            }) else {
                return;
            };
            let ((markers, mut parts, tasks), picker) = requests;
            let picker = match picker {
                Some(task) => task.await,
                None => None,
            };
            let mut hover_range = None;
            for (provider, task) in tasks {
                if let Ok(Some(hover)) = task.await
                    && is_valid_hover(&hover)
                {
                    hover_range = hover_range.or(hover.range.clone());
                    parts.push(MarkdownHoverPart {
                        contents: hover.contents.clone(),
                        source: Some(HoverPartSource {
                            provider,
                            hover,
                            position,
                        }),
                        long_line_warning: false,
                    });
                }
            }
            this.update(cx, |this, cx| {
                let unicode = this.unicode_hovers_at(position, cx);
                let toolbar = this.inline_toolbar_hover(position, cx);
                if parts.is_empty()
                    && markers.is_empty()
                    && picker.is_none()
                    && unicode.is_empty()
                    && toolbar.is_none()
                {
                    return;
                }
                let range = hover_range.unwrap_or(range);
                this.show_hover(range, markers, parts, picker, unicode, cx);
                this.set_hover_inline_toolbar(toolbar, cx);
            })
            .ok();
        }));
    }

    /// Monaco `GlyphHoverController`: the glyph margin hover messages of
    /// decorations on `row`, after `delay`.
    fn schedule_gutter_hover(&mut self, row: u32, delay: Duration, cx: &mut Context<Self>) {
        self.hover.gutter_row = Some(row);
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                let messages = this.glyph_margin_hover_messages(row, cx);
                if messages.is_empty() {
                    return;
                }
                let start = PointUtf16::new(row, 0);
                this.show_hover(start..start, Vec::new(), messages, None, Vec::new(), cx);
                this.hover.range = None;
                this.hover.gutter_row = Some(row);
            })
            .ok();
        }));
    }

    /// The `glyph_margin_hover_message`s of decorations starting on `row`.
    fn glyph_margin_hover_messages(
        &self,
        row: u32,
        cx: &mut Context<Self>,
    ) -> Vec<MarkdownHoverPart> {
        let rope = self.snapshot.rope();
        if row >= rope.line_count() {
            return Vec::new();
        }
        let start = rope.point_to_offset(text::Point::new(row, 0));
        let end = rope.point_to_offset(text::Point::new(row, u32::MAX));
        let editor_id = Some(cx.entity_id());
        self.model().update(cx, |model, _| {
            model
                .decorations_in_range(start..end, editor_id)
                .into_iter()
                .filter(|d| rope.offset_to_point(d.range.start).row == row)
                .filter_map(|d| d.options.glyph_margin_hover_message.clone())
                .map(|message| MarkdownHoverPart::new(vec![MarkdownString::new(message)]))
                .collect()
        })
    }

    /// The mouse is over an inlay hint's label `part`: after the hover
    /// delay, shows the hint's and part's tooltips, resolving the hint
    /// first (Monaco `InlayHintsHover`, which replaces the other hover
    /// parts there).
    pub(crate) fn hover_inlay_hint_part(
        &mut self,
        part: crate::inlay_hints::InlayHintPart,
        cx: &mut Context<Self>,
    ) {
        if self.hover.inlay == Some(part) {
            return;
        }
        self.hide_hover(cx);
        self.hover.inlay = Some(part);
        let delay = self.options.hover.delay;
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(contents) = this.update(cx, |this, cx| this.inlay_hint_hover_contents(part, cx))
            else {
                return;
            };
            let Some(crate::inlay_hints::InlayHintHover {
                position,
                contents,
                command,
            }) = contents.await
            else {
                return;
            };
            if contents.is_empty() && command.is_none() {
                return;
            }
            this.update(cx, |this, cx| {
                let parts = contents
                    .into_iter()
                    .map(|content| MarkdownHoverPart::new(vec![content]))
                    .collect();
                this.show_hover(position..position, Vec::new(), parts, None, Vec::new(), cx);
                this.hover.inlay = Some(part);
                if let Some((widget, _)) = &this.hover.widget {
                    widget.update(cx, |widget, cx| {
                        widget.command = command;
                        cx.notify();
                    });
                }
            })
            .ok();
        }));
    }

    /// The markdown contents of the visible hover card, for tests.
    #[doc(hidden)]
    pub fn hover_contents_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        self.hover
            .widget
            .as_ref()
            .map(|(widget, _)| {
                let widget = widget.read(cx);
                widget
                    .parts
                    .iter()
                    .flat_map(|part| part.contents.iter().map(|c| c.value.to_string()))
                    .chain(
                        widget
                            .command
                            .iter()
                            .map(|(_, keys)| format!("Execute Command{keys}")),
                    )
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Markers and decoration hover messages at `position`, plus provider
    /// requests.
    #[allow(clippy::type_complexity)]
    fn hover_requests(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> (
        Vec<crate::Marker>,
        Vec<MarkdownHoverPart>,
        ProviderHoverTasks,
    ) {
        let offset = self.snapshot.point_utf16_to_offset(position);
        // Monaco's marker hover part comes first.
        let markers = if self.shows_validation_decorations() {
            self.model().read(cx).markers_at(offset)
        } else {
            Vec::new()
        };
        // Monaco `MarkdownHoverParticipant.computeSync`: long line warnings,
        // then decoration hover messages.
        let mut parts = self.long_line_warnings(position, cx);
        let editor_id = Some(cx.entity_id());
        parts.extend(self.model().update(cx, |model, _| {
            model
                .decorations_in_range(offset..offset, editor_id)
                .into_iter()
                .filter(|d| d.range.start <= offset && offset <= d.range.end)
                .filter_map(|d| d.options.hover_message.clone())
                .map(|message| MarkdownHoverPart::new(vec![MarkdownString::new(message)]))
                .collect::<Vec<_>>()
        }));
        let model = self.model().read(cx);
        let context = model.provider_context();
        let providers = hover_providers(cx, self.provider_target(cx));
        let tasks = providers
            .into_iter()
            .map(|provider| {
                let task = provider.provide_hover(&context, position, cx);
                (provider, task)
            })
            .collect();
        (markers, parts, tasks)
    }

    /// Monaco `HoverColorPickerParticipant`: a picker for the color at
    /// `position`, if `source` may open one.
    pub(crate) fn color_picker_request(
        &mut self,
        position: PointUtf16,
        source: HoverSource,
        cx: &mut Context<Self>,
    ) -> PickerTask {
        use crate::options::ColorDecoratorsActivatedOn as On;
        let activated = self.options.color_decorators_activated_on;
        let valid = match source {
            HoverSource::Mouse => activated != On::Click,
            HoverSource::Click => activated != On::Hover,
            HoverSource::Keyboard => true,
        };
        if !valid {
            return None;
        }
        let data = self.color_data_at(position, cx)?;
        Some(self.create_color_picker(data, None, cx))
    }

    /// Shows the color picker hover for a click on the swatch at
    /// `position` (Monaco `HoverColorPickerContribution`).
    pub(crate) fn show_color_picker_hover(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        let Some(picker) = self.color_picker_request(position, HoverSource::Click, cx) else {
            return;
        };
        self.hide_hover(cx);
        self.hover.range = Some(position..position);
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            let Some(picker) = picker.await else {
                return;
            };
            this.update(cx, |this, cx| {
                this.show_hover(
                    position..position,
                    Vec::new(),
                    Vec::new(),
                    Some(picker),
                    Vec::new(),
                    cx,
                );
            })
            .ok();
        }));
    }

    /// The color picker in the hover, if any.
    pub(crate) fn hover_color_picker(
        &self,
        cx: &gpui::App,
    ) -> Option<Entity<crate::ColorPickerWidget>> {
        let (widget, _) = self.hover.widget.as_ref()?;
        widget.read(cx).color_picker.clone()
    }

    fn show_hover(
        &mut self,
        range: Range<PointUtf16>,
        markers: Vec<crate::Marker>,
        parts: Vec<MarkdownHoverPart>,
        color_picker: Option<Entity<crate::ColorPickerWidget>>,
        unicode: Vec<crate::unicode_highlighter::UnicodeHover>,
        cx: &mut Context<Self>,
    ) {
        // Monaco shows the color picker at the color (`forceShowAtRange`).
        let range = match &color_picker {
            Some(picker) => picker.read(cx).range().clone(),
            None => range,
        };
        let style = self.markdown_style();
        let theme = self.theme.clone();
        let quick_fix =
            !crate::providers::code_action_providers(cx, self.provider_target(cx)).is_empty();
        let editor = cx.entity().downgrade();
        let line_height = self.options.resolved_line_height();
        // Monaco `ContentHoverWidget._updateMaxDimensions`.
        let editor_size = self.layout_size().unwrap_or_default();
        let last = self.hover.last_size.get().unwrap_or_default();
        let max_size = size(
            (editor_size.width * 0.66).max(px(750.)).max(last.width),
            (editor_size.height / 4.).max(px(250.)).max(last.height),
        );
        let widget = cx.new(|cx| HoverWidget {
            markers,
            quick_fix,
            editor,
            parts,
            focused_part: None,
            verbosity_requests: HashMap::new(),
            anchor: range.start,
            size: None,
            max_size,
            resizing: None,
            last_size: self.hover.last_size.clone(),
            color_picker,
            unicode,
            command: None,
            inline_toolbar: None,
            style,
            theme,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            line_height,
            bounds: Default::default(),
            room: Default::default(),
        });
        // Monaco `hover.above`: the preferred side first.
        let preference = if self.options.hover.above {
            vec![
                ContentWidgetPreference::Above,
                ContentWidgetPreference::Below,
            ]
        } else {
            vec![
                ContentWidgetPreference::Below,
                ContentWidgetPreference::Above,
            ]
        };
        let (id, room) = self.add_overflow_content_widget(
            range.start,
            preference,
            widget.clone().into(),
            crate::widgets::layer::HOVER,
            cx,
        );
        widget.update(cx, |widget, _| widget.room = room);
        self.hover.range = Some(range);
        self.hover.widget = Some((widget, id));
    }
}
