//! Monaco `Sash`'s hover feedback (`base/browser/ui/sash/sash.ts`), for the
//! sashes that show `sash.hoverBorder`.

use std::time::Duration;

use gpui::{Context, Task};

/// Monaco's sash `hoverDelay` (`globalHoverDelay`, 300ms): how long the
/// mouse rests on a sash before `sash.hoverBorder` shows.
pub(crate) const SASH_HOVER_DELAY: Duration = Duration::from_millis(300);

/// Which of a view's sashes shows `sash.hoverBorder` for the mouse resting
/// on it (Monaco `Sash.onMouseEnter`'s `hoverDelayer`, which adds the
/// `hover` class after [`SASH_HOVER_DELAY`], and `onMouseLeave`, which
/// cancels it and removes the class at once). Nothing repaints while
/// waiting: the delay's timer notifies the view once, when it ends.
/// Dragging (Monaco's `active` class) is the view's to show, at once.
#[derive(Default)]
pub(crate) struct SashHover {
    hovered: Option<&'static str>,
    shown: bool,
    delay: Option<Task<()>>,
}

impl SashHover {
    /// Whether sash `id` shows `sash.hoverBorder` for the hover.
    pub fn shows(&self, id: &'static str) -> bool {
        self.shown && self.hovered == Some(id)
    }

    /// The mouse entered (`hovered`) or left sash `id` of view `V`, whose
    /// `SashHover` is `state(view)`.
    pub fn set_hovered<V: 'static>(
        &mut self,
        id: &'static str,
        hovered: bool,
        state: fn(&mut V) -> &mut SashHover,
        cx: &mut Context<V>,
    ) {
        if hovered {
            if self.hovered == Some(id) {
                return;
            }
            let was_shown = std::mem::replace(&mut self.shown, false);
            self.hovered = Some(id);
            self.delay = Some(cx.spawn(async move |view, cx| {
                cx.background_executor().timer(SASH_HOVER_DELAY).await;
                view.update(cx, |view, cx| {
                    let hover = state(view);
                    if hover.hovered == Some(id) {
                        hover.shown = true;
                        cx.notify();
                    }
                })
                .ok();
            }));
            if was_shown {
                cx.notify();
            }
        } else if self.hovered == Some(id) {
            self.hovered = None;
            self.delay = None;
            if std::mem::take(&mut self.shown) {
                cx.notify();
            }
        }
    }
}
