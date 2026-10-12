//! Monaco's pointer capture during drags (`GlobalPointerMoveMonitor` calls
//! `setPointerCapture` on the element a drag starts on): until the button is
//! released, the drag gets the mouse moves and the release anywhere in the
//! window, and the pointer stays that element's cursor over the whole
//! window (a sash keeps its resize cursor, a scrollbar slider the arrow).
//!
//! A drag renders [`capture`] while it goes on; it stops being painted, and
//! the pointer goes back to each area's own, once the drag ends: on the
//! release, on a move without the button (released where the window didn't
//! see it, like a lost capture), or when the widget closes.
//!
//! Drags made with GPUI's `on_drag` get the same from GPUI: the dragged
//! element's cursor is the window's while the drag is active.

use std::rc::Rc;

use gpui::{
    Context, CursorStyle, DispatchPhase, IntoElement, MouseMoveEvent, MouseUpEvent, Styled as _,
    WeakEntity, Window, canvas,
};

/// Shows `cursor` over the whole window, over every hitbox's cursor (what
/// Monaco's pointer capture does). Call during paint.
pub(crate) fn paint_captured_cursor(cursor: Option<CursorStyle>, window: &mut Window) {
    if let Some(cursor) = cursor {
        window.set_window_cursor_style(cursor);
    }
}

/// A drag's pointer capture, rendered while the drag goes on: `cursor`
/// over the whole window, `on_move` for every mouse move in the window
/// with a button pressed, and `on_release` for the release (or a move
/// without a button, the release having been missed).
pub(crate) fn capture<T: 'static>(
    entity: WeakEntity<T>,
    cursor: Option<CursorStyle>,
    on_move: impl Fn(&mut T, &MouseMoveEvent, &mut Context<T>) + 'static,
    on_release: impl Fn(&mut T, &mut Context<T>) + 'static,
) -> impl IntoElement {
    let on_move = Rc::new(on_move);
    let on_release = Rc::new(on_release);
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            paint_captured_cursor(cursor, window);
            let (moves, released) = (entity.clone(), on_release.clone());
            let on_move = on_move.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                moves
                    .update(cx, |this, cx| {
                        if event.pressed_button.is_some() {
                            on_move(this, event, cx);
                        } else {
                            released(this, cx);
                        }
                    })
                    .ok();
            });
            let (ups, on_release) = (entity.clone(), on_release.clone());
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    ups.update(cx, |this, cx| on_release(this, cx)).ok();
                }
            });
        },
    )
    .absolute()
    .size_0()
}
