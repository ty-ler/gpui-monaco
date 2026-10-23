//! Monaco `ScrollableElement`'s scrollbars over a GPUI scroll area (an
//! element tracking a [`ScrollHandle`]), for widgets whose content
//! scrolls: the hover card and the marker zone. Each scrollbar shows while
//! its content doesn't fit and the mouse is over the area (Monaco's
//! `ScrollbarVisibility.Auto`); its slider can be dragged, and a click on
//! the track centers the slider there.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use web_time::Instant;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, EntityId, InteractiveElement as _, IntoElement as _,
    MouseButton, ParentElement as _, Pixels, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, canvas, div, point, px,
};

use crate::theme::EditorColors;

/// Monaco `ScrollbarState`'s `MINIMUM_SLIDER_SIZE`.
const MIN_SLIDER: f32 = 20.;

/// A slider being dragged: its area's scroll handle, the axis, and where in
/// the slider it was grabbed.
struct SliderDrag {
    handle: ScrollHandle,
    vertical: bool,
    grab: Rc<Cell<Pixels>>,
}

/// One axis of a scroll area: the visible length, the content's, and the
/// scroll position (all positive).
#[derive(Clone, Copy)]
struct Axis {
    visible: Pixels,
    content: Pixels,
    position: Pixels,
}

impl Axis {
    fn of(handle: &ScrollHandle, vertical: bool) -> Self {
        let bounds = handle.bounds();
        let max = handle.max_offset();
        let offset = handle.offset();
        let (visible, max, offset) = if vertical {
            (bounds.size.height, max.y, offset.y)
        } else {
            (bounds.size.width, max.x, offset.x)
        };
        Axis {
            visible,
            content: visible + max,
            position: -offset,
        }
    }

    /// Monaco `ScrollbarState`: the slider's offset and length along a
    /// track `track` long, or `None` when everything is visible.
    fn slider(self, track: Pixels) -> Option<(Pixels, Pixels)> {
        if self.content <= self.visible || track <= px(0.) {
            return None;
        }
        let length = (track * (self.visible / self.content))
            .floor()
            .max(px(MIN_SLIDER))
            .min(track);
        let ratio = (track - length) / (self.content - self.visible);
        Some((self.position * ratio, length))
    }

    /// The scroll position putting the slider's start at `start` (Monaco
    /// `getDesiredScrollPositionFromOffset`).
    fn position_for_slider(self, track: Pixels, start: Pixels) -> Pixels {
        let Some((_, length)) = self.slider(track) else {
            return px(0.);
        };
        let ratio = (track - length) / (self.content - self.visible);
        if ratio <= 0. {
            return px(0.);
        }
        (start / ratio).clamp(px(0.), self.content - self.visible)
    }
}

/// Scrolls `handle`'s area so its slider starts at `start` along a track
/// `track` long.
fn scroll_to_slider(handle: &ScrollHandle, vertical: bool, track: Pixels, start: Pixels) {
    let position = Axis::of(handle, vertical).position_for_slider(track, start);
    let offset = handle.offset();
    handle.set_offset(if vertical {
        point(offset.x, -position)
    } else {
        point(-position, offset.y)
    });
}

/// The scrollbars of `handle`'s area, `size` thick, to put in a `relative`
/// element covering the area (inset by `inset`, e.g. a border) and named
/// `group` (for its hover). Monaco's slider is `scrollbarSlider.background`,
/// `.hoverBackground` under the mouse and `.activeBackground` while
/// dragged.
pub(crate) fn scrollbars(
    id: &'static str,
    group: SharedString,
    handle: &ScrollHandle,
    size: Pixels,
    inset: Pixels,
    colors: &EditorColors,
) -> Vec<AnyElement> {
    let bounds = handle.bounds();
    let vertical = Axis::of(handle, true);
    let horizontal = Axis::of(handle, false);
    let shows = |axis: Axis| axis.content > axis.visible;
    let (show_vertical, show_horizontal) = (shows(vertical), shows(horizontal));
    let mut bars = Vec::new();
    for (is_vertical, axis, shown) in [
        (true, vertical, show_vertical),
        (false, horizontal, show_horizontal),
    ] {
        if !shown {
            continue;
        }
        // Each bar stops short of the other's corner.
        let corner = if (is_vertical && show_horizontal) || (!is_vertical && show_vertical) {
            size
        } else {
            px(0.)
        };
        let track = if is_vertical {
            bounds.size.height
        } else {
            bounds.size.width
        } - inset * 2.
            - corner;
        let Some((start, length)) = axis.slider(track) else {
            continue;
        };
        let track_start = if is_vertical {
            bounds.top()
        } else {
            bounds.left()
        } + inset;
        bars.push(scrollbar(ScrollbarLayout {
            id,
            group: group.clone(),
            handle: handle.clone(),
            vertical: is_vertical,
            size,
            inset,
            corner,
            track,
            track_start,
            start,
            length,
            colors,
        }));
    }
    bars
}

/// Where one scrollbar goes.
struct ScrollbarLayout<'a> {
    id: &'static str,
    group: SharedString,
    handle: ScrollHandle,
    vertical: bool,
    size: Pixels,
    inset: Pixels,
    corner: Pixels,
    track: Pixels,
    track_start: Pixels,
    start: Pixels,
    length: Pixels,
    colors: &'a EditorColors,
}

fn scrollbar(layout: ScrollbarLayout) -> AnyElement {
    let ScrollbarLayout {
        id,
        group,
        handle,
        vertical,
        size,
        inset,
        corner,
        track,
        track_start,
        start,
        length,
        colors,
    } = layout;
    let axis = if vertical { "vertical" } else { "horizontal" };
    let track_id: SharedString = format!("{id}-{axis}-scrollbar").into();
    let slider_id: SharedString = format!("{id}-{axis}-slider").into();
    let selector = slider_id.clone();
    let grab = Rc::new(Cell::new(px(0.)));
    let click_handle = handle.clone();
    let hover = colors.scrollbar_slider_hover;
    let active = colors.scrollbar_slider_active;
    let slider = div()
        .id(slider_id)
        .debug_selector(move || selector.to_string())
        .absolute()
        .bg(colors.scrollbar_slider)
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(active))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_drag(
            SliderDrag {
                handle,
                vertical,
                grab: grab.clone(),
            },
            move |drag, offset, _, cx| {
                drag.grab
                    .set(if drag.vertical { offset.y } else { offset.x });
                cx.new(|_| gpui::Empty)
            },
        )
        .on_drag_move(
            move |event: &gpui::DragMoveEvent<SliderDrag>, window: &mut Window, cx| {
                let drag = event.drag(cx);
                let mouse = if drag.vertical {
                    event.event.position.y
                } else {
                    event.event.position.x
                };
                let start = mouse - track_start - drag.grab.get();
                scroll_to_slider(&drag.handle, drag.vertical, track, start);
                window.refresh();
            },
        );
    let slider = if vertical {
        slider.left_0().w_full().top(start).h(length)
    } else {
        slider.top_0().h_full().left(start).w(length)
    };
    let bar = div()
        .id(track_id)
        .absolute()
        .opacity(0.)
        .group_hover(group, |style| style.opacity(1.))
        // Monaco centers the slider where the track is clicked.
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            cx.stop_propagation();
            let mouse = if vertical {
                event.position.y
            } else {
                event.position.x
            };
            scroll_to_slider(
                &click_handle,
                vertical,
                track,
                mouse - track_start - length / 2.,
            );
            window.refresh();
        })
        .child(slider);
    let bar = if vertical {
        bar.top(inset).bottom(inset + corner).right(inset).w(size)
    } else {
        bar.left(inset).right(inset + corner).bottom(inset).h(size)
    };
    bar.into_any_element()
}

/// What a scroll area's scrollbars are laid out for: its bounds and how far
/// it scrolls, as of the last frame.
pub(crate) type Extent = (gpui::Bounds<Pixels>, gpui::Point<Pixels>);

/// `handle`'s area's [`Extent`] as of the last frame.
pub(crate) fn extent(handle: &ScrollHandle) -> Extent {
    (handle.bounds(), handle.max_offset())
}

/// Called while painting: when the frame changed `handle`'s area from
/// `rendered`, `view` renders again so its scrollbars follow.
pub(crate) fn follow_extent<T: 'static>(
    handle: &ScrollHandle,
    rendered: Extent,
    view: gpui::WeakEntity<T>,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    if extent(handle) != rendered {
        window.defer(cx, move |_, cx| {
            view.update(cx, |_, cx| cx.notify()).ok();
        });
    }
}

/// How long Monaco's `ScrollbarVisibility.Auto` scrollbars stay after
/// scrolling (`HIDE_TIMEOUT`).
pub(crate) const SCROLLBAR_HIDE: Duration = Duration::from_millis(500);
/// `.monaco-scrollable-element > .visible { transition: opacity 100ms }`.
const SCROLLBAR_FADE_IN: Duration = Duration::from_millis(100);
/// `.monaco-scrollable-element > .invisible.fade { transition: opacity
/// 800ms }`.
const SCROLLBAR_FADE_OUT: Duration = Duration::from_millis(800);
/// The mouse is over the scrollable element (for [`ScrollbarFade::hover`]).
pub(crate) const HOVER_AREA: u8 = 1;
/// The mouse is over the slider, which takes the mouse from the area
/// under it.
pub(crate) const HOVER_SLIDER: u8 = 2;

/// Monaco `ScrollableElement`'s `ScrollbarVisibility.Auto` and its
/// `ScrollbarVisibilityController`: a scrollbar shows while the mouse is
/// over its `.monaco-scrollable-element` or its slider is dragged, and
/// for `HIDE_TIMEOUT` after scrolling. It fades in over 100ms (`.visible`)
/// and out over 800ms (`.invisible.fade`), from wherever it was.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScrollbarFade {
    /// What the mouse is over ([`HOVER_AREA`], [`HOVER_SLIDER`]); Monaco's
    /// `_mouseIsOver`.
    pub(crate) hover: u8,
    /// When it last scrolled (Monaco `_reveal`, then `_scheduleHide`).
    pub(crate) revealed_at: Option<Instant>,
    /// Whether it's showing (or fading in), since when, and its opacity
    /// then.
    visible: bool,
    changed_at: Option<Instant>,
    from: f32,
}

impl ScrollbarFade {
    /// Whether it should show at `now`, the mouse aside: dragged, or just
    /// scrolled.
    pub(crate) fn revealed(&self, dragging: bool, now: Instant) -> bool {
        dragging || self.revealed_at.is_some_and(|at| now < at + SCROLLBAR_HIDE)
    }

    /// The opacity at `now` of the transition under way.
    fn current(&self, now: Instant) -> f32 {
        let Some(at) = self.changed_at else {
            return if self.visible { 1. } else { 0. };
        };
        let elapsed = now.saturating_duration_since(at).as_secs_f32();
        if self.visible {
            let t = (elapsed / SCROLLBAR_FADE_IN.as_secs_f32()).min(1.);
            self.from + (1. - self.from) * t
        } else {
            let t = (elapsed / SCROLLBAR_FADE_OUT.as_secs_f32()).min(1.);
            self.from * (1. - t)
        }
    }

    /// The opacity at `now`, starting a fade when whether it should show
    /// changed, and whether it's still fading.
    pub(crate) fn opacity(&mut self, dragging: bool, now: Instant) -> (f32, bool) {
        let show = self.hover != 0 || self.revealed(dragging, now);
        if show != self.visible {
            self.from = self.current(now);
            self.visible = show;
            self.changed_at = Some(now);
        }
        let opacity = self.current(now);
        let target = if self.visible { 1. } else { 0. };
        (opacity, opacity != target)
    }
}

/// The vertical scrollbar of a list of rows that its view scrolls itself
/// (the peek views' trees), as Monaco's `ScrollableElement` draws it with
/// `ScrollbarVisibility.Auto`: shown while the mouse is over the list, its
/// slider is dragged, or for a while after scrolling, fading in and out
/// (see [`ScrollbarFade`]).
#[derive(Default)]
pub(crate) struct ListScrollbar {
    fade: ScrollbarFade,
    hide: Option<Task<()>>,
    dragging: bool,
    track_top: Rc<Cell<Pixels>>,
}

/// A [`ListScrollbar`]'s slider being dragged by the view `view`, grabbed
/// `grab` below its top.
struct ListSliderDrag {
    view: EntityId,
    grab: Rc<Cell<Pixels>>,
}

impl ListScrollbar {
    /// The mouse entered or left the list.
    pub(crate) fn set_hovered<V: 'static>(&mut self, hovered: bool, cx: &mut Context<V>) {
        let before = self.fade.hover;
        if hovered {
            self.fade.hover |= HOVER_AREA;
        } else {
            self.fade.hover &= !HOVER_AREA;
        }
        if self.fade.hover != before {
            cx.notify();
        }
    }

    /// The list scrolled: shows the scrollbar for `HIDE_TIMEOUT` (Monaco
    /// `_reveal`, then `_scheduleHide`).
    pub(crate) fn reveal<V: 'static>(&mut self, cx: &mut Context<V>) {
        self.fade.revealed_at = Some(cx.background_executor().now());
        self.hide = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SCROLLBAR_HIDE).await;
            // Rendering starts the fade out.
            this.update(cx, |_, cx| cx.notify()).ok();
        }));
    }

    /// The scrollbar's opacity for this frame, asking for the next frame
    /// while it fades.
    pub(crate) fn opacity<V: 'static>(&mut self, window: &mut Window, cx: &mut Context<V>) -> f32 {
        if !cx.has_active_drag() {
            self.dragging = false;
        }
        let now = cx.background_executor().now();
        let (opacity, fading) = self.fade.opacity(self.dragging, now);
        if fading {
            window.request_animation_frame();
        }
        opacity
    }

    /// The scrollbar along a track `visible` tall at the list's right edge
    /// (`width` wide) for `content` scrolled by `position`, at `opacity`,
    /// or `None` when everything fits. Clicking the track centers the
    /// slider there; dragging the slider scrolls. `scroll_to` sets the
    /// list's scroll position and `state` finds this scrollbar in the view.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render<V: 'static>(
        &self,
        id: &'static str,
        width: Pixels,
        visible: Pixels,
        content: Pixels,
        position: Pixels,
        opacity: f32,
        colors: &EditorColors,
        state: fn(&mut V) -> &mut ListScrollbar,
        scroll_to: fn(&mut V, Pixels, &mut Context<V>),
        cx: &mut Context<V>,
    ) -> Option<AnyElement> {
        if content <= visible || visible <= px(0.) {
            return None;
        }
        let length = (visible * (visible / content))
            .max(px(MIN_SLIDER))
            .min(visible);
        let room = (visible - length).max(px(1.));
        let top = room * (position / (content - visible)).clamp(0., 1.);
        let to_position =
            move |slider_top: Pixels| (content - visible) * (slider_top / room).clamp(0., 1.);
        let view = cx.entity_id();
        let grab = Rc::new(Cell::new(px(0.)));
        let track_top = self.track_top.clone();
        let click_top = self.track_top.clone();
        let (track_id, slider_id): (SharedString, SharedString) = (
            format!("{id}-scrollbar").into(),
            format!("{id}-slider").into(),
        );
        let slider_selector = slider_id.clone();
        let track_selector = track_id.clone();
        let hover = colors.scrollbar_slider_hover;
        let active = colors.scrollbar_slider_active;
        let dragging = self.dragging;
        Some(
            div()
                .id(track_id)
                .debug_selector(move || track_selector.to_string())
                .absolute()
                .top_0()
                .right_0()
                .w(width)
                .h(visible)
                .opacity(opacity)
                // Monaco's `.invisible` scrollbar lets the mouse through.
                .when(opacity <= 0., |this| this.invisible())
                // GPUI keeps a drag's cursor over the whole window
                // (Monaco's pointer capture): the slider's arrow.
                .cursor(gpui::CursorStyle::Arrow)
                .child(
                    canvas(
                        move |bounds, _, _| track_top.set(bounds.top()),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        // Monaco centers the slider where the track is
                        // clicked.
                        let slider_top = event.position.y - click_top.get() - length / 2.;
                        scroll_to(this, to_position(slider_top), cx);
                        state(this).reveal(cx);
                    }),
                )
                .on_drag_move(cx.listener(
                    move |this, event: &gpui::DragMoveEvent<ListSliderDrag>, _, cx| {
                        let drag = event.drag(cx);
                        if drag.view != view {
                            return;
                        }
                        let slider_top =
                            event.event.position.y - event.bounds.top() - drag.grab.get();
                        state(this).dragging = true;
                        scroll_to(this, to_position(slider_top), cx);
                        state(this).reveal(cx);
                    },
                ))
                .child(
                    div()
                        .id(slider_id)
                        .debug_selector(move || slider_selector.to_string())
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(top)
                        .h(length)
                        .map(|slider| {
                            if dragging {
                                slider.bg(active)
                            } else {
                                slider
                                    .bg(colors.scrollbar_slider)
                                    .hover(move |style| style.bg(hover))
                            }
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_drag(ListSliderDrag { view, grab }, |drag, offset, _, cx| {
                            drag.grab.set(offset.y);
                            cx.new(|_| gpui::Empty)
                        }),
                )
                .into_any_element(),
        )
    }
}

/// Monaco's scrollbar shadow along the top of a list scrolled down
/// (`scrollbar.shadow`).
pub(crate) fn list_top_shadow(colors: &EditorColors) -> gpui::Div {
    div()
        .absolute()
        .top(px(-6.))
        .left_0()
        .right_0()
        .h(px(6.))
        .shadow(vec![gpui::BoxShadow {
            color: colors.scrollbar_shadow,
            offset: point(px(0.), px(0.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
            inset: false,
        }])
}
