//! The inline edit views (Monaco `view/inlineEdits`): the chosen view of
//! the shown inline edit, drawn with decorations, content widgets and view
//! zones, and the gutter indicator with its menu.
//!
//! Ported from Monaco / VS Code (MIT License, Copyright (c) Microsoft
//! Corporation), `vs/editor/contrib/inlineCompletions/browser/view/inlineEdits/`:
//! - `inlineEditsView.ts` (`InlineEditsView`: the view cache and the
//!   decorations of `originalEditorInlineDiffView.ts`),
//! - `inlineEditsViews/*`: word replacement (with its alternative action
//!   label), line replacement, insertion, deletion, side by side, collapsed,
//!   custom (`inlineEditsCustomView.ts`, an item's hint) and jump-to
//!   (`jumpToView.ts`) views, as boxes in content widgets rather than DOM
//!   overlays,
//! - `components/gutterIndicatorView.ts` (with the line number in the pill,
//!   the pulse when the view is hovered, and the pill's color and width
//!   and the icon's opacity transitions; across the whole gutter, so it
//!   shows without a glyph margin too) and `gutterIndicatorMenu.ts` (a
//!   popup kept in the window that closes when the editor scrolls, as
//!   Monaco's instant hover; with the keyboard once it has focus: Up,
//!   Down, Home and End move, Enter or Space runs, Escape closes),
//! - `inlineSuggestionsView.ts` (`_gutterIndicatorState`: the indicator for
//!   ghost text with `showInlineEditMenu`).

use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AppContext as _, Context, Entity, FocusHandle, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, WeakEntity, Window, div, point, px,
};
use text::PointUtf16;
use web_time::Instant;

use super::inline_edits::{
    InlineEditDiff, InlineEditViewKind, ViewSettings, determine_view, grow_edits_to_entire_word,
    grow_edits_until_whitespace, utf16_to_byte,
};
use super::long_distance_hint::{LongDistanceHintState, LongDistanceState};
use crate::actions::{
    ContextMenuFirst, ContextMenuHide, ContextMenuLast, ContextMenuNext, ContextMenuPrevious,
    ContextMenuSelect,
};
use crate::codicons::{Codicon, icon as codicon};
use crate::context_menu::CONTEXT_MENU_KEY_CONTEXT;
use crate::decorations::{BlockDecoration, DecorationId, DecorationOptions, InjectedText};
use crate::editor::CodeEditor;
use crate::options::LineNumbers;
use crate::providers::{Command, InlineCompletionHintStyle};
use crate::theme::{Theme, TokenStyle};
use crate::widgets::{
    ContentWidget, ContentWidgetPreference, OverlayPosition, OverlayWidget, ViewZone, WidgetId,
    ZoneHeight,
};

/// Monaco `ORIGINAL_END_PADDING`: the gap between the text and the side by
/// side card.
const ORIGINAL_END_PADDING: f32 = 20.;
/// Monaco `MODIFIED_END_PADDING` plus the card's borders.
const MODIFIED_END_PADDING: f32 = 12. + 2.;
/// Monaco: clicks on a view accept only once it showed this long.
const CLICK_DELAY: Duration = Duration::from_millis(350);
/// Monaco's gutter indicator pulse (`triggerAnimation`).
const PULSE_DURATION: Duration = Duration::from_millis(500);
/// Monaco `idealIconAreaWidth`.
const IDEAL_ICON_AREA_WIDTH: f32 = 22.;
/// Monaco `CODICON_SIZE_PX + CODICON_PADDING_PX`.
const MINIMAL_ICON_WIDTH: f32 = 16. + 2.;
/// Monaco `InlineEditsCustomView`'s `MIN_END_OF_LINE_PADDING`.
const CUSTOM_MIN_END_OF_LINE_PADDING: f32 = 14.;
/// Monaco `InlineEditsCustomView`'s `PADDING_HORIZONTALLY`.
const CUSTOM_PADDING_HORIZONTALLY: f32 = 4.;
/// Monaco `InlineEditsCustomView`'s `HORIZONTAL_OFFSET_WHEN_ABOVE_BELOW`.
const CUSTOM_HORIZONTAL_OFFSET: f32 = 4.;
/// Monaco `InlineEditsCustomView`'s `VERTICAL_OFFSET_WHEN_ABOVE_BELOW`.
const CUSTOM_VERTICAL_OFFSET: f32 = 2.;

/// The gutter menu paints above the editor's overflowing widgets, as
/// Monaco's hover service's hovers do.
const MENU_PRIORITY: usize = 1000;

/// Monaco's inline edit transitions (`transition: ... 0.2s ease-in-out`
/// on the gutter indicator, `opacity 200ms ease-in-out` on the
/// long-distance hint).
pub(crate) const TRANSITION_DURATION: Duration = Duration::from_millis(200);

/// A value a transition eases between.
pub(crate) trait Lerp: Clone + PartialEq + 'static {
    /// The value `t` (0 to 1) of the way from `self` to `to`.
    fn lerp(&self, to: &Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(&self, to: &Self, t: f32) -> Self {
        self + (to - self) * t
    }
}

impl Lerp for Pixels {
    fn lerp(&self, to: &Self, t: f32) -> Self {
        *self + (*to - *self) * t
    }
}

impl Lerp for Hsla {
    /// In sRGB, as CSS interpolates colors.
    fn lerp(&self, to: &Self, t: f32) -> Self {
        let (from, to) = (self.to_rgb(), to.to_rgb());
        gpui::Rgba {
            r: from.r.lerp(&to.r, t),
            g: from.g.lerp(&to.g, t),
            b: from.b.lerp(&to.b, t),
            a: from.a.lerp(&to.a, t),
        }
        .into()
    }
}

/// A CSS transition (Monaco's inline edit views): when the value changes,
/// it eases from what showed then to the new value over
/// [`TRANSITION_DURATION`] with `ease-in-out`. Drawn with a GPUI animation
/// that each change restarts (`generation`), so it repaints only while it
/// runs.
#[derive(Clone, Debug)]
pub(crate) struct Transition<T> {
    from: T,
    to: T,
    started: Instant,
    generation: usize,
}

impl<T: Lerp> Transition<T> {
    /// A value shown as it is, as CSS doesn't transition a first value.
    pub(crate) fn new(value: T) -> Self {
        Self {
            from: value.clone(),
            to: value,
            started: Instant::now(),
            generation: 0,
        }
    }

    /// How far along it is, 0 to 1.
    fn progress(&self) -> f32 {
        if self.generation == 0 {
            return 1.;
        }
        (self.started.elapsed().as_secs_f32() / TRANSITION_DURATION.as_secs_f32()).min(1.)
    }

    /// The value showing now.
    pub(crate) fn current(&self) -> T {
        self.from.lerp(
            &self.to,
            crate::view::animation::ease_in_out(self.progress()),
        )
    }

    /// The value it's going to.
    pub(crate) fn target(&self) -> &T {
        &self.to
    }

    /// Moves to `to` from the value showing now; whether it changed.
    pub(crate) fn set(&mut self, to: T) -> bool {
        if to == self.to {
            return false;
        }
        self.from = self.current();
        self.to = to;
        self.started = Instant::now();
        self.generation += 1;
        true
    }

    /// Whether it's still easing.
    pub(crate) fn animating(&self) -> bool {
        self.progress() < 1.
    }

    /// How many times it changed.
    pub(crate) fn generation(&self) -> usize {
        self.generation
    }

    /// `element` with the value applied: animated, for what's left of the
    /// transition, while it runs (`id` unique among the element's
    /// siblings), else as it is.
    pub(crate) fn apply<E: IntoElement + 'static>(
        &self,
        id: &'static str,
        element: E,
        apply: impl Fn(E, T) -> E + 'static,
    ) -> gpui::AnyElement {
        if !self.animating() {
            return apply(element, self.to.clone()).into_any_element();
        }
        let start = self.progress();
        let remaining = TRANSITION_DURATION.mul_f32(1. - start);
        let (from, to) = (self.from.clone(), self.to.clone());
        element
            .with_animation(
                (id, self.generation),
                Animation::new(remaining),
                move |element, delta| {
                    let t = start + (1. - start) * delta;
                    apply(
                        element,
                        from.lerp(&to, crate::view::animation::ease_in_out(t)),
                    )
                },
            )
            .into_any_element()
    }
}

/// What the gutter indicator's transitions animate (Monaco's pill
/// `transition: background-color 0.2s ease-in-out, width 0.2s ease-in-out`
/// and its icon's `opacity 0.2s ease-in-out`): the pill's foreground,
/// border and background, its width, and the icon's opacity.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct IndicatorLook {
    colors: [Hsla; 3],
    width: Pixels,
    icon_opacity: f32,
}

impl Lerp for IndicatorLook {
    fn lerp(&self, to: &Self, t: f32) -> Self {
        Self {
            colors: [
                self.colors[0].lerp(&to.colors[0], t),
                self.colors[1].lerp(&to.colors[1], t),
                self.colors[2].lerp(&to.colors[2], t),
            ],
            width: self.width.lerp(&to.width, t),
            icon_opacity: self.icon_opacity.lerp(&to.icon_opacity, t),
        }
    }
}

/// What Tab does (Monaco `InlineEditTabAction`); it colors the indicator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabAction {
    /// Tab jumps to the edit.
    Jump,
    /// Tab accepts it.
    Accept,
    /// Tab does neither.
    Inactive,
}

/// Where the gutter indicator is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndicatorPlacement {
    /// In the gutter beside this row.
    Docked(u32),
    /// Pinned to the top of the editor: the edit is above.
    Above,
    /// Pinned to the bottom: the edit is below.
    Below,
}

/// An entry of the gutter indicator's menu (Monaco `GutterIndicatorMenuContent`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GutterMenuAction {
    /// "Go To / Accept" (`editor.action.inlineSuggest.commit`).
    Accept,
    /// The alternative action
    /// (`editor.action.inlineSuggest.commitAlternativeAction`).
    AcceptAlternative,
    /// "Reject" (`editor.action.inlineSuggest.hide`).
    Reject,
    /// "Show Collapsed" or "Show Expanded"
    /// (`editor.action.inlineSuggest.toggleShowCollapsed`).
    ToggleShowCollapsed,
    /// One of the list's commands.
    Command(Command),
    /// The completion's `gutter_menu_link_action`, at the bottom.
    Link(Command),
}

/// A menu entry: its title, keybinding hint and action.
type GutterMenuItem = (SharedString, Option<&'static str>, GutterMenuAction);

/// A piece of a previewed line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Segment {
    pub(crate) text: String,
    pub(crate) color: Hsla,
    pub(crate) background: Option<Hsla>,
}

/// What is rendered, to redo it only when it changes.
#[derive(Clone, PartialEq)]
struct RenderKey {
    version: text::Version,
    id: u64,
    kind: InlineEditViewKind,
    tab_action: TabAction,
}

/// The rendered inline edit.
struct Rendered {
    key: RenderKey,
    decorations: Vec<DecorationId>,
    widgets: Vec<WidgetId>,
}

/// The inline edit view's state in the editor.
#[derive(Default)]
pub(crate) struct InlineEditViewState {
    rendered: Option<Rendered>,
    /// Monaco `_previousView`: the item, its view and the editor width the
    /// view was chosen for.
    previous: Option<(u64, InlineEditViewKind, Pixels)>,
    /// When the current view first showed.
    shown_at: Option<(u64, Instant)>,
    indicator: Option<Entity<GutterIndicator>>,
    indicator_widget: Option<(WidgetId, IndicatorPlacement)>,
    /// The long-distance hint.
    pub(crate) long_distance: LongDistanceHintState,
    /// Monaco `inlineEditsIsHovered`: the mouse is over a view.
    view_hovered: bool,
}

/// How an [`InlineEditBox`] is drawn.
pub(crate) struct BoxStyle {
    background: Option<Hsla>,
    border: Option<Hsla>,
    /// Left and top margins.
    margin: (Pixels, Pixels),
    /// Horizontal padding.
    padding: Pixels,
}

/// One previewed text box: the new word, lines or card.
pub(crate) struct InlineEditBox {
    editor: WeakEntity<CodeEditor>,
    lines: Vec<Vec<Segment>>,
    line_backgrounds: Vec<Option<Hsla>>,
    font: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    style: BoxStyle,
    /// The alternative action's label beside the new word, and its colors
    /// (foreground, border).
    alternative: Option<(SharedString, Hsla, Hsla)>,
}

impl Render for InlineEditBox {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.clone();
        let hover_editor = self.editor.clone();
        let lines = div()
            .id("inline-edit-box")
            .ml(self.style.margin.0)
            .mt(self.style.margin.1)
            .font_family(self.font.clone())
            .text_size(self.font_size)
            .line_height(self.line_height)
            .px(self.style.padding)
            .cursor_pointer()
            .when_some(self.style.background, |this, background| {
                this.bg(background)
            })
            .when_some(self.style.border, |this, border| {
                this.border_1().border_color(border).rounded(px(3.))
            })
            .on_hover(cx.listener(move |_, hovered: &bool, _, cx| {
                if let Some(editor) = hover_editor.upgrade() {
                    let hovered = *hovered;
                    editor.update(cx, |editor, cx| {
                        editor.set_inline_edit_view_hovered(hovered, cx)
                    });
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| editor.inline_edit_view_clicked(false, cx));
                    }
                }),
            )
            .children(self.lines.iter().zip(&self.line_backgrounds).map(
                |(segments, background)| {
                    div()
                        .h(self.line_height)
                        .flex()
                        .whitespace_nowrap()
                        .when_some(*background, |this, background| this.bg(background))
                        .children(segments.iter().map(|segment| {
                            div()
                                .text_color(segment.color)
                                .when_some(segment.background, |this, background| {
                                    this.bg(background)
                                })
                                .child(segment.text.clone())
                        }))
                },
            ));
        let Some((label, foreground, border)) = self.alternative.clone() else {
            return div().child(lines);
        };
        // Monaco's alternative action label (`inline-edit-alternative-
        // action-label`): the keybinding, then the label.
        let editor = self.editor.clone();
        let alternative = div()
            .id("inline-edit-alternative-action")
            .mt(self.style.margin.1)
            .ml(px(4.))
            .h(self.line_height)
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .whitespace_nowrap()
            .rounded(px(3.))
            .border_1()
            .border_color(border)
            .text_color(foreground)
            .text_size(self.font_size * 0.85)
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| editor.inline_edit_view_clicked(true, cx));
                    }
                }),
            )
            .child(div().opacity(0.7).child("⇧⇥"))
            .child(label);
        div().flex().child(lines).child(alternative)
    }
}

/// Monaco `InlineEditsCollapsedView`: a small triangle at the edit.
pub(crate) struct CollapsedMarker {
    color: Hsla,
}

impl Render for CollapsedMarker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .ml(px(-3.))
            .text_size(px(7.))
            .line_height(px(5.))
            .text_color(self.color)
            .child("▼")
    }
}

/// Monaco `JumpToView` (`style: 'label'`): "Tab to jump" at the target,
/// over the space a decoration makes for it.
pub(crate) struct JumpToLabel {
    editor: WeakEntity<CodeEditor>,
    colors: [Hsla; 3],
    line_height: Pixels,
}

impl Render for JumpToLabel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let [foreground, border, background] = self.colors;
        let editor = self.editor.clone();
        div()
            .id("inline-edit-jump-to")
            .ml(px(2.))
            .h(self.line_height)
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .whitespace_nowrap()
            .rounded(px(3.))
            .border_1()
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .text_size(px(11.))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| editor.jump_to_next_inline_edit(cx));
                    }
                }),
            )
            .child(
                div()
                    .px(px(3.))
                    .rounded(px(2.))
                    .border_1()
                    .border_color(foreground)
                    .child("Tab"),
            )
            .child("to jump")
    }
}

/// Monaco `GutterIndicatorCustomization`-free `InlineEditsGutterIndicator`:
/// a pill in the gutter beside the edit, or pinned to the top or bottom
/// while the edit is scrolled away. Docked, it spans the gutter with the
/// edit's line number before its icon. Clicking it accepts the edit (jumps
/// to it when pinned); hovering it shows the menu; hovering the edit's view
/// pulses it.
pub(crate) struct GutterIndicator {
    editor: WeakEntity<CodeEditor>,
    theme: Arc<Theme>,
    tab_action: TabAction,
    placement: IndicatorPlacement,
    /// The cursor is at or above the edit's first row.
    cursor_above: bool,
    header: SharedString,
    menu: Vec<GutterMenuItem>,
    line_height: Pixels,
    /// Monaco `_lineNumberToRender`, while docked.
    line_number: SharedString,
    /// The glyph margin's, line numbers' and line decorations' widths.
    gutter: (Pixels, Pixels, Pixels),
    /// The editor's background, under the translucent pill.
    background: Hsla,
    /// Monaco `_isHoveredOverInlineEditDebounced`: the edit's view is
    /// hovered, which shows the check mark.
    edit_hovered: bool,
    /// Bumped to restart the pulse (Monaco `triggerAnimation`).
    pulses: usize,
    hovered: bool,
    menu_hovered: bool,
    menu_open: bool,
    close: Option<Task<()>>,
    /// The menu's keyboard-selected (or hovered) row.
    active: Option<usize>,
    /// The menu's focus: while it has it, the menu stays open (Monaco
    /// `_focusIsInMenu`).
    focus: FocusHandle,
    /// The editor's scroll position when the menu opened: scrolling
    /// closes it (Monaco's `onDidScrollChange`).
    scroll: gpui::Point<Pixels>,
    /// The pill's colors and width and the icon's opacity, easing when
    /// they change.
    look: Transition<IndicatorLook>,
}

impl GutterIndicator {
    fn hover_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hovered || self.menu_hovered {
            self.close = None;
            if !self.menu_open {
                let scroll = self.editor.upgrade().map(|e| e.read(cx).scroll_position());
                self.open_menu(scroll, cx);
            }
            return;
        }
        if self.focus.contains_focused(window, cx) {
            return;
        }
        // Let the mouse move from the pill to the menu.
        self.close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            this.update(cx, |this, cx| {
                if !this.hovered && !this.menu_hovered {
                    this.menu_open = false;
                    this.active = None;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Opens the menu, with the editor's scroll position now.
    fn open_menu(&mut self, scroll: Option<gpui::Point<Pixels>>, cx: &mut Context<Self>) {
        self.menu_open = true;
        self.active = None;
        if let Some(scroll) = scroll {
            self.scroll = scroll;
        }
        cx.notify();
    }

    /// Closes the menu; with `refocus`, gives the editor the keyboard back
    /// if the menu had it.
    fn close_menu(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.focus.contains_focused(window, cx);
        self.menu_open = false;
        self.active = None;
        cx.notify();
        if refocus
            && had_focus
            && let Some(editor) = self.editor.upgrade()
        {
            let focus = editor.read(cx).focus_handle.clone();
            window.focus(&focus, cx);
        }
    }

    fn move_active(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.active = super::toolbar::step_menu(self.active, self.menu.len(), delta);
        cx.notify();
    }

    /// Runs row `ix` (Monaco's option `onAction`: closes, focuses the
    /// editor, runs the command).
    fn run(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, _, action)) = self.menu.get(ix).cloned() else {
            return;
        };
        self.close_menu(false, window, cx);
        if let Some(editor) = self.editor.upgrade() {
            let focus = editor.read(cx).focus_handle.clone();
            window.focus(&focus, cx);
            editor.update(cx, |editor, cx| {
                editor.run_inline_edit_gutter_menu_action(action, cx)
            });
        }
    }

    /// Monaco's indicator codicon: `check` while hovered, `keyboard-tab`
    /// when Tab accepts, else `arrow-down` / `arrow-up` towards the edit.
    fn icon(&self) -> Codicon {
        match self.placement {
            IndicatorPlacement::Docked(_) => {
                if self.hovered || self.edit_hovered {
                    Codicon::Check
                } else if self.tab_action == TabAction::Accept {
                    Codicon::KeyboardTab
                } else if self.cursor_above {
                    Codicon::ArrowDown
                } else {
                    Codicon::ArrowUp
                }
            }
            _ if self.tab_action == TabAction::Accept => Codicon::KeyboardTab,
            IndicatorPlacement::Above => Codicon::ArrowUp,
            IndicatorPlacement::Below => Codicon::ArrowDown,
        }
    }

    /// Monaco's docked layout: the pill spans the gutter (less a pixel on
    /// each side); the line number fills it up to the line numbers' end,
    /// the icon the rest, up to 22px, if at least 18px are left.
    fn docked_layout(&self) -> (Pixels, Pixels, Pixels, bool) {
        let (glyph, numbers, decorations) = self.gutter;
        let pill = (glyph + numbers + decorations - px(2.)).max(px(0.));
        let number = if numbers > px(0.) {
            (glyph + numbers - px(1.)).max(px(0.))
        } else {
            (glyph - px(1.))
                .max(px(0.))
                .min((pill - px(IDEAL_ICON_AREA_WIDTH)).max(px(0.)))
        };
        let icon = (pill - number).min(px(IDEAL_ICON_AREA_WIDTH));
        let visible = icon >= px(MINIMAL_ICON_WIDTH);
        (pill, number, icon.max(px(MINIMAL_ICON_WIDTH)), visible)
    }

    /// The look the indicator has for what it shows now: the pill's
    /// colors for what Tab does, its width (the gutter's when docked) and
    /// whether the icon fits.
    fn target_look(&self) -> IndicatorLook {
        let colors = &self.theme.colors;
        let colors = match self.tab_action {
            TabAction::Jump => colors.inline_edit_gutter_indicator_primary(),
            TabAction::Accept => colors.inline_edit_gutter_indicator_successful(),
            TabAction::Inactive => colors.inline_edit_gutter_indicator_secondary(),
        };
        let (pill_width, _, _, icon_visible) = self.docked_layout();
        let docked = matches!(self.placement, IndicatorPlacement::Docked(_));
        IndicatorLook {
            colors,
            width: if docked { pill_width } else { px(22.) },
            icon_opacity: if icon_visible || !docked { 1. } else { 0. },
        }
    }
}

impl Render for GutterIndicator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;
        let editor = self.editor.clone();
        let docked = matches!(self.placement, IndicatorPlacement::Docked(_));
        let (_, number_width, icon_width, _) = self.docked_layout();
        let line_height = self.line_height;
        // Monaco's transitions: the colors, width and icon opacity ease
        // from what showed when they change.
        let target = *self.look.target();
        let [_, border, _] = target.colors;
        let icon = self.look.apply(
            "inline-edit-gutter-indicator-icon",
            div()
                .flex_none()
                .w(icon_width)
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(codicon(self.icon())),
            |icon, look| icon.opacity(look.icon_opacity),
        );
        let pill = div()
            .id("inline-edit-gutter-indicator")
            .when(docked, |this| {
                this.absolute().top_0().left(px(1.)).h(line_height)
            })
            .when(!docked, |this| this.h(self.line_height))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .text_size(px(12.))
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                this.hovered = *hovered;
                this.hover_changed(window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.menu_open = false;
                    this.active = None;
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| {
                            editor.click_inline_edit_gutter_indicator(cx);
                        });
                    }
                }),
            )
            .when(docked, |this| {
                this.justify_start()
                    .child(
                        div()
                            .flex_none()
                            .w(number_width)
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_end()
                            .overflow_hidden()
                            .child(self.line_number.clone()),
                    )
                    .child(icon)
            })
            .when(!docked, |this| this.child(codicon(self.icon())));
        let pill = self
            .look
            .apply("inline-edit-gutter-indicator-look", pill, |pill, look| {
                let [foreground, border, background] = look.colors;
                pill.w(look.width)
                    .border_color(border)
                    .bg(background)
                    .text_color(foreground)
            });
        // Monaco's pulse: an outline growing from the pill as it fades.
        let pulse = (self.pulses > 0).then(|| {
            let width = target.width;
            div().absolute().rounded(px(4.)).border_2().with_animation(
                ("inline-edit-gutter-indicator-pulse", self.pulses),
                Animation::new(PULSE_DURATION),
                move |this, delta| {
                    let grow = px(-1. + 11. * delta);
                    let mut color = border;
                    color.a *= 1. - delta;
                    this.top(-grow)
                        .left(px(1.) - grow)
                        .w(width + grow * 2.)
                        .h(line_height + grow * 2.)
                        .border_color(color)
                },
            )
        });
        let menu = self.menu_open.then(|| {
            let mut menu = div()
                .id("inline-edit-gutter-menu")
                .track_focus(&self.focus)
                .key_context(CONTEXT_MENU_KEY_CONTEXT)
                .on_action(cx.listener(|this, _: &ContextMenuNext, _, cx| this.move_active(1, cx)))
                .on_action(
                    cx.listener(|this, _: &ContextMenuPrevious, _, cx| this.move_active(-1, cx)),
                )
                .on_action(cx.listener(|this, _: &ContextMenuFirst, _, cx| {
                    this.move_active(-(isize::MAX / 2), cx)
                }))
                .on_action(cx.listener(|this, _: &ContextMenuLast, _, cx| {
                    this.move_active(isize::MAX / 2, cx)
                }))
                .on_action(cx.listener(|this, _: &ContextMenuSelect, window, cx| {
                    if let Some(ix) = this.active {
                        this.run(ix, window, cx);
                    }
                }))
                .on_action(cx.listener(|this, _: &ContextMenuHide, window, cx| {
                    this.close_menu(true, window, cx)
                }))
                .p(px(4.))
                .min_w(px(180.))
                .flex()
                .flex_col()
                .occlude()
                .bg(colors.widget_background)
                .border_1()
                .border_color(colors.widget_border)
                .text_color(colors.widget_foreground)
                .text_size(px(13.))
                .shadow_md()
                .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                    this.menu_hovered = *hovered;
                    this.hover_changed(window, cx);
                }))
                // A click on the menu gives it the keyboard.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        window.focus(&this.focus, cx);
                    }),
                )
                .child(
                    div()
                        .px(px(4.))
                        .py(px(4.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .opacity(0.8)
                        .child(self.header.clone()),
                );
            for (ix, (title, keys, action)) in self.menu.iter().enumerate() {
                let separator = matches!(action, GutterMenuAction::Link(_))
                    || (matches!(action, GutterMenuAction::Command(_))
                        && !matches!(
                            self.menu.get(ix.wrapping_sub(1)),
                            Some((_, _, GutterMenuAction::Command(_)))
                        ));
                if separator {
                    menu = menu.child(div().my(px(4.)).h(px(1.)).bg(colors.widget_border));
                }
                let active = self.active == Some(ix);
                menu = menu.child(
                    div()
                        .id(("inline-edit-gutter-menu-item", ix))
                        .flex()
                        .justify_between()
                        .gap(px(12.))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        // Monaco's `.monaco-menu-option.active`, set by the
                        // mouse or the keyboard.
                        .when(active, |this| {
                            this.bg(colors.menu_selection_background)
                                .text_color(colors.menu_selection_foreground)
                        })
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.active != Some(ix) {
                                this.active = Some(ix);
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.run(ix, window, cx);
                            }),
                        )
                        .child(title.clone())
                        .children(keys.map(|keys| div().opacity(0.6).child(keys))),
                );
            }
            // Monaco shows it with the hover service, kept in the window:
            // a popup beside the pill, moved to fit.
            let left = if docked {
                target.width + px(1.)
            } else {
                px(22.)
            };
            div().absolute().top_0().left(left).child(
                gpui::deferred(
                    gpui::anchored()
                        .snap_to_window_with_margin(px(4.))
                        .child(div().pl(px(4.)).child(menu)),
                )
                .with_priority(MENU_PRIORITY),
            )
        });
        div()
            .relative()
            .size_full()
            .when(docked, |this| {
                // The editor's background under the translucent pill, so the
                // line number below doesn't show through.
                this.child(
                    self.look.apply(
                        "inline-edit-gutter-indicator-underlay",
                        div()
                            .absolute()
                            .top_0()
                            .left(px(1.))
                            .h(line_height)
                            .rounded(px(4.))
                            .bg(self.background),
                        |underlay, look| underlay.w(look.width),
                    ),
                )
            })
            .child(pill)
            .children(pulse)
            .children(menu)
    }
}
impl CodeEditor {
    /// Monaco `tabAction`: what Tab does with the inline edit.
    fn inline_edit_tab_action(&self, cx: &gpui::App) -> TabAction {
        if !self.has_focus {
            TabAction::Inactive
        } else if self.tab_should_jump_to_inline_edit(cx) {
            TabAction::Jump
        } else if self.tab_should_accept_inline_edit(cx) {
            TabAction::Accept
        } else {
            TabAction::Inactive
        }
    }

    /// Monaco `getContentRenderWidth`: `line`'s width in typical characters.
    fn inline_edit_text_width(&self, line: &str, cx: &gpui::App) -> Pixels {
        let em = self
            .layout
            .as_ref()
            .map_or(self.options.font_size * 0.6, |layout| {
                layout.metrics.em_width
            });
        let tab_size = self.model().read(cx).options().tab_size.max(1) as f32;
        let tabs = line.matches('\t').count() as f32;
        let others = line.encode_utf16().count() as f32 - tabs;
        em * (others + tabs * tab_size)
    }

    /// Monaco `InlineEditsSideBySideView.fitsInsideViewport`.
    fn inline_edit_fits_side_by_side(&self, diff: &InlineEditDiff, cx: &gpui::App) -> bool {
        let Some(layout) = self.layout.as_ref() else {
            return true;
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let original = diff
            .display_range()
            .map(|row| self.inline_edit_text_width(&rope.line(row), cx))
            .fold(px(0.), Pixels::max);
        let modified = diff
            .new_lines
            .iter()
            .map(|line| self.inline_edit_text_width(line, cx))
            .fold(px(0.), Pixels::max);
        original + modified + px(ORIGINAL_END_PADDING) + px(MODIFIED_END_PADDING)
            < layout.text_bounds.size.width
    }

    /// The item the gutter indicator is for: the inline edit, or ghost text
    /// asking for the menu (Monaco `showInlineEditMenu`), with the row of
    /// that ghost text.
    fn inline_indicator_item(&self, cx: &gpui::App) -> Option<(usize, Option<u32>)> {
        if let Some((ix, _)) = self.inline_edit_state(cx) {
            return Some((ix, None));
        }
        let (visible, selected, ghost) = self.inline_state(cx)?;
        let ix = visible[selected].0;
        self.inline.items[ix]
            .completion
            .show_inline_edit_menu
            .then_some((ix, Some(ghost.line)))
    }

    /// The menu entries (Monaco `GutterIndicatorMenuContent`).
    pub(crate) fn inline_edit_gutter_menu(
        &self,
        cx: &gpui::App,
    ) -> (SharedString, Vec<GutterMenuItem>) {
        let Some((ix, _)) = self.inline_indicator_item(cx) else {
            return (SharedString::default(), Vec::new());
        };
        let item = &self.inline.items[ix];
        let header = item
            .provider
            .display_name()
            .unwrap_or_else(|| "Inline Suggestion".into());
        let mut menu: Vec<GutterMenuItem> = vec![(
            "Go To / Accept".into(),
            Some("Tab"),
            GutterMenuAction::Accept,
        )];
        if let Some(alternative) = &item.completion.alternative_action {
            menu.push((
                alternative.command.title.clone(),
                Some("Shift+Tab"),
                GutterMenuAction::AcceptAlternative,
            ));
        }
        menu.push(("Reject".into(), Some("Escape"), GutterMenuAction::Reject));
        menu.push((
            if self.options.inline_suggest.edits.show_collapsed {
                "Show Expanded".into()
            } else {
                "Show Collapsed".into()
            },
            None,
            GutterMenuAction::ToggleShowCollapsed,
        ));
        for command in &item.commands {
            menu.push((
                command.title.clone(),
                None,
                GutterMenuAction::Command(command.clone()),
            ));
        }
        if let Some(link) = &item.completion.gutter_menu_link_action {
            menu.push((
                format!("{}...", link.title).into(),
                None,
                GutterMenuAction::Link(link.clone()),
            ));
        }
        (header, menu)
    }

    /// Runs a gutter menu entry.
    pub(crate) fn run_inline_edit_gutter_menu_action(
        &mut self,
        action: GutterMenuAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            GutterMenuAction::Accept => {
                self.accept_inline_suggestion(cx);
            }
            GutterMenuAction::AcceptAlternative => {
                self.accept_inline_suggestion_alternative_action(cx);
            }
            GutterMenuAction::Reject => self.hide_inline_suggestion(cx),
            GutterMenuAction::ToggleShowCollapsed => self.toggle_inline_edits_show_collapsed(cx),
            GutterMenuAction::Command(command) | GutterMenuAction::Link(command) => {
                if let Err(err) = crate::commands::execute_command(cx, &command) {
                    log::warn!("{err:#}");
                }
            }
        }
    }

    /// The gutter menu's titles, for tests.
    #[doc(hidden)]
    pub fn inline_edit_gutter_menu_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        let (header, menu) = self.inline_edit_gutter_menu(cx);
        std::iter::once(header.to_string())
            .chain(menu.into_iter().map(|(title, _, _)| title.to_string()))
            .collect()
    }

    /// Runs the gutter menu entry titled `title`, for tests.
    #[doc(hidden)]
    pub fn run_inline_edit_gutter_menu_item_for_tests(
        &mut self,
        title: &str,
        cx: &mut Context<Self>,
    ) {
        let (_, menu) = self.inline_edit_gutter_menu(cx);
        if let Some((_, _, action)) = menu.into_iter().find(|(t, _, _)| t == title) {
            self.run_inline_edit_gutter_menu_action(action, cx);
        }
    }

    /// Opens the gutter indicator's menu as hovering it does, focused as
    /// clicking it makes it, for tests.
    #[doc(hidden)]
    pub fn open_inline_edit_gutter_menu_for_tests(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scroll = self.scroll_position();
        if let Some(indicator) = self.inline.edit_view.indicator.clone() {
            indicator.update(cx, |indicator, cx| {
                indicator.open_menu(Some(scroll), cx);
                window.focus(&indicator.focus, cx);
            });
        }
    }

    /// Whether the gutter indicator's menu is open, and its selected row,
    /// for tests.
    #[doc(hidden)]
    pub fn inline_edit_gutter_menu_state_for_tests(&self, cx: &gpui::App) -> Option<Option<usize>> {
        let indicator = self.inline.edit_view.indicator.as_ref()?.read(cx);
        indicator.menu_open.then_some(indicator.active)
    }

    /// The gutter indicator was clicked: it accepts the edit when beside it
    /// (Monaco shows a check mark there on hover), else jumps to it. Like
    /// Monaco's `model.accept()`, accepting here doesn't ask again.
    pub fn click_inline_edit_gutter_indicator(&mut self, cx: &mut Context<Self>) {
        let docked = matches!(
            self.inline.edit_view.indicator_widget,
            Some((_, IndicatorPlacement::Docked(_)))
        );
        if docked {
            self.accept_inline(cx);
        } else {
            self.jump_to_next_inline_edit(cx);
        }
    }

    /// A view was clicked: accepts once it has shown for a moment (Monaco
    /// `_viewHasBeenShownLongerThan(350)`), with the alternative action when
    /// its label was clicked. Nothing asks again (Monaco `model.accept`).
    pub(crate) fn inline_edit_view_clicked(&mut self, alternative: bool, cx: &mut Context<Self>) {
        if !self
            .inline
            .edit_view
            .shown_at
            .is_some_and(|(_, at)| at.elapsed() >= CLICK_DELAY)
        {
            return;
        }
        if alternative {
            self.accept_inline_suggestion_alternative_action(cx);
        } else {
            self.accept_inline(cx);
        }
    }

    /// The mouse entered or left a view (Monaco `inlineEditsIsHovered`):
    /// the gutter indicator shows a check mark and pulses.
    #[doc(hidden)]
    pub fn set_inline_edit_view_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.inline.edit_view.view_hovered == hovered {
            return;
        }
        self.inline.edit_view.view_hovered = hovered;
        if let Some(indicator) = self.inline.edit_view.indicator.clone() {
            indicator.update(cx, |indicator, cx| {
                indicator.edit_hovered = hovered;
                if hovered {
                    indicator.pulses += 1;
                }
                cx.notify();
            });
        }
    }

    /// How the inline edit shows, for tests.
    #[doc(hidden)]
    pub fn inline_edit_view_for_tests(&self) -> Option<InlineEditViewKind> {
        self.inline.edit_view.rendered.as_ref().map(|r| r.key.kind)
    }

    /// Where the gutter indicator is, for tests: `Some(Ok(row))` beside a
    /// row, `Some(Err(true))` pinned at the top, `Some(Err(false))` at the
    /// bottom.
    #[doc(hidden)]
    pub fn inline_edit_gutter_indicator_for_tests(&self) -> Option<Result<u32, bool>> {
        self.inline
            .edit_view
            .indicator_widget
            .map(|(_, placement)| match placement {
                IndicatorPlacement::Docked(row) => Ok(row),
                IndicatorPlacement::Above => Err(true),
                IndicatorPlacement::Below => Err(false),
            })
    }

    /// The gutter indicator's line number, icon (as its Unicode stand-in)
    /// and pulse count, for tests.
    #[doc(hidden)]
    pub fn inline_edit_gutter_pill_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(String, String, usize)> {
        let indicator = self.inline.edit_view.indicator.as_ref()?.read(cx);
        Some((
            indicator.line_number.to_string(),
            indicator.icon().fallback().to_string(),
            indicator.pulses,
        ))
    }

    /// The inline edit views' transitions, for tests: the gutter indicator's
    /// (how many there were, whether one runs) and the long-distance hint's
    /// (the opacity it goes to, whether it's still easing).
    #[doc(hidden)]
    #[allow(clippy::type_complexity)]
    pub fn inline_edit_transitions_for_tests(
        &self,
        cx: &gpui::App,
    ) -> (Option<(usize, bool)>, Option<(f32, bool)>) {
        let indicator = self.inline.edit_view.indicator.as_ref().map(|indicator| {
            let look = &indicator.read(cx).look;
            (look.generation(), look.animating())
        });
        let hint = self.long_distance_hint_opacity(cx);
        (indicator, hint)
    }

    /// Clears the inline edit's view, indicator and long-distance hint.
    pub(crate) fn clear_inline_edit_view(&mut self, cx: &mut Context<Self>) {
        self.clear_inline_edit_rendering(cx);
        self.clear_inline_edit_indicator(cx);
        self.clear_long_distance_hint(cx);
    }

    fn clear_inline_edit_indicator(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.inline.edit_view.indicator_widget.take() {
            self.remove_widget(id, cx);
        }
        self.inline.edit_view.indicator = None;
    }

    fn clear_inline_edit_rendering(&mut self, cx: &mut Context<Self>) {
        let Some(rendered) = self.inline.edit_view.rendered.take() else {
            return;
        };
        if !rendered.decorations.is_empty() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &rendered.decorations, Vec::new(), cx)
            });
        }
        for id in rendered.widgets {
            self.remove_widget(id, cx);
        }
    }

    /// Renders the inline edit before painting (Monaco `InlineEditsView`
    /// and `_determineRenderState`). Without one, ghost text asking for the
    /// inline edit menu gets the gutter indicator.
    pub(crate) fn refresh_inline_edit(&mut self, cx: &mut Context<Self>) {
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            self.clear_inline_edit_rendering(cx);
            self.clear_long_distance_hint(cx);
            match self.inline_indicator_item(cx) {
                Some((_, Some(row))) => {
                    let focused = self.has_focus;
                    let action = if focused {
                        TabAction::Accept
                    } else {
                        TabAction::Inactive
                    };
                    self.refresh_inline_edit_indicator(row, action, cx);
                }
                _ => self.clear_inline_edit_indicator(cx),
            }
            return;
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let id = self.inline.items[ix].id;
        let cross_file = self.inline_edit_is_cross_file(ix);
        let jump_to = self.inline_edit_jump_to(ix, cx);
        let hint = self.inline_edit_hint(ix, cx);
        let jumped = self.inline.items[ix].jumped;
        let has_alternative = self.inline.items[ix]
            .completion
            .alternative_action
            .is_some();
        // The diff, against the edited model.
        let (target_rope, target_language) = match self
            .inline_edit_item(ix)
            .and_then(|edit| edit.target.clone())
        {
            Some((model, _)) => {
                let model = model.read(cx);
                (
                    model.buffer().rope().clone(),
                    model.language_id().map(|l| l.to_string()),
                )
            }
            None => (
                rope.clone(),
                self.model().read(cx).language_id().map(|l| l.to_string()),
            ),
        };
        let diff = if jump_to.is_none() {
            InlineEditDiff::new(&target_rope, edits.clone())
        } else {
            None
        };
        // Monaco reuses the view chosen for this item, unless the editor's
        // width changed for views that depend on it.
        let width = self
            .layout
            .as_ref()
            .map_or(px(0.), |layout| layout.bounds.size.width);
        let cached = self
            .inline
            .edit_view
            .previous
            .filter(|(previous, kind, previous_width)| {
                *previous == id
                    && (*previous_width == width
                        || !matches!(
                            kind,
                            InlineEditViewKind::SideBySide | InlineEditViewKind::LineReplacement
                        ))
            })
            .map(|(_, kind, _)| kind);
        let kind = if jump_to.is_some() && !cross_file {
            InlineEditViewKind::JumpTo
        } else if let Some(kind) = cached {
            kind
        } else if cross_file || (hint.is_some() && !jumped) {
            InlineEditViewKind::Custom
        } else if let Some(diff) = diff.as_ref().filter(|_| !cross_file) {
            if has_alternative {
                InlineEditViewKind::WordReplacements
            } else {
                let settings = ViewSettings {
                    cursor: self.cursors.primary().position,
                    cursors: self.cursors.selections().iter().map(|s| s.head).collect(),
                    code_shifting: self.options.inline_suggest.edits.allow_code_shifting,
                    render_side_by_side: self.options.inline_suggest.edits.render_side_by_side,
                };
                determine_view(diff, &settings, || {
                    self.inline_edit_fits_side_by_side(diff, cx)
                })
            }
        } else {
            InlineEditViewKind::Custom
        };
        if kind != InlineEditViewKind::JumpTo {
            self.inline.edit_view.previous = Some((id, kind, width));
        }
        let kind = if self.inline_edit_show_collapsed(cx) {
            InlineEditViewKind::Collapsed
        } else {
            kind
        };
        if self
            .inline
            .edit_view
            .shown_at
            .is_none_or(|(shown, _)| shown != id)
        {
            self.inline.edit_view.shown_at = Some((id, Instant::now()));
        }
        self.inline_edit_did_show(ix, cx);
        let tab_action = self.inline_edit_tab_action(cx);
        // Monaco `displayRange`: none for a cross-file edit.
        let display_row = if cross_file {
            None
        } else if let Some(at) = jump_to {
            Some(at.row)
        } else if kind == InlineEditViewKind::Custom {
            hint.as_ref().map(|(range, _)| range.start.row)
        } else {
            diff.as_ref()
                .map(|diff| diff.display_range().start.min(rope.max_point().row))
        };
        match display_row {
            Some(row) => self.refresh_inline_edit_indicator(row, tab_action, cx),
            None => self.clear_inline_edit_indicator(cx),
        }
        // The long-distance hint.
        let target = self.inline_edit_target(ix, &edits, cx);
        let hint_row = self.long_distance_hint_row(ix, target.as_ref(), cx);
        let long_distance = hint_row
            .filter(|(_, visible)| *visible)
            .and_then(|(hint_row, _)| {
                let preview = self.long_distance_preview(
                    if cross_file || jump_to.is_none() {
                        diff.as_ref()
                    } else {
                        None
                    },
                    jump_to,
                    &target_rope,
                    target_language.as_deref(),
                    cx,
                )?;
                let location = match self
                    .inline_edit_item(ix)
                    .and_then(|edit| edit.target.clone())
                {
                    Some((_, uri)) => Some(
                        uri.rsplit('/')
                            .next()
                            .unwrap_or(uri.as_ref())
                            .to_string()
                            .into(),
                    ),
                    None => self
                        .sticky_ranges_enclosing(preview.target_row)
                        .first()
                        .map(|(start, _)| {
                            let line = rope.line(*start);
                            let line = line.trim();
                            let line: String = line.chars().take(60).collect();
                            line.into()
                        }),
                };
                Some(LongDistanceState {
                    hint_row,
                    preview,
                    location,
                    cross_file,
                })
            });
        if hint_row.is_some_and(|(_, visible)| !visible) {
            self.fade_out_long_distance_hint(cx);
        } else {
            self.refresh_long_distance_hint(long_distance, tab_action, cx);
        }
        let key = RenderKey {
            version: self.model().read(cx).buffer().version(),
            id,
            kind,
            tab_action,
        };
        if self.inline.edit_view.rendered.as_ref().map(|r| &r.key) == Some(&key) {
            return;
        }
        self.clear_inline_edit_rendering(cx);
        let (decorations, widgets) = match (kind, diff.as_ref()) {
            (InlineEditViewKind::JumpTo, _) => match jump_to {
                Some(at) => self.render_inline_edit_jump_to(at, cx),
                None => (Vec::new(), Vec::new()),
            },
            (InlineEditViewKind::Custom, _) => match hint {
                Some(hint) => self.render_inline_edit_hint(hint, tab_action, cx),
                None => (Vec::new(), Vec::new()),
            },
            (_, Some(diff)) => self.render_inline_edit(ix, diff, kind, cx),
            (_, None) => (Vec::new(), Vec::new()),
        };
        let owner = Some(cx.entity_id());
        let decorations = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &[], decorations, cx)
        });
        self.inline.edit_view.rendered = Some(Rendered {
            key,
            decorations,
            widgets,
        });
    }

    /// Monaco `_lineNumberToRender`: the label the editor's line numbers
    /// give `row`.
    fn inline_edit_indicator_line_number(&self, row: u32) -> SharedString {
        let line = row + 1;
        let cursor = self.cursors.primary().position.row + 1;
        match &self.options.line_numbers {
            LineNumbers::Off => SharedString::default(),
            LineNumbers::On => line.to_string().into(),
            LineNumbers::Interval => {
                if line.is_multiple_of(10) || line == cursor {
                    line.to_string().into()
                } else {
                    SharedString::default()
                }
            }
            LineNumbers::Relative => {
                let relative = line.abs_diff(cursor);
                if relative == 0 { line } else { relative }
                    .to_string()
                    .into()
            }
            LineNumbers::Custom(render) => render.render(line),
        }
    }

    /// Places the gutter indicator: beside `row`, or pinned
    /// to the top or bottom while that row is scrolled away.
    fn refresh_inline_edit_indicator(
        &mut self,
        row: u32,
        tab_action: TabAction,
        cx: &mut Context<Self>,
    ) {
        let (placement, line_height, gutter) = match self.layout.as_ref() {
            Some(layout) => {
                let shown = layout
                    .lines
                    .iter()
                    .any(|line| line.row == row && line.columns.start == 0);
                let first = layout.lines.first().map_or(0, |line| line.row);
                let placement = if shown {
                    IndicatorPlacement::Docked(row)
                } else if row < first {
                    IndicatorPlacement::Above
                } else {
                    IndicatorPlacement::Below
                };
                let info = &layout.layout_info;
                (
                    placement,
                    layout.metrics.line_height,
                    (
                        info.glyph_margin_width,
                        info.line_numbers_width,
                        info.decorations_width,
                    ),
                )
            }
            None => (
                IndicatorPlacement::Docked(row),
                self.options.resolved_line_height(),
                (px(0.), px(0.), px(0.)),
            ),
        };
        let cursor_above = self.cursors.primary().position.row <= row;
        let (header, menu) = self.inline_edit_gutter_menu(cx);
        let line_number = match placement {
            IndicatorPlacement::Docked(row) => self.inline_edit_indicator_line_number(row),
            _ => SharedString::default(),
        };
        let background = self.theme.colors.background;
        let edit_hovered = self.inline.edit_view.view_hovered;
        let scroll = self.scroll_position();
        let indicator = match self.inline.edit_view.indicator.clone() {
            Some(indicator) => {
                indicator.update(cx, |indicator, cx| {
                    let changed = indicator.tab_action != tab_action
                        || indicator.placement != placement
                        || indicator.cursor_above != cursor_above
                        || indicator.menu != menu
                        || indicator.header != header
                        || indicator.line_number != line_number
                        || indicator.gutter != gutter;
                    indicator.tab_action = tab_action;
                    indicator.placement = placement;
                    indicator.cursor_above = cursor_above;
                    indicator.menu = menu;
                    indicator.header = header;
                    indicator.line_height = line_height;
                    indicator.line_number = line_number;
                    indicator.gutter = gutter;
                    indicator.background = background;
                    let target = indicator.target_look();
                    let changed = indicator.look.set(target) || changed;
                    // Monaco closes the menu when the editor scrolls.
                    if indicator.menu_open && indicator.scroll != scroll {
                        indicator.menu_open = false;
                        indicator.active = None;
                        indicator.hovered = false;
                        indicator.menu_hovered = false;
                        cx.notify();
                    }
                    if changed {
                        cx.notify();
                    }
                });
                indicator
            }
            None => {
                let editor = cx.entity().downgrade();
                let theme = self.theme.clone();
                let indicator = cx.new(|cx| {
                    let mut indicator = GutterIndicator {
                        editor,
                        theme,
                        tab_action,
                        placement,
                        cursor_above,
                        header,
                        menu,
                        line_height,
                        line_number,
                        gutter,
                        background,
                        edit_hovered,
                        pulses: 0,
                        hovered: false,
                        menu_hovered: false,
                        menu_open: false,
                        close: None,
                        active: None,
                        focus: cx.focus_handle(),
                        scroll,
                        look: Transition::new(IndicatorLook::default()),
                    };
                    indicator.look = Transition::new(indicator.target_look());
                    indicator
                });
                self.inline.edit_view.indicator = Some(indicator.clone());
                indicator
            }
        };
        if self
            .inline
            .edit_view
            .indicator_widget
            .is_some_and(|(_, current)| current == placement)
        {
            return;
        }
        if let Some((id, _)) = self.inline.edit_view.indicator_widget.take() {
            self.remove_widget(id, cx);
        }
        let height = self
            .layout
            .as_ref()
            .map_or(px(0.), |layout| layout.bounds.size.height);
        let id = match placement {
            // Monaco's indicator spans the gutter, so it shows without a
            // glyph margin too.
            IndicatorPlacement::Docked(row) => self.add_gutter_widget(row, indicator.into(), cx),
            IndicatorPlacement::Above => self.add_overlay_widget(
                OverlayWidget::new(
                    OverlayPosition::Custom(point(px(1.), px(2.))),
                    indicator.into(),
                ),
                cx,
            ),
            IndicatorPlacement::Below => self.add_overlay_widget(
                OverlayWidget::new(
                    OverlayPosition::Custom(point(
                        px(1.),
                        (height - line_height - px(2.)).max(px(0.)),
                    )),
                    indicator.into(),
                ),
                cx,
            ),
        };
        self.inline.edit_view.indicator_widget = Some((id, placement));
    }

    /// `lines` (the first being row `first_row`) colored like code in
    /// `language`, with `highlights` given `highlight` as background.
    pub(crate) fn colored_lines(
        &self,
        lines: &[String],
        first_row: u32,
        language: Option<&str>,
        highlights: &[std::ops::Range<PointUtf16>],
        highlight: Hsla,
        cx: &gpui::App,
    ) -> Vec<Vec<Segment>> {
        let text = lines.join("\n");
        let spans = language.and_then(|language| {
            let registry = crate::model::language_registry(cx);
            crate::colorize::capture_spans(&text, language, &registry)
        });
        let foreground = self.theme.colors.foreground;
        let mut start = 0;
        lines
            .iter()
            .enumerate()
            .map(|(ix, line)| {
                let row = first_row + ix as u32;
                let line_start = start;
                start += line.len() + 1;
                let mut colors: Vec<Hsla> = vec![foreground; line.len()];
                if let Some(spans) = &spans {
                    for (range, id) in spans {
                        let from = range.start.clamp(line_start, line_start + line.len());
                        let to = range.end.clamp(line_start, line_start + line.len());
                        let color = self.theme.style_for(*id).foreground.unwrap_or(foreground);
                        for slot in &mut colors[from - line_start..to - line_start] {
                            *slot = color;
                        }
                    }
                }
                let mut changed = vec![false; line.len()];
                for range in highlights {
                    if range.start.row > row || range.end.row < row {
                        continue;
                    }
                    let from = if range.start.row == row {
                        utf16_to_byte(line, range.start.column)
                    } else {
                        0
                    };
                    let to = if range.end.row == row {
                        utf16_to_byte(line, range.end.column)
                    } else {
                        line.len()
                    };
                    for slot in &mut changed[from..to.max(from)] {
                        *slot = true;
                    }
                }
                let mut segments: Vec<Segment> = Vec::new();
                for (ix, ch) in line.char_indices() {
                    let background = changed[ix].then_some(highlight);
                    match segments.last_mut() {
                        Some(last) if last.color == colors[ix] && last.background == background => {
                            last.text.push(ch)
                        }
                        _ => segments.push(Segment {
                            text: ch.to_string(),
                            color: colors[ix],
                            background,
                        }),
                    }
                }
                segments
            })
            .collect()
    }

    /// Monaco `JumpToView`: room made after the target with injected
    /// spaces, and the "Tab to jump" label over it.
    fn render_inline_edit_jump_to(
        &mut self,
        at: PointUtf16,
        cx: &mut Context<Self>,
    ) -> (
        Vec<(std::ops::Range<usize>, DecorationOptions)>,
        Vec<WidgetId>,
    ) {
        let rope = self.model().read(cx).buffer().rope().clone();
        let offset = rope.point_utf16_to_offset(rope.clip_point_utf16(at, text::Bias::Left));
        let decorations = vec![(
            offset..offset,
            DecorationOptions {
                after: Some(InjectedText {
                    content: "          ".into(),
                    ..Default::default()
                }),
                show_if_collapsed: true,
                ..Default::default()
            },
        )];
        let editor = cx.entity().downgrade();
        let colors = self.theme.colors.inline_edit_gutter_indicator_primary();
        let line_height = self.options.resolved_line_height();
        let view = cx.new(|_| JumpToLabel {
            editor,
            colors,
            line_height,
        });
        let widget = self.add_content_widget(
            ContentWidget::new(at, vec![ContentWidgetPreference::Exact], view.into()),
            cx,
        );
        (decorations, vec![widget])
    }

    /// Monaco `InlineEditsCustomView`: the hint's content in a box at the
    /// end of its line when the range ends near there, else below or above
    /// it where the neighboring line leaves room, else at the end.
    fn render_inline_edit_hint(
        &mut self,
        (range, hint): (
            std::ops::Range<PointUtf16>,
            crate::providers::InlineCompletionHint,
        ),
        tab_action: TabAction,
        cx: &mut Context<Self>,
    ) -> (
        Vec<(std::ops::Range<usize>, DecorationOptions)>,
        Vec<WidgetId>,
    ) {
        let rope = self.model().read(cx).buffer().rope().clone();
        let max_row = rope.max_point().row;
        let row = range.start.row.min(max_row);
        let width_of = |row: u32| self.inline_edit_text_width(&rope.line(row), cx);
        let column_x = |row: u32, column: u32| {
            let line = rope.line(row);
            let prefix = &line[..utf16_to_byte(&line, column)];
            self.inline_edit_text_width(prefix, cx)
        };
        let em = self
            .layout
            .as_ref()
            .map_or(self.options.font_size * 0.6, |layout| {
                layout.metrics.em_width
            });
        let line_width = width_of(row);
        let start_x = column_x(row, range.start.column);
        let end_x = column_x(range.end.row.min(max_row), range.end.column);
        let label_width = em * hint.content.encode_utf16().count() as f32;
        let fits = self.layout.as_ref().is_none_or(|layout| {
            line_width
                + label_width
                + px(CUSTOM_PADDING_HORIZONTALLY + CUSTOM_MIN_END_OF_LINE_PADDING)
                < layout.text_bounds.size.width
        });
        let room = |neighbor: u32| {
            width_of(neighbor)
                + px(CUSTOM_MIN_END_OF_LINE_PADDING
                    - CUSTOM_HORIZONTAL_OFFSET
                    - CUSTOM_PADDING_HORIZONTALLY)
                < start_x
        };
        let (position, preference, margin) =
            if range.start.row == range.end.row && end_x + em * 5. >= line_width && fits {
                (
                    PointUtf16::new(row, rope.line_len_utf16(row)),
                    ContentWidgetPreference::Exact,
                    (
                        px(CUSTOM_PADDING_HORIZONTALLY + CUSTOM_MIN_END_OF_LINE_PADDING),
                        px(0.),
                    ),
                )
            } else if row < max_row && room(row + 1) {
                (
                    range.start,
                    ContentWidgetPreference::Below,
                    (
                        px(CUSTOM_PADDING_HORIZONTALLY + CUSTOM_HORIZONTAL_OFFSET),
                        px(CUSTOM_VERTICAL_OFFSET),
                    ),
                )
            } else if row > 0 && room(row - 1) {
                (
                    range.start,
                    ContentWidgetPreference::Above,
                    (
                        px(CUSTOM_PADDING_HORIZONTALLY + CUSTOM_HORIZONTAL_OFFSET),
                        px(CUSTOM_VERTICAL_OFFSET),
                    ),
                )
            } else {
                (
                    PointUtf16::new(row, rope.line_len_utf16(row)),
                    ContentWidgetPreference::Exact,
                    (
                        px(CUSTOM_PADDING_HORIZONTALLY + CUSTOM_MIN_END_OF_LINE_PADDING),
                        px(0.),
                    ),
                )
            };
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let language = match hint.style {
            InlineCompletionHintStyle::Code => language,
            InlineCompletionHintStyle::Label => None,
        };
        let lines = self.colored_lines(
            std::slice::from_ref(&hint.content),
            0,
            language.as_deref(),
            &[],
            self.theme.colors.background,
            cx,
        );
        let colors = &self.theme.colors;
        let border = match tab_action {
            TabAction::Jump => colors.inline_edit_gutter_indicator_primary()[1],
            TabAction::Accept => colors.inline_edit_gutter_indicator_successful()[1],
            TabAction::Inactive => colors.inline_edit_gutter_indicator_secondary()[1],
        };
        let background = colors.background;
        let view = self.inline_edit_box(
            lines,
            vec![None],
            BoxStyle {
                background: Some(background),
                border: Some(border),
                margin,
                padding: px(CUSTOM_PADDING_HORIZONTALLY),
            },
            None,
            cx,
        );
        let widget = self.add_content_widget(
            ContentWidget::new(position, vec![preference], view.into()),
            cx,
        );
        (Vec::new(), vec![widget])
    }

    /// Previewed lines: `rows` of the modified text, colored like code,
    /// with `highlights` (modified ranges) given `highlight` as background.
    fn inline_edit_preview_lines(
        &self,
        diff: &InlineEditDiff,
        rows: std::ops::Range<u32>,
        highlights: &[std::ops::Range<PointUtf16>],
        highlight: Hsla,
        cx: &gpui::App,
    ) -> Vec<Vec<Segment>> {
        let (window_start, lines) = diff.modified_window();
        let text = lines.join("\n");
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let spans = language.and_then(|language| {
            let registry = crate::model::language_registry(cx);
            crate::colorize::capture_spans(&text, &language, &registry)
        });
        let foreground = self.theme.colors.foreground;
        let mut line_start = 0;
        let mut starts = Vec::with_capacity(lines.len());
        for line in lines {
            starts.push(line_start);
            line_start += line.len() + 1;
        }
        rows.map(|row| {
            let Some(line) = diff.modified_line(row) else {
                return Vec::new();
            };
            let start = starts[(row - window_start) as usize];
            // The innermost span over each byte, and whether it's changed.
            let mut colors: Vec<Hsla> = vec![foreground; line.len()];
            if let Some(spans) = &spans {
                for (range, id) in spans {
                    let from = range.start.max(start).min(start + line.len());
                    let to = range.end.max(start).min(start + line.len());
                    let color = self.theme.style_for(*id).foreground.unwrap_or(foreground);
                    for slot in &mut colors[from - start..to - start] {
                        *slot = color;
                    }
                }
            }
            let mut changed = vec![false; line.len()];
            for range in highlights {
                if range.start.row > row || range.end.row < row {
                    continue;
                }
                let from = if range.start.row == row {
                    utf16_to_byte(line, range.start.column)
                } else {
                    0
                };
                let to = if range.end.row == row {
                    utf16_to_byte(line, range.end.column)
                } else {
                    line.len()
                };
                for slot in &mut changed[from..to.max(from)] {
                    *slot = true;
                }
            }
            let mut segments: Vec<Segment> = Vec::new();
            for (ix, ch) in line.char_indices() {
                let background = changed[ix].then_some(highlight);
                match segments.last_mut() {
                    Some(last) if last.color == colors[ix] && last.background == background => {
                        last.text.push(ch)
                    }
                    _ => segments.push(Segment {
                        text: ch.to_string(),
                        color: colors[ix],
                        background,
                    }),
                }
            }
            segments
        })
        .collect()
    }

    /// A box view with the editor's font.
    fn inline_edit_box(
        &self,
        lines: Vec<Vec<Segment>>,
        line_backgrounds: Vec<Option<Hsla>>,
        style: BoxStyle,
        alternative: Option<(SharedString, Hsla, Hsla)>,
        cx: &mut Context<Self>,
    ) -> Entity<InlineEditBox> {
        let editor = cx.entity().downgrade();
        let font = self
            .options
            .inline_suggest
            .font_family
            .clone()
            .unwrap_or_else(|| self.options.font_family.clone());
        let font_size = self.options.font_size;
        let line_height = self.options.resolved_line_height();
        cx.new(|_| InlineEditBox {
            editor,
            lines,
            line_backgrounds,
            font,
            font_size,
            line_height,
            style,
            alternative,
        })
    }

    /// The decorations and widgets showing `diff` as `kind`.
    fn render_inline_edit(
        &mut self,
        ix: usize,
        diff: &InlineEditDiff,
        kind: InlineEditViewKind,
        cx: &mut Context<Self>,
    ) -> (
        Vec<(std::ops::Range<usize>, DecorationOptions)>,
        Vec<WidgetId>,
    ) {
        use InlineEditViewKind::*;
        let colors = self.theme.colors.clone();
        // Monaco's alternative action label beside a word replacement
        // ("Rename"; Monaco shows the occurrences while Shift is held).
        let alternative = self.inline.items[ix]
            .completion
            .alternative_action
            .as_ref()
            .map(|action| {
                let label: SharedString = match action.count {
                    Some(1) => format!("{} (1 occurrence)", action.label).into(),
                    Some(count) => format!("{} ({count} occurrences)", action.label).into(),
                    None => action.label.clone(),
                };
                let [foreground, border, _] = colors.inline_edit_gutter_indicator_secondary();
                (label, foreground, border)
            });
        let rope = self.model().read(cx).buffer().rope().clone();
        let offset = |point: PointUtf16| {
            rope.point_utf16_to_offset(rope.clip_point_utf16(point, text::Bias::Left))
        };
        let range_offsets =
            |range: &std::ops::Range<PointUtf16>| offset(range.start)..offset(range.end);
        let deleted = colors.inline_edit_original_changed_text_background();
        let inserted = colors.inline_edit_modified_changed_text_background();
        let em = self
            .layout
            .as_ref()
            .map_or(self.options.font_size * 0.6, |layout| {
                layout.metrics.em_width
            });
        let mut decorations: Vec<(std::ops::Range<usize>, DecorationOptions)> = Vec::new();
        let mut widgets = Vec::new();
        // Monaco `OriginalEditorInlineDiffView`: the original text's
        // changes, for the views that leave the text in place.
        if matches!(
            kind,
            SideBySide | Deletion | InsertionInline | LineReplacement
        ) {
            let whole_line = |rows: &std::ops::Range<u32>| {
                (
                    offset(PointUtf16::new(rows.start, 0))
                        ..offset(PointUtf16::new(
                            rows.end - 1,
                            rope.line_len_utf16(rows.end - 1),
                        )),
                    DecorationOptions {
                        background: Some(deleted),
                        is_whole_line: true,
                        ..Default::default()
                    },
                )
            };
            for (original, modified, indices) in &diff.lines {
                if original.is_empty() || modified.is_empty() {
                    if !original.is_empty() {
                        decorations.push(whole_line(original));
                    }
                    continue;
                }
                for &ix in indices {
                    let (original_range, modified_range) = &diff.inner[ix];
                    if !original.contains(&original_range.start.row) {
                        continue;
                    }
                    if kind == LineReplacement && original_range.is_empty() {
                        continue;
                    }
                    if !original_range.is_empty() {
                        decorations.push((
                            range_offsets(original_range),
                            DecorationOptions {
                                background: Some(deleted),
                                z_index: 1,
                                ..Default::default()
                            },
                        ));
                    }
                    if kind == InsertionInline {
                        let text = diff.modified_text(modified_range);
                        let at = offset(original_range.end);
                        decorations.push((
                            at..at,
                            DecorationOptions {
                                before: Some(InjectedText {
                                    content: text.into(),
                                    style: TokenStyle::default(),
                                    background: Some(inserted),
                                    ..Default::default()
                                }),
                                show_if_collapsed: true,
                                z_index: 2,
                                ..Default::default()
                            },
                        ));
                    }
                }
            }
        }
        let editor_background = colors.background;
        let modified_border = colors.inline_edit_modified_border();
        let line_background = Some(colors.inline_edit_modified_background());
        match kind {
            WordReplacements => {
                let replacements: Vec<_> = diff
                    .inner
                    .iter()
                    .map(|(original, modified)| (original.clone(), diff.modified_text(modified)))
                    .collect();
                let mut grown = grow_edits_to_entire_word(diff, replacements.clone());
                if grown.iter().any(|(range, _)| range.is_empty()) {
                    grown = grow_edits_until_whitespace(diff, replacements);
                }
                for (range, text) in grown {
                    decorations.push((
                        range_offsets(&range),
                        DecorationOptions {
                            background: Some(deleted),
                            border: Some(colors.inline_edit_original_border()),
                            ..Default::default()
                        },
                    ));
                    // The new word, colored as it would be in its line.
                    let row = range.start.row;
                    let length = text.encode_utf16().count() as u32;
                    let word = PointUtf16::new(row, range.start.column)
                        ..PointUtf16::new(row, range.start.column + length);
                    let line = self
                        .inline_edit_preview_lines(diff, row..row + 1, &[], inserted, cx)
                        .pop()
                        .unwrap_or_default();
                    let segments = clip_segments(line, word.start.column, word.end.column);
                    let segments = segments
                        .into_iter()
                        .map(|segment| Segment {
                            background: Some(inserted),
                            ..segment
                        })
                        .collect();
                    let view = self.inline_edit_box(
                        vec![segments],
                        vec![None],
                        BoxStyle {
                            background: Some(editor_background),
                            border: Some(modified_border),
                            margin: (em * 3., px(4.)),
                            padding: px(1.),
                        },
                        alternative.clone(),
                        cx,
                    );
                    widgets.push(self.add_content_widget(
                        ContentWidget::new(
                            range.start,
                            vec![ContentWidgetPreference::Below],
                            view.into(),
                        ),
                        cx,
                    ));
                }
            }
            LineReplacement => {
                let highlights: Vec<_> = diff
                    .inner
                    .iter()
                    .map(|(_, modified)| modified.clone())
                    .collect();
                let rows = diff.modified_line_range.clone();
                let lines =
                    self.inline_edit_preview_lines(diff, rows.clone(), &highlights, inserted, cx);
                let count = lines.len();
                let view = self.inline_edit_box(
                    lines,
                    vec![line_background; count],
                    BoxStyle {
                        background: Some(editor_background),
                        border: Some(modified_border),
                        margin: (px(0.), px(0.)),
                        padding: px(0.),
                    },
                    None,
                    cx,
                );
                let anchor_row = diff
                    .original_line_range
                    .end
                    .max(diff.original_line_range.start + 1)
                    .saturating_sub(1)
                    .min(rope.max_point().row);
                widgets.push(self.add_content_widget(
                    ContentWidget::new(
                        PointUtf16::new(anchor_row, 0),
                        vec![ContentWidgetPreference::Below],
                        view.into(),
                    ),
                    cx,
                ));
            }
            SideBySide => {
                let rows = diff.modified_line_range.clone();
                let highlights: Vec<_> = diff
                    .inner
                    .iter()
                    .map(|(_, modified)| modified.clone())
                    .collect();
                let mut lines =
                    self.inline_edit_preview_lines(diff, rows.clone(), &highlights, inserted, cx);
                // Monaco `createReindentEdit`: without the common indentation.
                let indentation = diff
                    .new_lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| line.len() - line.trim_start_matches([' ', '\t']).len())
                    .min()
                    .unwrap_or(0);
                for line in &mut lines {
                    let mut skip = indentation;
                    line.retain_mut(|segment| {
                        if skip == 0 {
                            return true;
                        }
                        let taken = skip.min(segment.text.len());
                        segment.text.drain(..taken);
                        skip -= taken;
                        !segment.text.is_empty()
                    });
                }
                let count = lines.len();
                let display = diff.display_range();
                let start_row = display.start.min(rope.max_point().row);
                let widest = display
                    .clone()
                    .map(|row| self.inline_edit_text_width(&rope.line(row), cx))
                    .fold(px(0.), Pixels::max);
                let first = self.inline_edit_text_width(&rope.line(start_row), cx);
                let view = self.inline_edit_box(
                    lines,
                    vec![None; count],
                    BoxStyle {
                        background: Some(editor_background),
                        border: Some(modified_border),
                        margin: (widest - first + px(ORIGINAL_END_PADDING), px(0.)),
                        padding: px(4.),
                    },
                    None,
                    cx,
                );
                widgets.push(self.add_content_widget(
                    ContentWidget::new(
                        PointUtf16::new(start_row, rope.line_len_utf16(start_row)),
                        vec![ContentWidgetPreference::Exact],
                        view.into(),
                    ),
                    cx,
                ));
            }
            Deletion => {
                let rows = &diff.original_line_range;
                if !rows.is_empty() {
                    let end = rows.end - 1;
                    decorations.push((
                        offset(PointUtf16::new(rows.start, 0))
                            ..offset(PointUtf16::new(end, rope.line_len_utf16(end))),
                        DecorationOptions {
                            is_whole_line: true,
                            block: Some(BlockDecoration {
                                border: colors.inline_edit_original_border(),
                                border_width: px(1.),
                                padding: [px(0.); 4],
                            }),
                            ..Default::default()
                        },
                    ));
                }
            }
            InsertionInline | Custom | JumpTo => {}
            InsertionMultiLine => {
                // Monaco `InlineEditsInsertionView`: ghost text at the
                // insertion, lines after the first in a zone.
                let Some((original, modified)) = diff.inner.first() else {
                    return (decorations, widgets);
                };
                let mut at = original.start;
                let mut text = diff.modified_text(modified);
                if at.column == 0
                    && at.row > 0
                    && rope.line_len_utf16(at.row) != 0
                    && text.ends_with('\n')
                    && !text.starts_with('\n')
                {
                    at = PointUtf16::new(at.row - 1, rope.line_len_utf16(at.row - 1));
                    text = format!("\n{}", &text[..text.len() - 1]);
                }
                let mut lines = text.split('\n');
                let first = lines.next().unwrap_or_default();
                if !first.is_empty() {
                    let at = offset(at);
                    decorations.push((
                        at..at,
                        DecorationOptions {
                            after: Some(InjectedText {
                                content: first.to_string().into(),
                                background: line_background,
                                ..Default::default()
                            }),
                            show_if_collapsed: true,
                            ..Default::default()
                        },
                    ));
                }
                let rest: Vec<Vec<Segment>> = lines
                    .map(|line| {
                        vec![Segment {
                            text: line.to_string(),
                            color: colors.foreground,
                            background: None,
                        }]
                    })
                    .collect();
                if !rest.is_empty() {
                    let count = rest.len();
                    let view = self.inline_edit_box(
                        rest,
                        vec![line_background; count],
                        BoxStyle {
                            background: None,
                            border: None,
                            margin: (px(0.), px(0.)),
                            padding: px(0.),
                        },
                        None,
                        cx,
                    );
                    widgets.push(self.add_view_zone(
                        ViewZone::new(at.row + 1, ZoneHeight::Lines(count as f32), view.into()),
                        cx,
                    ));
                }
            }
            Collapsed => {
                // Monaco puts the marker at the first replacement's start,
                // past what it keeps.
                let line_text = |row: u32| rope.line(row);
                let start = diff
                    .edits
                    .first()
                    .map_or(PointUtf16::default(), |(range, text)| {
                        super::ghost_text::single_text_remove_common_prefix(
                            &super::ghost_text::TextReplacement {
                                range: range.clone(),
                                text: text.clone(),
                            },
                            &line_text,
                        )
                        .range
                        .start
                    });
                let color = colors.inline_edit_gutter_indicator_primary()[1];
                let view = cx.new(|_| CollapsedMarker { color });
                widgets.push(self.add_content_widget(
                    ContentWidget::new(start, vec![ContentWidgetPreference::Exact], view.into()),
                    cx,
                ));
            }
        }
        (decorations, widgets)
    }
}

/// The part of `segments` (a line) between UTF-16 columns `from` and `to`.
fn clip_segments(segments: Vec<Segment>, from: u32, to: u32) -> Vec<Segment> {
    let mut column = 0u32;
    let mut out = Vec::new();
    for segment in segments {
        let mut text = String::new();
        for ch in segment.text.chars() {
            if column >= from && column < to {
                text.push(ch);
            }
            column += ch.len_utf16() as u32;
        }
        if !text.is_empty() {
            out.push(Segment { text, ..segment });
        }
    }
    out
}
