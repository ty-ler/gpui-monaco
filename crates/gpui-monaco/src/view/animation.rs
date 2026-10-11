//! The view's animations: the cursor's blinking styles (Monaco's
//! `cursor-smooth`, `cursor-phase` and `cursor-expand` CSS animations),
//! smooth caret moves (`cursorSmoothCaretAnimation`) and smooth scrolling
//! (`smoothScrolling`, Monaco's `SmoothScrollingOperation`) and inertial
//! scrolling (`inertialScroll`, from `scrollableElement.ts`).

use std::time::Duration;

use web_time::Instant;

use gpui::{Bounds, Pixels, Point, point, px, size};

use crate::options::{CursorBlinking, MinimapAutohide};

/// Monaco `ViewCursors.BLINK_INTERVAL`.
pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// The CSS blink animations run 20 half-second iterations after a blink
/// interval.
pub const BLINK_ANIMATION_END: Duration = Duration::from_millis(500 + 20 * 500);
/// Monaco's smooth scroll duration (`smoothScrollDuration`).
pub const SMOOTH_SCROLL_DURATION: Duration = Duration::from_millis(125);
/// Monaco's `.cursor-smooth-caret-animation` transition.
pub const CARET_DURATION: Duration = Duration::from_millis(80);
/// How often an inertial scroll moves (Monaco's `1000 / 60` ms timeout).
pub const INERTIAL_SCROLL_INTERVAL: Duration = Duration::from_micros(16_667);
/// Monaco `SCROLL_WHEEL_SENSITIVITY`: pixels per wheel delta unit.
const SCROLL_WHEEL_SENSITIVITY: f32 = 50.;

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
    pub minimap: MinimapFade,
    /// Inertial scrolling (`inertialScroll`).
    pub inertial: InertialScroll,
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
            minimap: MinimapFade::default(),
            inertial: InertialScroll::default(),
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

    /// Where a running smooth scroll is now, if one is running.
    pub fn animated_scroll_position(&self) -> Option<Point<Pixels>> {
        self.current_scroll(Instant::now())
    }

    /// A smooth scroll is running or about to start (Monaco
    /// `hasPendingScrollAnimation`).
    pub fn is_scrolling(&self) -> bool {
        self.scroll.is_some() || self.smooth_scroll_requested
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

/// Monaco `minimap.autohide`: the minimap fades in and out over 0.5s
/// (`.minimap-autohide-*` in `minimap.css`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MinimapFade {
    /// When the view last scrolled, for `scroll`.
    pub scrolled_at: Option<Instant>,
    /// Whether it's shown (or fading in), and since when.
    shown: bool,
    changed_at: Option<Instant>,
    /// The opacity the current transition started from.
    from: f32,
}

/// Monaco's `_hideDelayedScheduler` delay.
const MINIMAP_HIDE_DELAY: Duration = Duration::from_millis(500);
/// The CSS `transition: opacity 0.5s`.
const MINIMAP_FADE: Duration = Duration::from_millis(500);

impl MinimapFade {
    /// (opacity, whether it's still changing, whether it takes the mouse)
    /// for `autohide`, with the mouse over the minimap or not.
    pub fn update(&mut self, autohide: MinimapAutohide, mouse_over: bool) -> (f32, bool, bool) {
        let now = Instant::now();
        let (target, waiting, interactive) = match autohide {
            MinimapAutohide::None => return (1., false, true),
            MinimapAutohide::Mouseover => (mouse_over, false, true),
            MinimapAutohide::Scroll => {
                // Monaco checks again after the delay while the mouse is
                // over it; while hidden it doesn't take the mouse.
                if self.shown && mouse_over {
                    self.scrolled_at = Some(now);
                }
                let active = self
                    .scrolled_at
                    .is_some_and(|at| now < at + MINIMAP_HIDE_DELAY);
                (active, active, active)
            }
        };
        let current = self.opacity(now);
        if target != self.shown {
            self.from = current;
            self.shown = target;
            self.changed_at = Some(now);
        }
        let opacity = self.opacity(now);
        let target_opacity = if self.shown { 1. } else { 0. };
        (opacity, waiting || opacity != target_opacity, interactive)
    }

    fn opacity(&self, now: Instant) -> f32 {
        let target = if self.shown { 1. } else { 0. };
        let Some(changed_at) = self.changed_at else {
            return target;
        };
        let t = ((now - changed_at).as_secs_f32() / MINIMAP_FADE.as_secs_f32()).min(1.);
        self.from + (target - self.from) * ease(t)
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

/// Monaco's inertial scrolling (`inertialScroll`, the `_inertialSpeed` of
/// `AbstractScrollableElement` in `scrollableElement.ts`): each wheel event
/// sets a speed from its delta, and every 1/60s the view scrolls by it
/// while it decays by a tenth, so a flick keeps going after the wheel
/// events stop.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InertialScroll {
    /// In Monaco's wheel delta units (positive scrolls up and left).
    speed: Point<f32>,
}

impl InertialScroll {
    /// A wheel event scrolled by `delta` pixels (after sensitivity and the
    /// predominant axis; positive scrolls up and left). Returns whether
    /// the periodic scroll should start (it isn't running yet).
    pub fn on_wheel(&mut self, delta: Point<Pixels>) -> bool {
        if delta.x == px(0.) && delta.y == px(0.) {
            return false;
        }
        let start = !self.is_moving();
        // Monaco: `(delta < 0 ? -1 : 1) * Math.abs(delta) ** 1.02`.
        let speed = |delta: Pixels| {
            let units = f32::from(delta) / SCROLL_WHEEL_SENSITIVITY;
            units.signum() * units.abs().powf(1.02)
        };
        self.speed = point(speed(delta.x), speed(delta.y));
        start
    }

    /// Monaco `_periodicSync`: the pixels to scroll by now (subtracted from
    /// the scroll position), then the speed decays, stopping under 0.01.
    pub fn tick(&mut self) -> Option<Point<Pixels>> {
        if !self.is_moving() {
            return None;
        }
        let delta = point(px(self.speed.x * 100.), px(self.speed.y * 100.));
        let decay = |speed: f32| {
            let speed = speed * 0.9;
            if speed.abs() < 0.01 { 0. } else { speed }
        };
        self.speed = point(decay(self.speed.x), decay(self.speed.y));
        Some(delta)
    }

    /// Whether there's speed left.
    pub fn is_moving(&self) -> bool {
        self.speed.x != 0. || self.speed.y != 0.
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
    fn inertial_scroll_decays() {
        let mut inertial = InertialScroll::default();
        assert_eq!(inertial.tick(), None);
        assert!(!inertial.on_wheel(point(px(0.), px(0.))));
        // A 50px wheel delta is one Monaco unit: 100px on the first tick.
        assert!(inertial.on_wheel(point(px(0.), px(-50.))));
        assert_eq!(inertial.tick(), Some(point(px(0.), px(-100.))));
        assert_eq!(inertial.tick(), Some(point(px(0.), px(-90.))));
        // Another wheel event resets the speed without restarting.
        assert!(!inertial.on_wheel(point(px(0.), px(-50.))));
        assert_eq!(inertial.tick(), Some(point(px(0.), px(-100.))));
        // It stops once the speed is under 0.01 units.
        let ticks = std::iter::from_fn(|| inertial.tick()).count();
        assert_eq!(ticks, 43);
        assert!(!inertial.is_moving());
        // Larger deltas grow slightly faster than linearly.
        inertial.on_wheel(point(px(500.), px(0.)));
        let delta = inertial.tick().unwrap();
        assert!(delta.x > px(1000.) && delta.x < px(1060.));
    }

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
    fn minimap_autohide() {
        let mut fade = MinimapFade::default();
        assert_eq!(fade.update(MinimapAutohide::None, false), (1., false, true));
        // `mouseover`: hidden, then fading in while hovered.
        assert_eq!(
            fade.update(MinimapAutohide::Mouseover, false),
            (0., false, true)
        );
        let (opacity, fading, interactive) = fade.update(MinimapAutohide::Mouseover, true);
        assert!(opacity < 1. && fading && interactive);
        fade.changed_at = Some(Instant::now() - Duration::from_millis(600));
        assert_eq!(
            fade.update(MinimapAutohide::Mouseover, true),
            (1., false, true)
        );

        // `scroll`: hidden and click-through until the view scrolls.
        let mut fade = MinimapFade::default();
        assert_eq!(
            fade.update(MinimapAutohide::Scroll, true),
            (0., false, false)
        );
        fade.scrolled_at = Some(Instant::now());
        let (_, waiting, interactive) = fade.update(MinimapAutohide::Scroll, false);
        assert!(waiting && interactive);
        // Half a second later it fades out, unless the mouse is over it.
        fade.scrolled_at = Some(Instant::now() - Duration::from_millis(600));
        fade.changed_at = Some(Instant::now() - Duration::from_millis(600));
        let (opacity, _, interactive) = fade.update(MinimapAutohide::Scroll, true);
        assert!(opacity == 1. && interactive);
        fade.scrolled_at = Some(Instant::now() - Duration::from_millis(600));
        let (opacity, fading, interactive) = fade.update(MinimapAutohide::Scroll, false);
        assert!(opacity > 0. && fading && !interactive);
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
