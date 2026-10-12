//! Editor font zoom (Monaco `editorZoom.ts` and `contrib/fontZoom`): one
//! zoom level shared by every editor, scaling the font size and line
//! height by 10% per level.

use std::time::Duration;

use web_time::Instant;

use gpui::{
    App, Context, Global, Modifiers, PinchEvent, Pixels, ScrollDelta, ScrollWheelEvent, px,
};

use crate::editor::CodeEditor;
use crate::options::EditorOptions;

/// Monaco clamps the zoom level to `[-5, 20]`.
const MIN_ZOOM_LEVEL: f32 = -5.;
const MAX_ZOOM_LEVEL: f32 = 20.;

/// Monaco `EditorZoom`.
#[derive(Default)]
pub(crate) struct EditorZoom {
    level: f32,
}

impl Global for EditorZoom {}

/// The editor zoom level (Monaco `EditorZoom.getZoomLevel`).
pub fn editor_zoom_level(cx: &App) -> f32 {
    cx.try_global::<EditorZoom>().map_or(0., |zoom| zoom.level)
}

/// Sets the zoom level of every editor, clamped to `[-5, 20]` (Monaco
/// `EditorZoom.setZoomLevel`).
pub fn set_editor_zoom_level(level: f32, cx: &mut App) {
    let level = level.clamp(MIN_ZOOM_LEVEL, MAX_ZOOM_LEVEL);
    if editor_zoom_level(cx) != level {
        cx.set_global(EditorZoom { level });
    }
}

/// Monaco `editor.remeasureFonts`: does nothing. Monaco caches font
/// measurements and must clear them after web fonts load; GPUI lays text
/// out with the current fonts every frame, so editors already use them.
pub fn remeasure_fonts(_cx: &mut App) {}

/// Monaco `BareFontInfo._create`: the font size and line height grow by
/// 10% per zoom level, after [`EditorOptions::validated`].
///
/// The zoomed options keep the line height resolved, in pixels, in
/// `line_height`: Monaco rounds the line height before zooming and not
/// after, so at zoom level 1 an 18px line is 19.8px. Read it with
/// [`effective_line_height`], not [`EditorOptions::resolved_line_height`],
/// which would round it again (and read values under 8 as multiples of the
/// font size).
pub(crate) fn zoomed_options(options: &EditorOptions, level: f32) -> EditorOptions {
    let mut zoomed = options.validated();
    let multiplier = 1. + level * 0.1;
    zoomed.line_height = Some(zoomed.resolved_line_height() * multiplier);
    if level != 0. {
        zoomed.font_size *= multiplier;
    }
    zoomed
}

/// The line height of options from [`zoomed_options`] (Monaco
/// `fontInfo.lineHeight`), unrounded.
pub(crate) fn effective_line_height(zoomed: &EditorOptions) -> Pixels {
    zoomed
        .line_height
        .unwrap_or_else(|| zoomed.resolved_line_height())
}

/// A trackpad zoom gesture in progress (Monaco treats wheel events within
/// 50ms of each other as one gesture).
pub(crate) struct ZoomGesture {
    last: Instant,
    start_level: f32,
    has_modifiers: bool,
    accumulated: f32,
}

/// Monaco `hasMouseWheelZoomModifiers`: Cmd or Ctrl on macOS (pinching
/// sets Ctrl), Ctrl elsewhere; never with Shift or Alt.
fn has_zoom_modifiers(m: &Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        (m.platform || m.control) && !m.shift && !m.alt
    } else {
        m.control && !m.platform && !m.shift && !m.alt
    }
}

impl CodeEditor {
    /// Monaco `mouseWheelZoom`: zooms with the wheel while the modifiers
    /// are held. Returns whether it zoomed (and the wheel shouldn't scroll).
    pub(crate) fn mouse_wheel_zoom(
        &mut self,
        event: &ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.options.mouse_wheel_zoom {
            return false;
        }
        match event.delta {
            // A physical wheel steps one level per event.
            ScrollDelta::Lines(delta) => {
                if !has_zoom_modifiers(&event.modifiers) {
                    return false;
                }
                let step = if delta.y > 0. { 1. } else { -1. };
                set_editor_zoom_level(editor_zoom_level(cx) + step, cx);
                true
            }
            // Trackpads accumulate the gesture's delta, a level per five
            // wheel notches (Monaco's 120 pixels).
            ScrollDelta::Pixels(delta) => self.zoom_gesture_delta(
                f32::from(delta.y) / f32::from(px(120.)),
                &event.modifiers,
                cx,
            ),
        }
    }

    /// Monaco `mouseWheelZoom` for a trackpad pinch. Browsers deliver a
    /// pinch as a Ctrl+wheel event of one wheel notch per update (Chromium's
    /// synthetic pinch wheel has `wheelDeltaY` ±120, positive when spreading
    /// the fingers), which Monaco accumulates like any trackpad gesture: a
    /// zoom level per five updates. GPUI delivers the pinch itself, so this
    /// adds Ctrl to its modifiers and accumulates the same notches.
    pub(crate) fn mouse_pinch_zoom(&mut self, event: &PinchEvent, cx: &mut Context<Self>) -> bool {
        if !self.options.mouse_wheel_zoom || event.delta == 0. {
            return false;
        }
        let modifiers = Modifiers {
            control: true,
            ..event.modifiers
        };
        self.zoom_gesture_delta(event.delta.signum(), &modifiers, cx)
    }

    /// Adds a trackpad wheel delta (in wheel notches) to the zoom gesture,
    /// starting a new gesture after 50ms without events (Monaco
    /// `onMouseWheel`'s `gestureAccumulatedDelta`). Returns whether it
    /// zoomed: whether the gesture began with the zoom modifiers held.
    fn zoom_gesture_delta(
        &mut self,
        delta: f32,
        modifiers: &Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        let now = Instant::now();
        let gesture = match &mut self.zoom_gesture {
            Some(gesture) if now - gesture.last <= Duration::from_millis(50) => gesture,
            gesture => gesture.insert(ZoomGesture {
                last: now,
                start_level: editor_zoom_level(cx),
                has_modifiers: has_zoom_modifiers(modifiers),
                accumulated: 0.,
            }),
        };
        gesture.last = now;
        gesture.accumulated += delta;
        if !gesture.has_modifiers {
            return false;
        }
        let level = gesture.start_level + gesture.accumulated / 5.;
        set_editor_zoom_level(level, cx);
        true
    }

    /// Re-applies the zoom after it changes.
    pub(crate) fn on_zoom_changed(&mut self, cx: &mut Context<Self>) {
        let level = self.zoom_level(cx);
        if self.options.font_size != zoomed_options(self.options(), level).font_size {
            self.update_options(self.options().clone(), cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_scales_font_and_line_height() {
        let mut options = EditorOptions {
            font_size: px(10.),
            line_height: Some(px(20.)),
            ..Default::default()
        };
        let zoomed = zoomed_options(&options, 2.);
        assert_eq!(zoomed.font_size, px(12.));
        assert_eq!(effective_line_height(&zoomed), px(24.));
        options.line_height = None;
        let unzoomed = zoomed_options(&options, 0.);
        assert_eq!(unzoomed.font_size, options.font_size);
        assert_eq!(
            effective_line_height(&unzoomed),
            options.resolved_line_height()
        );
    }

    /// Monaco `BareFontInfo._create` rounds the line height, then zooms it
    /// without rounding again.
    #[test]
    fn zoomed_line_height_stays_fractional() {
        let options = EditorOptions {
            font_size: px(12.),
            line_height: Some(px(18.)),
            ..Default::default()
        };
        let line_height =
            |level: f32| f32::from(effective_line_height(&zoomed_options(&options, level)));
        assert!((line_height(1.) - 19.8).abs() < 1e-4);
        assert!((line_height(0.5) - 18.9).abs() < 1e-4);
        // A rounded user line height: 18.4 is 18 before zooming.
        let options = EditorOptions {
            line_height: Some(px(18.4)),
            ..options
        };
        assert!(
            (f32::from(effective_line_height(&zoomed_options(&options, 1.))) - 19.8).abs() < 1e-4
        );
        // Zooming out goes under Monaco's 8px minimum, as Monaco does.
        let options = EditorOptions {
            line_height: Some(px(8.)),
            ..options
        };
        assert_eq!(
            effective_line_height(&zoomed_options(&options, -5.)),
            px(4.)
        );
        // Zooming in goes over the 150px limit of the option.
        let options = EditorOptions {
            line_height: Some(px(150.)),
            ..options
        };
        assert_eq!(
            effective_line_height(&zoomed_options(&options, 10.)),
            px(300.)
        );
    }
}
