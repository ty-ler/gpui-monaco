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
use crate::widgets::{ContentWidgetPreference, WidgetId, card};

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
    /// The `hoverHighlight` decoration on the visible hover's range
    /// (Monaco `RenderedContentHoverParts._createEditorDecorations`).
    highlight: Option<crate::api::DecorationsCollection>,
    /// The marker whose quick fixes were last asked for, and whether it had
    /// any (Monaco `MarkerHoverParticipant.recentMarkerCodeActionsInfo`).
    recent_marker_code_actions: Option<(crate::Marker, bool)>,
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
    /// The range it's about, for the `hoverHighlight` decoration (Monaco
    /// `IHoverPart.range`); `None` for parts that aren't about text.
    range: Option<Range<PointUtf16>>,
}

impl MarkdownHoverPart {
    fn new(contents: Vec<MarkdownString>) -> Self {
        Self {
            contents,
            source: None,
            long_line_warning: false,
            range: None,
        }
    }
}

/// Monaco's message when rendering stops on a long line.
const STOPPED_RENDERING: &str = "Rendering paused for long line for performance reasons. This can be configured via `stop_rendering_line_after`.";
/// Monaco's message when tokenization skips a long line.
const TOO_MANY_CHARACTERS: &str = "Tokenization is skipped for long lines for performance reasons. This can be configured via `max_tokenization_line_length`.";

/// Monaco's minimum hover width when resizing.
const MIN_HOVER_WIDTH: f32 = 150.;

/// Monaco `MarkerHoverParticipant`: "Checking for quick fixes..." shows
/// once the request takes this long.
const CHECKING_QUICK_FIXES_DELAY: Duration = Duration::from_millis(200);

/// The width of a resize sash's hit area (Monaco's `Sash` size).
const SASH_SIZE: f32 = 4.;

/// The quick fix entry of the hover's status bar (Monaco
/// `MarkerHoverParticipant._renderMarkerStatusbar`).
#[derive(Clone)]
enum QuickFixStatus {
    /// A read-only editor, or no marker: no entry.
    Hidden,
    /// Asked for: a blank placeholder, so the bar doesn't flicker.
    Pending,
    /// Still asking after 200ms: "Checking for quick fixes...".
    Checking,
    /// The marker has none: "No quick fixes available".
    Unavailable,
    /// "Quick Fix...", which lists these, and the first AI action.
    Available(Rc<crate::code_action::CodeActionSet>),
}

/// Which edges a resize drag moves (Monaco `ResizableHTMLElement`'s
/// sashes: east, and north above the line or south below it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResizeEdges {
    /// The east edge: the width.
    east: bool,
    /// The north or south edge (away from the line): the height.
    vertical: bool,
}

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

/// The hover card.
pub struct HoverWidget {
    /// Diagnostics at the position (Monaco's marker hover part).
    markers: Vec<crate::Marker>,
    /// The status bar's quick fix entry, for the most severe marker.
    quick_fix: QuickFixStatus,
    /// Ask for the most severe marker's quick fixes.
    quick_fix_tasks: Vec<Task<()>>,
    /// The model's URI, naming same-file related information.
    uri: Option<gpui::SharedString>,
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
    /// A resize drag: (mouse start, size at start, card above its line,
    /// the edges it moves).
    resizing: Option<(gpui::Point<Pixels>, Size<Pixels>, bool, ResizeEdges)>,
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;
        // Monaco's `.hover-row`s: the parts in participant order, then the
        // status bar.
        let mut rows = Vec::new();
        for (ix, marker) in self.markers.iter().enumerate() {
            let copy = marker_accessible_content(marker, self.uri.as_deref());
            let row = self.render_marker(ix, marker);
            rows.push(self.copy_row(("marker-row", ix), row, copy, cx));
        }
        if let Some(picker) = &self.color_picker {
            rows.push(picker.clone().into_any_element());
        }
        for (ix, hover) in self.unicode.iter().enumerate() {
            // The reason, then Monaco's "Adjust settings" link.
            let editor = self.editor.clone();
            let part = hover.clone();
            let row = div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(2.))
                .px(px(8.))
                .py(px(4.))
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
            rows.push(self.copy_row(("unicode-row", ix), row, copy, cx));
        }
        for ix in 0..self.parts.len() {
            let row = self.render_markdown_part(ix, cx);
            let copy = self.parts[ix]
                .contents
                .iter()
                .map(|c| c.value.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            rows.push(self.copy_row(("markdown-row", ix), row, copy, cx));
        }
        if let Some(toolbar) = &self.inline_toolbar {
            rows.push(
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .child(toolbar.clone())
                    .into_any_element(),
            );
        }
        if let Some((command, keys)) = &self.command {
            let editor = self.editor.clone();
            let run = command.clone();
            rows.push(
                div()
                    .flex()
                    .px(px(8.))
                    .py(px(4.))
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
        rows.extend(self.render_status_bar());
        // Monaco `.hover-row:not(:first-child):not(:empty)`: a top border
        // across the card's full width.
        let rows = rows.into_iter().enumerate().map(|(ix, row)| {
            div()
                .when(ix > 0, |this| {
                    this.border_t_1().border_color(colors.hover_border)
                })
                .child(row)
        });
        let bounds = self.bounds.clone();
        let line = self.line_height;
        let resize = self.render_resize_sashes(cx);
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
            .flex()
            .flex_col()
            // `.monaco-hover`: `border-radius: var(--vscode-cornerRadius-large)`
            // and `box-shadow: var(--vscode-shadow-lg)`.
            .rounded(card::CORNER_RADIUS)
            .shadow(card::shadow_lg())
            .bg(colors.hover_background)
            .text_color(colors.hover_foreground)
            .border_1()
            .border_color(if self.focus_handle.is_focused(window) {
                colors.focus_border
            } else {
                colors.hover_border
            })
            .children(rows);
        // Monaco's `.monaco-hover` is a `role="tooltip"` whose contents
        // screen readers read.
        let label = self.accessible_content();
        div()
            .id("hover")
            .role(gpui::Role::Tooltip)
            .aria_label(gpui::SharedString::from(label))
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
            .children(resize)
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
                    // `.hover-copy-button { top: 4px; right: 4px }`.
                    .top(px(4.))
                    .right(px(4.))
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
        // Each content is a `div.hover-contents` with `padding: 4px 8px`.
        let mut contents = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(8.))
            .py(px(4.));
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
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, _| this.focused_part = Some(ix)),
            )
            .children(verbosity)
            .child(contents)
            .into_any_element()
    }

    /// Monaco `ResizableContentWidget`'s sashes: the full east edge, and the
    /// full edge away from the line (north above it, south below it), with
    /// the corner between them moving both.
    fn render_resize_sashes(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let entity = cx.entity().downgrade();
        let resizing = self.resizing;
        let above = self.shown_above(cx);
        let sash_size = px(SASH_SIZE);
        let offset = -sash_size / 2.;
        let sash = |id: &'static str, edges: ResizeEdges, cursor: gpui::CursorStyle| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .absolute()
                .cursor(cursor)
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if let Some(bounds) = this.bounds.get() {
                            this.resizing = Some((event.position, bounds.size, above, edges));
                            cx.notify();
                        }
                    }),
                )
        };
        let east = sash(
            "hover-resize-east",
            ResizeEdges {
                east: true,
                vertical: false,
            },
            gpui::CursorStyle::ResizeLeftRight,
        )
        .top_0()
        .bottom_0()
        .right(offset)
        .w(sash_size);
        let vertical = sash(
            if above {
                "hover-resize-north"
            } else {
                "hover-resize-south"
            },
            ResizeEdges {
                east: false,
                vertical: true,
            },
            gpui::CursorStyle::ResizeUpDown,
        )
        .left_0()
        .right_0()
        .h(sash_size)
        .when(above, |this| this.top(offset))
        .when(!above, |this| this.bottom(offset));
        let corner = sash(
            "hover-resize",
            ResizeEdges {
                east: true,
                vertical: true,
            },
            if above {
                gpui::CursorStyle::ResizeUpRightDownLeft
            } else {
                gpui::CursorStyle::ResizeUpLeftDownRight
            },
        )
        .right(-sash_size)
        .size(sash_size * 2.)
        .when(above, |this| this.top(-sash_size))
        .when(!above, |this| this.bottom(-sash_size));
        let min_height = self.line_height + px(8.);
        let editor = self.editor.clone();
        // While dragging: follow the mouse anywhere in the window.
        let drag = canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let Some((start, start_size, above, edges)) = resizing else {
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
                        let dx = if edges.east { delta.x } else { px(0.) };
                        let dy = match (edges.vertical, above) {
                            (false, _) => px(0.),
                            (true, true) => -delta.y,
                            (true, false) => delta.y,
                        };
                        let mut width = (start_size.width + dx).max(px(MIN_HOVER_WIDTH));
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
        )
        .absolute()
        .size_0();
        vec![
            east.into_any_element(),
            vertical.into_any_element(),
            corner.into_any_element(),
            drag.into_any_element(),
        ]
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

    /// Monaco `MarkerHoverParticipant._renderMarkerHover`, in the editor's
    /// font: the message (`white-space: pre-wrap`), its source and code
    /// dimmed (the code a link with a target), and each related
    /// information's `file(line, col): ` as a link before its message.
    fn render_marker(&self, ix: usize, marker: &crate::Marker) -> gpui::AnyElement {
        let link = self.style.link;
        let details = (marker.source.is_some() || marker.code.is_some()).then(|| {
            div()
                .flex_none()
                .flex()
                .pl(px(6.))
                .opacity(0.6)
                .children(marker.source.clone())
                .children(marker.code.as_ref().map(|code| {
                    crate::markers::code_element(
                        code,
                        marker.code_target.as_ref(),
                        link,
                        self.editor.clone(),
                    )
                }))
        });
        let mut column = div()
            .id(("marker", ix))
            .flex()
            .flex_col()
            .px(px(8.))
            .py(px(4.))
            .font_family(self.style.code_font.clone())
            .child(
                div()
                    .flex()
                    .child(div().min_w_0().child(marker.message.clone()))
                    .children(details),
            );
        for (ix, related) in marker.related_information.iter().enumerate() {
            let editor = self.editor.clone();
            let location = related.location();
            column = column.child(
                div()
                    .mt(px(8.))
                    .flex()
                    .child(
                        div()
                            .id(("related", ix))
                            .debug_selector(move || format!("hover-related-{ix}"))
                            .role(gpui::Role::Link)
                            .flex_none()
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
                            .child(related.location_label(self.uri.as_deref())),
                    )
                    .child(div().min_w_0().child(related.message.clone())),
            );
        }
        column.into_any_element()
    }

    /// The status bar's entries (Monaco `_renderMarkerStatusbar` for the
    /// most severe marker): "View Problem" for errors, warnings and infos,
    /// then the quick fix entry.
    fn status_bar_entries(&self) -> Vec<StatusBarEntry> {
        let Some(marker) = most_severe(&self.markers) else {
            return Vec::new();
        };
        let mac = cfg!(target_os = "macos");
        let mut entries = Vec::new();
        if marker.severity >= crate::MarkerSeverity::Info {
            entries.push(StatusBarEntry::Action {
                label: "View Problem".into(),
                keys: Some(if mac { "⌥F8" } else { "Alt+F8" }),
                icon: None,
                run: StatusBarRun::ViewProblem(marker.clone()),
            });
        }
        match &self.quick_fix {
            QuickFixStatus::Hidden => {}
            // Monaco's `&nbsp;` placeholder.
            QuickFixStatus::Pending => entries.push(StatusBarEntry::Text("\u{a0}")),
            QuickFixStatus::Checking => {
                entries.push(StatusBarEntry::Text("Checking for quick fixes..."))
            }
            QuickFixStatus::Unavailable => {
                entries.push(StatusBarEntry::Text("No quick fixes available"))
            }
            QuickFixStatus::Available(set) => {
                entries.push(StatusBarEntry::Action {
                    label: "Quick Fix...".into(),
                    keys: Some(if mac { "⌘." } else { "Ctrl+." }),
                    icon: None,
                    run: StatusBarRun::QuickFix(set.clone(), marker.range.start),
                });
                if let Some(item) = set.valid_ai() {
                    entries.push(StatusBarEntry::Action {
                        label: item.action.title.clone(),
                        keys: None,
                        icon: Some(Codicon::Sparkle),
                        run: StatusBarRun::Apply(Box::new(item.clone())),
                    });
                }
            }
        }
        entries
    }

    /// Monaco `EditorHoverStatusBar`: `div.hover-row.status-bar` on
    /// `editorHoverWidget.statusBarBackground`, 12px text on 22px rows,
    /// each action a link `label (keys)`.
    fn render_status_bar(&self) -> Option<gpui::AnyElement> {
        let entries = self.status_bar_entries();
        if entries.is_empty() {
            return None;
        }
        let colors = &self.theme.colors;
        let link = self.style.link;
        let actions = entries.into_iter().enumerate().map(|(ix, entry)| {
            let StatusBarEntry::Action {
                label,
                keys,
                icon,
                run,
            } = entry
            else {
                let StatusBarEntry::Text(text) = entry else {
                    unreachable!()
                };
                return div().mr(px(16.)).child(text).into_any_element();
            };
            let editor = self.editor.clone();
            let run = Rc::new(run);
            div()
                .id(("hover-status-action", ix))
                .debug_selector(move || format!("hover-status-action-{ix}"))
                .role(gpui::Role::Button)
                .flex()
                .items_center()
                .mr(px(16.))
                .cursor_pointer()
                .text_color(link)
                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = editor.upgrade() {
                        let run = run.clone();
                        editor.update(cx, |editor, cx| run.run(editor, cx));
                    }
                })
                .children(icon.map(|icon| div().pr(px(4.)).child(codicon(icon))))
                .child(match keys {
                    Some(keys) => format!("{label} ({keys})"),
                    None => label.to_string(),
                })
                .into_any_element()
        });
        let radius = card::CORNER_RADIUS - px(1.);
        Some(
            div()
                .id("hover-status-bar")
                .debug_selector(|| "hover-status-bar".into())
                .flex()
                .px(px(8.))
                .bg(colors.hover_status_bar_background)
                .rounded_bl(radius)
                .rounded_br(radius)
                .text_size(px(12.))
                .line_height(px(22.))
                .whitespace_nowrap()
                .children(actions)
                .into_any_element(),
        )
    }

    /// Monaco `RenderedContentHoverParts.getAccessibleContent`: each part's
    /// accessible content, then the status bar's description.
    fn accessible_content(&self) -> String {
        let mut content: Vec<String> = self
            .markers
            .iter()
            .map(|marker| marker_accessible_content(marker, self.uri.as_deref()))
            .collect();
        content.extend(self.unicode.iter().map(|hover| hover.reason.to_string()));
        content.extend(self.parts.iter().map(|part| {
            part.contents
                .iter()
                .map(|c| c.value.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        }));
        let entries = self.status_bar_entries();
        if !entries.is_empty() {
            let mut description = vec!["This is a hover status bar.".to_string()];
            for entry in entries {
                match entry {
                    StatusBarEntry::Action {
                        label,
                        keys: Some(keys),
                        ..
                    } => description.push(format!(
                        "It has an action with label {label} and keybinding {keys}."
                    )),
                    StatusBarEntry::Action { label, .. } => {
                        description.push(format!("It has an action with label {label}."))
                    }
                    StatusBarEntry::Text(_) => {}
                }
            }
            content.push(description.join("\n"));
        }
        content.retain(|c| !c.is_empty());
        content.join("\n\n")
    }
}

/// An entry of the hover's status bar.
enum StatusBarEntry {
    /// Monaco `HoverAction`: a link, with its keybinding and icon.
    Action {
        label: gpui::SharedString,
        keys: Option<&'static str>,
        icon: Option<Codicon>,
        run: StatusBarRun,
    },
    /// Monaco's quick fix placeholder text.
    Text(&'static str),
}

/// What a status bar action does.
enum StatusBarRun {
    /// "View Problem": the marker zone at it.
    ViewProblem(crate::Marker),
    /// "Quick Fix...": the marker's quick fixes in the code action menu.
    QuickFix(Rc<crate::code_action::CodeActionSet>, PointUtf16),
    /// An AI quick fix, applied right away.
    Apply(Box<crate::code_action::Item>),
}

impl StatusBarRun {
    /// Monaco: hides the hover, then runs.
    fn run(&self, editor: &mut CodeEditor, cx: &mut Context<CodeEditor>) {
        editor.hide_hover(cx);
        match self {
            StatusBarRun::ViewProblem(marker) => editor.show_marker(marker, cx),
            StatusBarRun::QuickFix(set, position) => {
                editor.show_marker_quick_fixes(set.clone(), *position, cx)
            }
            StatusBarRun::Apply(item) => {
                editor.apply_code_action((**item).clone(), false, false, cx)
            }
        }
    }
}

/// The most severe of `markers`, the first of equals (Monaco sorts the
/// marker parts by `MarkerSeverity.compare` for the status bar).
fn most_severe(markers: &[crate::Marker]) -> Option<&crate::Marker> {
    markers.iter().fold(None, |best, marker| match best {
        Some(best) if best.severity >= marker.severity => Some(best),
        _ => Some(marker),
    })
}

/// Monaco `MarkerHoverParticipant.getAccessibleContent`: the message, then
/// each related information as `file(line, col): message`; also what the
/// copy button copies.
fn marker_accessible_content(marker: &crate::Marker, uri: Option<&str>) -> String {
    let mut lines = vec![marker.message.to_string()];
    lines.extend(
        marker
            .related_information
            .iter()
            .map(|related| format!("{}{}", related.location_label(uri), related.message)),
    );
    lines.join("\n")
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
            range: Some(position..position),
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
        if let Some(mut highlight) = self.hover.highlight.take() {
            highlight.clear(cx);
        }
        if let Some((_, id)) = self.hover.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Monaco `editor.action.showHover` ("Show or Focus Hover", Cmd/Ctrl+K
    /// Cmd/Ctrl+I): focuses a visible hover, or shows one at the cursor.
    pub fn show_or_focus_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_hover(window, cx) {
            return;
        }
        self.show_hover_at_cursor(cx);
    }

    /// Focuses the visible hover and has screen readers read it; returns
    /// whether one shows.
    pub(crate) fn focus_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((widget, _)) = &self.hover.widget else {
            return false;
        };
        let (handle, content) = {
            let widget = widget.read(cx);
            (widget.focus_handle.clone(), widget.accessible_content())
        };
        window.focus(&handle, cx);
        // Monaco: the focused hover's contents are its accessible name, so
        // screen readers read them on focus (`ContentHoverWidget.show`).
        if !content.is_empty() {
            self.announce(content, cx);
        }
        true
    }

    /// The visible hover's accessible content (Monaco
    /// `ContentHoverController.getAccessibleWidgetContent`), as screen
    /// readers get it when it takes focus.
    pub fn hover_accessible_content(&self, cx: &gpui::App) -> Option<String> {
        let (widget, _) = self.hover.widget.as_ref()?;
        Some(widget.read(cx).accessible_content())
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
                        range: Some(hover.range.clone().unwrap_or(position..position)),
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
                        range: Some(hover.range.clone().unwrap_or(position..position)),
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
        let snapshot = &self.snapshot;
        parts.extend(self.model().update(cx, |model, _| {
            model
                .decorations_in_range(offset..offset, editor_id)
                .into_iter()
                .filter(|d| d.range.start <= offset && offset <= d.range.end)
                .filter_map(|d| {
                    let message = d.options.hover_message.clone()?;
                    let range = snapshot.offset_to_point_utf16(d.range.start)
                        ..snapshot.offset_to_point_utf16(d.range.end);
                    Some(MarkdownHoverPart {
                        range: Some(range),
                        ..MarkdownHoverPart::new(vec![MarkdownString::new(message)])
                    })
                })
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
        let highlight = self.hover_highlight_range(&range, &markers, &parts, &color_picker, cx);
        let style = self.markdown_style();
        let theme = self.theme.clone();
        let status_marker = most_severe(&markers).cloned();
        // Monaco forgets whether another marker had quick fixes.
        if self
            .hover
            .recent_marker_code_actions
            .as_ref()
            .is_some_and(|(marker, _)| Some(marker) != status_marker.as_ref())
        {
            self.hover.recent_marker_code_actions = None;
        }
        let known_unavailable = matches!(&self.hover.recent_marker_code_actions, Some((_, false)));
        // Monaco: no quick fixes in a read-only editor.
        let quick_fix = match &status_marker {
            Some(_) if !self.options.read_only => {
                if known_unavailable {
                    QuickFixStatus::Unavailable
                } else {
                    QuickFixStatus::Pending
                }
            }
            _ => QuickFixStatus::Hidden,
        };
        let editor = cx.entity().downgrade();
        let line_height = self.options.resolved_line_height();
        // Monaco `ContentHoverWidget._updateMaxDimensions`.
        let editor_size = self.layout_size().unwrap_or_default();
        let last = self.hover.last_size.get().unwrap_or_default();
        let max_size = size(
            (editor_size.width * 0.66).max(px(750.)).max(last.width),
            (editor_size.height / 4.).max(px(250.)).max(last.height),
        );
        let uri = self.model().read(cx).uri().cloned();
        let request_quick_fixes = !matches!(quick_fix, QuickFixStatus::Hidden);
        let widget = cx.new(|cx| HoverWidget {
            markers,
            quick_fix,
            quick_fix_tasks: Vec::new(),
            uri,
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
        if let Some(marker) = status_marker.filter(|_| request_quick_fixes) {
            self.request_hover_quick_fixes(&widget, marker, !known_unavailable, cx);
        }
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
        if let Some(mut old) = self.hover.highlight.take() {
            old.clear(cx);
        }
        // Monaco's `hoverHighlight` (`editor.hoverHighlightBackground`).
        self.hover.highlight = highlight.map(|highlight| {
            let options = crate::DecorationOptions {
                background: Some(self.theme.colors.hover_highlight_background),
                ..Default::default()
            };
            self.create_decorations_collection(vec![(highlight, options)], cx)
        });
        self.hover.range = Some(range);
        self.hover.widget = Some((widget, id));
    }

    /// Monaco `RenderedContentHoverParts._createEditorDecorations`: the
    /// union of the parts' ranges, a marker's clipped to the hovered line;
    /// `None` when no part is about text or the union is empty.
    fn hover_highlight_range(
        &self,
        anchor: &Range<PointUtf16>,
        markers: &[crate::Marker],
        parts: &[MarkdownHoverPart],
        color_picker: &Option<Entity<crate::ColorPickerWidget>>,
        cx: &gpui::App,
    ) -> Option<Range<PointUtf16>> {
        let row = anchor.start.row;
        let rope = self.snapshot.rope();
        let line_end = PointUtf16::new(row, rope.line_len_utf16(row.min(rope.max_point().row)));
        // Monaco `MarkerHoverParticipant.computeSync`.
        let marker_ranges = markers.iter().map(|marker| {
            let start = if marker.range.start.row == row {
                marker.range.start
            } else {
                PointUtf16::new(row, 0)
            };
            let end = if marker.range.end.row == row {
                marker.range.end
            } else {
                line_end
            };
            start..end
        });
        let picker_range = color_picker
            .as_ref()
            .map(|picker| picker.read(cx).range().clone());
        marker_ranges
            .chain(parts.iter().filter_map(|part| part.range.clone()))
            .chain(picker_range)
            .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end))
            .filter(|range| range.start < range.end)
    }

    /// Monaco `_renderMarkerStatusbar`'s quick fixes: asks for `marker`'s
    /// code actions, showing "Checking for quick fixes..." after 200ms
    /// (unless `show_checking` is off: `marker` is known to have none),
    /// then "Quick Fix..." or "No quick fixes available".
    fn request_hover_quick_fixes(
        &mut self,
        widget: &Entity<HoverWidget>,
        marker: crate::Marker,
        show_checking: bool,
        cx: &mut Context<Self>,
    ) {
        let mut tasks = Vec::new();
        if show_checking {
            let weak = widget.downgrade();
            tasks.push(cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(CHECKING_QUICK_FIXES_DELAY)
                    .await;
                weak.update(cx, |widget, cx| {
                    if matches!(widget.quick_fix, QuickFixStatus::Pending) {
                        widget.quick_fix = QuickFixStatus::Checking;
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        let request = self.marker_quick_fixes(&marker, cx);
        let weak = widget.downgrade();
        tasks.push(cx.spawn(async move |this, cx| {
            let set = request.await;
            let has_actions = set.has_valid();
            this.update(cx, |this, _| {
                this.hover.recent_marker_code_actions = Some((marker, has_actions));
            })
            .ok();
            weak.update(cx, |widget, cx| {
                widget.quick_fix = if has_actions {
                    QuickFixStatus::Available(Rc::new(set))
                } else {
                    QuickFixStatus::Unavailable
                };
                cx.notify();
            })
            .ok();
        }));
        widget.update(cx, |widget, _| widget.quick_fix_tasks = tasks);
    }

    /// The visible hover's status bar, as `label (keys)` links and plain
    /// texts.
    #[doc(hidden)]
    pub fn hover_status_bar_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        let Some((widget, _)) = &self.hover.widget else {
            return Vec::new();
        };
        widget
            .read(cx)
            .status_bar_entries()
            .into_iter()
            .map(|entry| match entry {
                StatusBarEntry::Action {
                    label,
                    keys: Some(keys),
                    ..
                } => format!("{label} ({keys})"),
                StatusBarEntry::Action { label, .. } => label.to_string(),
                StatusBarEntry::Text(text) => text.to_string(),
            })
            .collect()
    }

    /// Runs the visible hover's status bar action whose label starts with
    /// `label`; returns whether there was one.
    #[doc(hidden)]
    pub fn run_hover_status_action_for_tests(
        &mut self,
        label: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((widget, _)) = &self.hover.widget else {
            return false;
        };
        let run = widget
            .read(cx)
            .status_bar_entries()
            .into_iter()
            .find_map(|entry| match entry {
                StatusBarEntry::Action { label: l, run, .. } if l.starts_with(label) => Some(run),
                _ => None,
            });
        match run {
            Some(run) => {
                run.run(self, cx);
                true
            }
            None => false,
        }
    }

    /// The range of the visible hover's `hoverHighlight` decoration.
    #[doc(hidden)]
    pub fn hover_highlight_for_tests(&self, cx: &gpui::App) -> Option<Range<PointUtf16>> {
        self.hover.highlight.as_ref()?.range(0, cx)
    }
}
