//! The view's animations: the cursor's blinking styles (Monaco's
//! `cursor-smooth`, `cursor-phase` and `cursor-expand` CSS animations),
//! smooth caret moves (`cursorSmoothCaretAnimation`) and smooth scrolling
//! (`smoothScrolling`, Monaco's `SmoothScrollingOperation`).

use std::time::{Duration, Instant};

use gpui::{Bounds, Pixels, Point, point, px, size};

use crate::options::CursorBlinking;

/// Monaco `ViewCursors.BLINK_INTERVAL`.
pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// The CSS blink animations run 20 half-second iterations after a blink
/// interval.
pub const BLINK_ANIMATION_END: Duration = Duration::from_millis(500 + 20 * 500);
/// Monaco's smooth scroll duration (`smoothScrollDuration`).
pub const SMOOTH_SCROLL_DURATION: Duration = Duration::from_millis(125);
/// Monaco's `.cursor-smooth-caret-animation` transition.
pub const CARET_DURATION: Duration = Duration::from_millis(80);

/// A cubic Bézier timing function (CSS `cubic-bezier(x1, y1, x2, y2)`).
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let bezier = |a: f32, b: f32, t: f32| {
        3. * a * t * (1. - t) * (1. - t) + 3. * b * t * t * (1. - t) + t * t * t
    };
    // Solve x(t) = x by bisection, then evaluate y(t).
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if bezier(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier(y1, y2, (lo + hi) / 2.)
}

/// CSS `ease-in-out`.
fn ease_in_out(x: f32) -> f32 {
    cubic_bezier(0.42, 0., 0.58, 1., x.clamp(0., 1.))
}

/// CSS `ease` (the default transition timing).
fn ease(x: f32) -> f32 {
    cubic_bezier(0.25, 0.1, 0.25, 1., x.clamp(0., 1.))
}

/// Monaco `easeOutCubic`.
fn ease_out_cubic(x: f32) -> f32 {
    1. - (1. - x.clamp(0., 1.)).powi(3)
}

/// A keyframe animation holding 1 until `from`, easing to 0 by `to`.
fn keyframe(p: f32, from: f32, to: f32) -> f32 {
    if p <= from {
        1.
    } else if p >= to {
        0.
    } else {
        1. - ease_in_out((p - from) / (to - from))
    }
}

/// The cursor's opacity and vertical scale `elapsed` after its blink
/// restarted, for an animated blinking style (Monaco's keyframes, 0.5s
/// `alternate`, 20 iterations, after one blink interval).
pub fn cursor_appearance(style: CursorBlinking, elapsed: Duration) -> (f32, f32) {
    if elapsed < BLINK_INTERVAL || elapsed >= BLINK_ANIMATION_END {
        return (1., 1.);
    }
    let t = (elapsed - BLINK_INTERVAL).as_secs_f32() / BLINK_INTERVAL.as_secs_f32();
    let iteration = t.floor() as u32;
    let mut p = t.fract();
    if iteration % 2 == 1 {
        p = 1. - p;
    }
    match style {
        CursorBlinking::Smooth => (keyframe(p, 0.2, 0.6), 1.),
        CursorBlinking::Phase => (keyframe(p, 0.2, 0.9), 1.),
        CursorBlinking::Expand => (1., keyframe(p, 0.2, 0.8)),
        CursorBlinking::Blink | CursorBlinking::Solid => (1., 1.),
    }
}

#[derive(Clone, Copy, Debug)]
struct Transition<T> {
    from: T,
    to: T,
    start: Instant,
}

/// The editor's animation state.
#[derive(Debug)]
pub(crate) struct ViewAnimation {
    /// When the cursor last restarted blinking.
    pub blink_started: Instant,
    /// The next scroll change should animate (reveals, cursor moves,
    /// mouse wheels).
    pub smooth_scroll_requested: bool,
    scroll: Option<Transition<Point<Pixels>>>,
    /// The scroll position last shown.
    shown_scroll: Option<Point<Pixels>>,
    caret: Option<Transition<Bounds<Pixels>>>,
    /// The primary cursor's last target, in content coordinates.
    caret_target: Option<Bounds<Pixels>>,
    /// The next caret move jumps (Monaco pauses the transition when the
    /// cursor count changes, or for non-explicit moves with `explicit`).
    pub caret_jump: bool,
    /// The cursor count at the last cursor change.
    pub cursor_count: usize,
    /// Middle-click scrolling (`scrollOnMiddleClick`).
    pub middle_scroll: Option<MiddleScroll>,
    pub scrollbars: ScrollbarFade,
}

impl Default for ViewAnimation {
    fn default() -> Self {
        Self {
            blink_started: Instant::now(),
            smooth_scroll_requested: false,
            scroll: None,
            shown_scroll: None,
            caret: None,
            caret_target: None,
            caret_jump: false,
            cursor_count: 1,
            middle_scroll: None,
            scrollbars: ScrollbarFade::default(),
        }
    }
}

fn lerp(a: Pixels, b: Pixels, t: f32) -> Pixels {
    a + (b - a) * t
}

impl ViewAnimation {
    /// Stops a smooth scroll where it is.
    pub fn cancel_scroll(&mut self) {
        self.scroll = None;
    }

    /// The scroll position to show for `target`, animating there when
    /// smooth scrolling was requested; `true` while animating.
    pub fn scroll_position(
        &mut self,
        target: Point<Pixels>,
        smooth: bool,
    ) -> (Point<Pixels>, bool) {
        let now = Instant::now();
        let requested = std::mem::take(&mut self.smooth_scroll_requested);
        let current = self.current_scroll(now).or(self.shown_scroll);
        let retarget = self.scroll.is_none_or(|s| s.to != target);
        if retarget {
            self.scroll = match current {
                Some(from) if smooth && requested && from != target => Some(Transition {
                    from,
                    to: target,
                    start: now,
                }),
                // Continue towards a moved target only for smooth changes.
                Some(from) if smooth && self.scroll.is_some() && from != target => {
                    Some(Transition {
                        from,
                        to: target,
                        start: now,
                    })
                }
                _ => None,
            };
        }
        let shown = self.current_scroll(now).unwrap_or(target);
        if self
            .scroll
            .is_some_and(|s| now - s.start >= SMOOTH_SCROLL_DURATION)
        {
            self.scroll = None;
        }
        self.shown_scroll = Some(shown);
        (shown, self.scroll.is_some())
    }

    fn current_scroll(&self, now: Instant) -> Option<Point<Pixels>> {
        let s = self.scroll?;
        let t =
            ease_out_cubic((now - s.start).as_secs_f32() / SMOOTH_SCROLL_DURATION.as_secs_f32());
        Some(point(lerp(s.from.x, s.to.x, t), lerp(s.from.y, s.to.y, t)))
    }

    /// The primary cursor's bounds to show for `target` (content
    /// coordinates), sliding there over Monaco's 80ms when `enabled`;
    /// `true` while animating.
    pub fn caret(&mut self, target: Bounds<Pixels>, enabled: bool) -> (Bounds<Pixels>, bool) {
        let now = Instant::now();
        let jump = std::mem::take(&mut self.caret_jump);
        if !enabled {
            self.caret = None;
            self.caret_target = Some(target);
            return (target, false);
        }
        if self.caret_target != Some(target) {
            let shown = self.current_caret(now).or(self.caret_target);
            self.caret = match shown {
                Some(from) if !jump => Some(Transition {
                    from,
                    to: target,
                    start: now,
                }),
                _ => None,
            };
            self.caret_target = Some(target);
        }
        let shown = self.current_caret(now).unwrap_or(target);
        if self.caret.is_some_and(|c| now - c.start >= CARET_DURATION) {
            self.caret = None;
        }
        (shown, self.caret.is_some())
    }

    fn current_caret(&self, now: Instant) -> Option<Bounds<Pixels>> {
        let c = self.caret?;
        let t = ease((now - c.start).as_secs_f32() / CARET_DURATION.as_secs_f32());
        Some(Bounds::new(
            point(
                lerp(c.from.origin.x, c.to.origin.x, t),
                lerp(c.from.origin.y, c.to.origin.y, t),
            ),
            size(
                lerp(c.from.size.width, c.to.size.width, t),
                lerp(c.from.size.height, c.to.size.height, t),
            ),
        ))
    }
}

/// Monaco's `auto` scrollbar visibility: shown while the mouse is over the
/// editor or a scrollbar is dragged, and for a moment after scrolling,
/// then fading out.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScrollbarFade {
    pub mouse_over: bool,
    /// The mouse, for the slider's hover color.
    pub mouse_position: Option<Point<Pixels>>,
    pub revealed_at: Option<Instant>,
    pub left_at: Option<Instant>,
}

/// Monaco's `HIDE_TIMEOUT` before hiding, and its 800ms fade.
const SCROLLBAR_HIDE_TIMEOUT: Duration = Duration::from_millis(500);
const SCROLLBAR_FADE: Duration = Duration::from_millis(800);

impl ScrollbarFade {
    /// The sliders' opacity, and whether it's still changing.
    pub fn opacity(&self, dragging: bool) -> (f32, bool) {
        if self.mouse_over || dragging {
            return (1., false);
        }
        let hide_from = [
            self.revealed_at.map(|t| t + SCROLLBAR_HIDE_TIMEOUT),
            self.left_at,
        ]
        .into_iter()
        .flatten()
        .max();
        let Some(hide_from) = hide_from else {
            return (0., false);
        };
        let now = Instant::now();
        if now < hide_from {
            return (1., true);
        }
        let t = (now - hide_from).as_secs_f32() / SCROLLBAR_FADE.as_secs_f32();
        ((1. - t).max(0.), t < 1.)
    }
}

/// A middle-click scrolling session (Monaco `MiddleScrollController`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MiddleScroll {
    /// Where the middle button went down (window coordinates).
    pub anchor: Point<Pixels>,
    /// The mouse now.
    pub mouse: Point<Pixels>,
    pub last_frame: Instant,
    pub did_scroll: bool,
}

impl MiddleScroll {
    pub fn new(anchor: Point<Pixels>) -> Self {
        Self {
            anchor,
            mouse: anchor,
            last_frame: Instant::now(),
            did_scroll: false,
        }
    }

    /// This frame's scroll: the mouse's offset from the anchor, past a
    /// 5px threshold, every 32ms.
    pub fn frame_delta(&mut self) -> Point<Pixels> {
        let now = Instant::now();
        let factor = (now - self.last_frame).as_secs_f32() * 1000. / 32.;
        self.last_frame = now;
        let threshold = |v: Pixels| {
            if v.abs() <= px(5.) {
                px(0.)
            } else {
                v - px(5.) * v.signum()
            }
        };
        let delta = point(
            threshold(self.mouse.x - self.anchor.x) * factor,
            threshold(self.mouse.y - self.anchor.y) * factor,
        );
        if delta != point(px(0.), px(0.)) {
            self.did_scroll = true;
        }
        delta
    }
}

/// Monaco's selection corner rounding (`roundedSelection`): which corners
/// of a line's selection rectangle are outer corners, given the rows
/// above and below as `(left, right)`.
pub fn rounded_corners(
    current: (Pixels, Pixels),
    above: Option<(Pixels, Pixels)>,
    below: Option<(Pixels, Pixels)>,
) -> [bool; 4] {
    let (left, right) = current;
    let eps = px(0.5);
    // top-left, top-right, bottom-right, bottom-left
    [
        above.is_none_or(|(l, r)| l > left + eps || r < left),
        above.is_none_or(|(_, r)| r < right - eps),
        below.is_none_or(|(_, r)| r < right - eps),
        below.is_none_or(|(l, r)| l > left + eps || r < left),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_keyframes() {
        let at = |ms: u64| cursor_appearance(CursorBlinking::Smooth, Duration::from_millis(ms));
        // Solid for the first interval.
        assert_eq!(at(100), (1., 1.));
        // 0-20%: visible; 60%+: hidden.
        assert_eq!(at(500 + 50).0, 1.);
        assert_eq!(at(500 + 400).0, 0.);
        // Alternating: the second iteration runs backwards.
        assert_eq!(at(1000 + 450).0, 1.);
        assert!(at(500 + 200).0 > 0. && at(500 + 200).0 < 1.);
        // Expand scales instead.
        let expand = cursor_appearance(CursorBlinking::Expand, Duration::from_millis(500 + 450));
        assert_eq!(expand, (1., 0.));
        // Done after 20 iterations.
        assert_eq!(at(11_000), (1., 1.));
    }

    #[test]
    fn timing_functions() {
        assert!((ease_in_out(0.5) - 0.5).abs() < 0.01);
        assert!(ease(0.5) > 0.7);
        assert_eq!(ease_out_cubic(1.), 1.);
    }

    #[test]
    fn auto_scrollbars_fade() {
        let mut fade = ScrollbarFade::default();
        // Never revealed: hidden.
        assert_eq!(fade.opacity(false), (0., false));
        fade.mouse_over = true;
        assert_eq!(fade.opacity(false), (1., false));
        fade.mouse_over = false;
        // Just scrolled: shown, then fading.
        fade.revealed_at = Some(Instant::now());
        assert_eq!(fade.opacity(false), (1., true));
        fade.revealed_at = Some(Instant::now() - Duration::from_millis(900));
        let (opacity, fading) = fade.opacity(false);
        assert!(opacity > 0. && opacity < 1. && fading);
        fade.revealed_at = Some(Instant::now() - Duration::from_millis(2000));
        assert_eq!(fade.opacity(false), (0., false));
        // Dragging keeps it.
        assert_eq!(fade.opacity(true), (1., false));
    }

    #[test]
    fn selection_corners() {
        let px2 = |a: f32, b: f32| (px(a), px(b));
        // A single line: all round.
        assert_eq!(rounded_corners(px2(0., 10.), None, None), [true; 4]);
        // A middle line sticking out past the one above on both sides.
        assert_eq!(
            rounded_corners(px2(0., 20.), Some(px2(5., 10.)), Some(px2(0., 20.))),
            [true, true, false, false]
        );
    }
}
