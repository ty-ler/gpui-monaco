//! Tab focus mode (Monaco `TabFocus` and `toggleTabFocusMode`, MIT): Tab
//! moves focus to the next element instead of inserting a tab, so keyboard
//! users can leave the editor.

use gpui::{App, Global};

/// Monaco `TabFocus`: one mode for every editor.
#[derive(Default)]
pub(crate) struct TabFocus(bool);

impl Global for TabFocus {}

/// Whether Tab moves focus in every editor (Monaco
/// `TabFocus.getTabFocusMode`).
pub fn tab_focus_mode(cx: &App) -> bool {
    cx.try_global::<TabFocus>().is_some_and(|mode| mode.0)
}

/// Sets whether Tab moves focus in every editor (Monaco
/// `TabFocus.setTabFocusMode`, `editor.action.toggleTabFocusMode`).
pub fn set_tab_focus_mode(on: bool, cx: &mut App) {
    if tab_focus_mode(cx) != on {
        cx.set_global(TabFocus(on));
        cx.refresh_windows();
    }
}
