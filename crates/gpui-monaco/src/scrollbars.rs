//! Monaco `ScrollableElement`'s scrollbars over a GPUI scroll area (an
//! element tracking a [`ScrollHandle`]), for widgets whose content
//! scrolls: the hover card and the marker zone. Each scrollbar shows while
//! its content doesn't fit and the mouse is over the area (Monaco's
//! `ScrollbarVisibility.Auto`); its slider can be dragged, and a click on
//! the track centers the slider there.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, AppContext as _, InteractiveElement as _, IntoElement as _, MouseButton,
    ParentElement as _, Pixels, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, point, px,
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
