//! Hover (Monaco's `contrib/hover`): after the mouse rests over text, shows
//! decoration hover messages (e.g. diagnostics) and hover provider results in
//! a content widget.

use std::ops::Range;
use std::time::Duration;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, Modifiers, ParentElement, Pixels, Render, ScrollHandle,
    StatefulInteractiveElement, Styled, Task, Window, canvas, div, point, px,
};
use text::PointUtf16;

use crate::actions::{
    GoToBottomHover, GoToTopHover, HideHover, PageDownHover, PageUpHover, ScrollDownHover,
    ScrollLeftHover, ScrollRightHover, ScrollUpHover,
};
use crate::cursor;
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_markdown};
use crate::options::{HoverEnabled, MultiCursorModifier};
use crate::providers::{MarkdownString, hover_providers};
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

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

/// What a hover action link runs.
type HoverAction = Box<dyn Fn(&mut CodeEditor, &mut Context<CodeEditor>)>;

/// The hover card.
pub struct HoverWidget {
    /// Diagnostics at the position (Monaco's marker hover part).
    markers: Vec<crate::Marker>,
    /// Whether code actions can offer a quick fix.
    quick_fix: bool,
    editor: gpui::WeakEntity<CodeEditor>,
    contents: Vec<MarkdownString>,
    /// Monaco's color picker hover part.
    color_picker: Option<Entity<crate::ColorPickerWidget>>,
    /// Monaco's unicode highlighter hover parts.
    unicode: Vec<crate::unicode_highlighter::UnicodeHover>,
    style: MarkdownStyle,
    theme: std::sync::Arc<crate::Theme>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The editor's line height: one scroll step (Monaco `scrollUp`).
    line_height: Pixels,
    /// Where the card was painted, so the editor knows the mouse is on it.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
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

/// A pending color picker hover part.
type PickerTask = Option<Task<Option<Entity<crate::ColorPickerWidget>>>>;

impl Render for HoverWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;
        let registry = crate::model::language_registry(cx);
        let mut sections = Vec::new();
        for marker in &self.markers {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.widget_border).into_any_element());
            }
            sections.push(self.render_marker(marker));
        }
        if let Some(picker) = &self.color_picker {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.widget_border).into_any_element());
            }
            sections.push(picker.clone().into_any_element());
        }
        for (ix, hover) in self.unicode.iter().enumerate() {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.widget_border).into_any_element());
            }
            // The reason, then Monaco's "Adjust settings" link.
            let editor = self.editor.clone();
            let part = hover.clone();
            sections.push(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().w(px(480.)).child(render_markdown(
                        &hover.reason,
                        &self.style,
                        &self.theme,
                        &registry,
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
                    .into_any_element(),
            );
        }
        for content in &self.contents {
            if !sections.is_empty() {
                sections.push(div().h(px(1.)).bg(colors.widget_border).into_any_element());
            }
            sections.push(render_markdown(
                &content.0,
                &self.style,
                &self.theme,
                &registry,
            ));
        }
        let bounds = self.bounds.clone();
        let line = self.line_height;
        div()
            .id("hover")
            .key_context(HOVER_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .track_scroll(&self.scroll)
            .max_w(px(500.))
            .max_h(px(250.))
            .overflow_scroll()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .bg(colors.widget_background)
            .border_1()
            .border_color(if self.focus_handle.is_focused(_window) {
                colors.focus_border
            } else {
                colors.widget_border
            })
            .text_size(self.style.font_size)
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
            .children(sections)
    }
}

impl HoverWidget {
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
        let actions = div()
            .mt(px(4.))
            .pt(px(4.))
            .border_t_1()
            .border_color(colors.widget_border)
            .flex()
            .gap(px(12.))
            .child(action(
                "view-problem",
                "View Problem (F8)".into(),
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
        MarkdownStyle {
            foreground: colors.widget_foreground,
            link: colors.input_option_active_border,
            code_background: colors.input_background,
            code_font: self.options.font_family.clone(),
            font_size: self.options.font_size,
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
        let position = self.cursors.primary().position;
        self.hide_hover(cx);
        self.hover.from_keyboard = true;
        let config = self.cursor_config(cx);
        let range =
            cursor::word_at(&config, self.snapshot.rope(), position).unwrap_or(position..position);
        let (markers, mut contents, tasks) = self.hover_requests(position, cx);
        let picker = self.color_picker_request(position, HoverSource::Keyboard, cx);
        self.hover.range = Some(range.clone());
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            let picker = match picker {
                Some(task) => task.await,
                None => None,
            };
            let mut hover_range = None;
            for task in tasks {
                if let Ok(Some(hover)) = task.await {
                    contents.extend(hover.contents);
                    hover_range = hover_range.or(hover.range);
                }
            }
            this.update(cx, |this, cx| {
                let unicode = this.unicode_hovers_at(position, cx);
                if !contents.is_empty()
                    || !markers.is_empty()
                    || picker.is_some()
                    || !unicode.is_empty()
                {
                    let range = hover_range.unwrap_or(range);
                    this.show_hover(range, markers, contents, picker, unicode, cx);
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
        let options = self.options.hover;
        let visible = self.hover.widget.is_some();
        // Monaco `_shouldKeepCurrentHover`.
        let keep = visible
            && (hover_focused
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
            let ((markers, mut contents, tasks), picker) = requests;
            let picker = match picker {
                Some(task) => task.await,
                None => None,
            };
            let mut hover_range = None;
            for task in tasks {
                if let Ok(Some(hover)) = task.await {
                    contents.extend(hover.contents);
                    hover_range = hover_range.or(hover.range);
                }
            }
            this.update(cx, |this, cx| {
                let unicode = this.unicode_hovers_at(position, cx);
                if contents.is_empty()
                    && markers.is_empty()
                    && picker.is_none()
                    && unicode.is_empty()
                {
                    return;
                }
                let range = hover_range.unwrap_or(range);
                this.show_hover(range, markers, contents, picker, unicode, cx);
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
    fn glyph_margin_hover_messages(&self, row: u32, cx: &mut Context<Self>) -> Vec<MarkdownString> {
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
                .map(MarkdownString)
                .collect()
        })
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
        Vec<MarkdownString>,
        Vec<Task<anyhow::Result<Option<crate::providers::Hover>>>>,
    ) {
        let offset = self.snapshot.point_utf16_to_offset(position);
        // Monaco's marker hover part comes first.
        let markers = if self.shows_validation_decorations() {
            self.model().read(cx).markers_at(offset)
        } else {
            Vec::new()
        };
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
        (markers, messages, tasks)
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
        contents: Vec<MarkdownString>,
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
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let quick_fix =
            !crate::providers::code_action_providers(cx, language.as_deref()).is_empty();
        let editor = cx.entity().downgrade();
        let line_height = self.options.resolved_line_height();
        let widget = cx.new(|cx| HoverWidget {
            markers,
            quick_fix,
            editor,
            contents,
            color_picker,
            unicode,
            style,
            theme,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            line_height,
            bounds: Default::default(),
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
        let id = self.add_content_widget(
            ContentWidget {
                position: range.start,
                preference,
                view: widget.clone().into(),
            },
            cx,
        );
        self.hover.range = Some(range);
        self.hover.widget = Some((widget, id));
    }
}
