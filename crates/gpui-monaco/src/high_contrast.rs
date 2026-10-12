//! Monaco `autoDetectHighContrast` (`standaloneThemeService.ts`, MIT):
//! while the OS is in high contrast mode, editors switch to the matching
//! high contrast theme.
//!
//! Monaco watches the browser's `(forced-colors: active)` media query. GPUI
//! doesn't report the OS's high contrast setting, so the host reports it
//! with [`set_os_high_contrast`] (for example from the platform's
//! accessibility settings) and editors react as Monaco's do.

use gpui::{App, Global};

use crate::theme::{ThemeKind, global_theme, set_theme};

/// Monaco `StandaloneThemeService._autoDetectHighContrast` and the OS state
/// it follows.
struct HighContrast {
    auto_detect: bool,
    os_high_contrast: bool,
}

impl Default for HighContrast {
    fn default() -> Self {
        Self {
            // Monaco's default.
            auto_detect: true,
            os_high_contrast: false,
        }
    }
}

impl Global for HighContrast {}

/// Whether editors follow the OS's high contrast mode (Monaco
/// `autoDetectHighContrast`, default `true`).
pub fn auto_detect_high_contrast(cx: &App) -> bool {
    cx.try_global::<HighContrast>()
        .is_none_or(|state| state.auto_detect)
}

/// Turns following the OS's high contrast mode on or off (Monaco
/// `autoDetectHighContrast`, a construction and `updateOptions` option that
/// sets `setAutoDetectHighContrast` for every editor). Turning it on
/// applies the OS state reported with [`set_os_high_contrast`] right away.
pub fn set_auto_detect_high_contrast(cx: &mut App, auto_detect: bool) {
    cx.default_global::<HighContrast>().auto_detect = auto_detect;
    on_os_scheme_changed(cx);
}

/// Whether the host reported the OS in high contrast mode.
pub fn os_high_contrast(cx: &App) -> bool {
    cx.try_global::<HighContrast>()
        .is_some_and(|state| state.os_high_contrast)
}

/// Reports whether the OS is in high contrast mode (what Monaco reads from
/// `(forced-colors: active)`). With [`auto_detect_high_contrast`] on, a
/// change switches every editor to `hc-black` or `hc-light` (keeping dark
/// or light), or back to `vs-dark` or `vs`.
pub fn set_os_high_contrast(cx: &mut App, high_contrast: bool) {
    cx.default_global::<HighContrast>().os_high_contrast = high_contrast;
    on_os_scheme_changed(cx);
}

/// Monaco `StandaloneThemeService._onOSSchemeChanged`.
fn on_os_scheme_changed(cx: &mut App) {
    let state = cx.default_global::<HighContrast>();
    if !state.auto_detect {
        return;
    }
    let wants_high_contrast = state.os_high_contrast;
    let current = global_theme(cx).unwrap_or_else(|| std::sync::Arc::new(crate::Theme::vs()));
    if wants_high_contrast == current.high_contrast {
        return;
    }
    // Switch to or from high contrast, staying dark or light.
    let name = match (current.kind, wants_high_contrast) {
        (ThemeKind::Dark, true) => "hc-black",
        (ThemeKind::Dark, false) => "vs-dark",
        (ThemeKind::Light, true) => "hc-light",
        (ThemeKind::Light, false) => "vs",
    };
    set_theme(cx, name);
}
