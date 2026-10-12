//! Keybinding labels as Monaco writes them: "F12", "⌘F2", "⇧⌥F" on macOS,
//! "Ctrl+Shift+P" elsewhere, chords joined by a space.
//!
//! Ports `base/common/keybindingLabels.ts` (`UILabelProvider`,
//! `AriaLabelProvider`, `ElectronAcceleratorLabelProvider`,
//! `UserSettingsLabelProvider` and `_simpleAsString`), the key names of
//! `base/common/keyCodes.ts` (`KeyCodeUtils.toString`,
//! `toElectronAccelerator`, `toUserSettingsUS`) and
//! `USLayoutResolvedKeybinding`'s per-chord labels (MIT), over GPUI
//! keystrokes. GPUI's `platform` modifier is Monaco's meta key (Cmd on
//! macOS, Windows or Super elsewhere) and `function` has no Monaco
//! equivalent, so it's left out.
//!
//! The command palette, the context menu, the code action menu, the
//! lightbulb and the find widget's titles all format keys through here.

use gpui::{KeyBinding, Keystroke, SharedString};

/// The platform whose conventions a label follows (Monaco
/// `OperatingSystem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatingSystem {
    /// macOS: modifier symbols, no separator.
    Macintosh,
    /// Windows: "Ctrl+Shift+Alt+Windows+A".
    Windows,
    /// Linux and the rest: "Ctrl+Shift+Alt+Super+A".
    Linux,
}

impl OperatingSystem {
    /// The platform this build runs on (Monaco `OS`).
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            OperatingSystem::Macintosh
        } else if cfg!(target_os = "windows") {
            OperatingSystem::Windows
        } else {
            OperatingSystem::Linux
        }
    }
}

/// Monaco `ModifierLabels`.
struct ModifierLabels {
    ctrl: &'static str,
    shift: &'static str,
    alt: &'static str,
    meta: &'static str,
    separator: &'static str,
}

/// The label providers of `keybindingLabels.ts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Ui,
    Aria,
    Electron,
    UserSettings,
}

impl Provider {
    fn modifiers(self, os: OperatingSystem) -> ModifierLabels {
        use OperatingSystem::*;
        let words = |ctrl, alt, meta| ModifierLabels {
            ctrl,
            shift: "Shift",
            alt,
            meta,
            separator: "+",
        };
        let settings = |meta| ModifierLabels {
            ctrl: "ctrl",
            shift: "shift",
            alt: "alt",
            meta,
            separator: "+",
        };
        match (self, os) {
            (Provider::Ui, Macintosh) => ModifierLabels {
                ctrl: "\u{2303}",
                shift: "⇧",
                alt: "⌥",
                meta: "⌘",
                separator: "",
            },
            (Provider::Ui, Windows) => words("Ctrl", "Alt", "Windows"),
            (Provider::Ui, Linux) => words("Ctrl", "Alt", "Super"),
            (Provider::Aria, Macintosh) => words("Control", "Option", "Command"),
            (Provider::Aria, Windows) => words("Control", "Alt", "Windows"),
            (Provider::Aria, Linux) => words("Control", "Alt", "Super"),
            (Provider::Electron, Macintosh) => words("Ctrl", "Alt", "Cmd"),
            (Provider::Electron, _) => words("Ctrl", "Alt", "Super"),
            (Provider::UserSettings, Macintosh) => settings("cmd"),
            (Provider::UserSettings, Windows) => settings("win"),
            (Provider::UserSettings, Linux) => settings("meta"),
        }
    }
}

/// Whether `key` is a modifier key itself (Monaco
/// `isDuplicateModifierCase`: its label comes from the modifiers alone).
fn is_modifier_key(key: &str) -> bool {
    matches!(
        key,
        "shift" | "control" | "ctrl" | "alt" | "option" | "cmd" | "platform" | "super" | "win"
    )
}

/// Monaco `KeyCodeUtils.toString`: the key's name in the UI map ("A",
/// "F12", "Enter", "PageDown", "UpArrow", "Del", "NumPad1", ",").
fn key_name(key: &str) -> String {
    let named = match key {
        "enter" => "Enter",
        "escape" => "Escape",
        "backspace" => "Backspace",
        "tab" => "Tab",
        "space" => "Space",
        "delete" => "Del",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "up" => "UpArrow",
        "down" => "DownArrow",
        "left" => "LeftArrow",
        "right" => "RightArrow",
        "capslock" => "CapsLock",
        "numlock" => "NumLock",
        "scrolllock" => "ScrollLock",
        "pause" => "PauseBreak",
        "menu" | "contextmenu" => "ContextMenu",
        "shift" => "Shift",
        "control" | "ctrl" => "Ctrl",
        "alt" | "option" => "Alt",
        "cmd" | "platform" | "super" | "win" => "Meta",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    if let Some(n) = key.strip_prefix('f').filter(|n| {
        !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && n.parse::<u8>().is_ok()
    }) {
        return format!("F{n}");
    }
    if let Some(rest) = key.strip_prefix("numpad") {
        return match rest {
            "add" => "NumPad_Add".into(),
            "subtract" => "NumPad_Subtract".into(),
            "multiply" => "NumPad_Multiply".into(),
            "divide" => "NumPad_Divide".into(),
            "decimal" => "NumPad_Decimal".into(),
            digits => format!("NumPad{digits}"),
        };
    }
    // Letters are upper case; digits and punctuation are themselves.
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => c.to_uppercase().collect(),
        _ => {
            let mut chars = key.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// The key's label for `provider`, or `None` when the keystroke can't be
/// expressed (Monaco's `KeyLabelProvider` returning `null`).
fn key_label(key: &str, provider: Provider, os: OperatingSystem) -> Option<String> {
    let arrow = |mac: &str, electron: &str| match (provider, os) {
        (Provider::Ui, OperatingSystem::Macintosh) => Some(mac.to_string()),
        (Provider::Electron, _) => Some(electron.to_string()),
        _ => None,
    };
    let special = match key {
        "up" => arrow("↑", "Up"),
        "down" => arrow("↓", "Down"),
        "left" => arrow("←", "Left"),
        "right" => arrow("→", "Right"),
        // `toUserSettingsUS` and `toElectronAccelerator` spell it out.
        "delete" if matches!(provider, Provider::Electron | Provider::UserSettings) => {
            Some("Delete".to_string())
        }
        _ => None,
    };
    if provider == Provider::Electron && key.starts_with("numpad") {
        // Electron renders numpad keys like the main ones.
        return None;
    }
    let label = special.unwrap_or_else(|| key_name(key));
    Some(match provider {
        Provider::UserSettings => label.to_lowercase(),
        _ => label,
    })
}

/// One chord's modifier labels and key label (Monaco `_simpleAsString`'s
/// parts).
fn chord_parts(
    keystroke: &Keystroke,
    provider: Provider,
    os: OperatingSystem,
) -> Option<Vec<String>> {
    let labels = provider.modifiers(os);
    let modifiers = &keystroke.modifiers;
    let duplicate = is_modifier_key(&keystroke.key);
    let key = if duplicate {
        if provider == Provider::Electron {
            return None;
        }
        String::new()
    } else {
        key_label(&keystroke.key, provider, os)?
    };
    let mut parts = Vec::new();
    // Ctrl, Shift, Alt, Meta: on macOS ⌃⇧⌥⌘.
    let duplicate_of = |name: &[&str]| duplicate && name.contains(&keystroke.key.as_str());
    if modifiers.control || duplicate_of(&["control", "ctrl"]) {
        parts.push(labels.ctrl.to_string());
    }
    if modifiers.shift || duplicate_of(&["shift"]) {
        parts.push(labels.shift.to_string());
    }
    if modifiers.alt || duplicate_of(&["alt", "option"]) {
        parts.push(labels.alt.to_string());
    }
    if modifiers.platform || duplicate_of(&["cmd", "platform", "super", "win"]) {
        parts.push(labels.meta.to_string());
    }
    if !key.is_empty() {
        parts.push(key);
    }
    Some(parts)
}

fn label(keystrokes: &[Keystroke], provider: Provider, os: OperatingSystem) -> Option<String> {
    if keystrokes.is_empty() {
        return None;
    }
    // Electron accelerators can't express chords.
    if provider == Provider::Electron && keystrokes.len() > 1 {
        return None;
    }
    let separator = provider.modifiers(os).separator;
    let chords = keystrokes
        .iter()
        .map(|keystroke| Some(chord_parts(keystroke, provider, os)?.join(separator)))
        .collect::<Option<Vec<_>>>()?;
    Some(chords.join(" "))
}

/// Monaco `ResolvedKeybinding.getLabel()`: the label shown in menus and
/// lists ("⇧⌘P", "Ctrl+Shift+P", "⌘K ⌘C").
pub fn ui_label(keystrokes: &[Keystroke], os: OperatingSystem) -> Option<String> {
    label(keystrokes, Provider::Ui, os)
}

/// Monaco `getAriaLabel()`: modifiers spelled out for screen readers
/// ("Control+Shift+Option+Command+A").
pub fn aria_label(keystrokes: &[Keystroke], os: OperatingSystem) -> Option<String> {
    label(keystrokes, Provider::Aria, os)
}

/// Monaco `getElectronAccelerator()`: `None` for chords, numpad keys and
/// modifier keys alone.
pub fn electron_accelerator(keystrokes: &[Keystroke], os: OperatingSystem) -> Option<String> {
    label(keystrokes, Provider::Electron, os)
}

/// Monaco `getUserSettingsLabel()`: the `keybindings.json` form
/// ("ctrl+shift+alt+cmd+a").
pub fn user_settings_label(keystrokes: &[Keystroke], os: OperatingSystem) -> Option<String> {
    label(keystrokes, Provider::UserSettings, os)
}

/// The keycaps Monaco's `KeybindingLabel` draws: per chord, one cap per
/// modifier and one for the key, and the separator shown between caps
/// (`"+"` except on macOS).
pub fn ui_keycaps(
    keystrokes: &[Keystroke],
    os: OperatingSystem,
) -> (Vec<Vec<String>>, &'static str) {
    let chords = keystrokes
        .iter()
        .filter_map(|keystroke| chord_parts(keystroke, Provider::Ui, os))
        .collect();
    (chords, Provider::Ui.modifiers(os).separator)
}

/// The keystrokes of `binding`.
pub(crate) fn binding_keystrokes(binding: &KeyBinding) -> Vec<Keystroke> {
    binding
        .keystrokes()
        .iter()
        .map(|keystroke| keystroke.inner().clone())
        .collect()
}

/// The binding Monaco's `lookupKeybinding` shows for `action` in the
/// context of `focus`: the last keybinding rule (`add_keybinding_rules`,
/// bound as [`RunCommand`](crate::actions::RunCommand)) naming its Monaco
/// command id, as Monaco's dynamic keybindings come after the defaults;
/// else its default binding. `id` names the command when the action runs
/// several; by default it's the action's.
pub(crate) fn command_binding(
    action: &dyn gpui::Action,
    id: Option<&str>,
    focus: &gpui::FocusHandle,
    window: &gpui::Window,
) -> Option<KeyBinding> {
    let default = window.highest_precedence_binding_for_action_in(action, focus);
    let Some(id) = id.or_else(|| crate::editor_actions::monaco_id(action.name())) else {
        return default;
    };
    let rule = window
        .bindings_for_action_in(
            &crate::actions::RunCommand {
                id: id.to_string(),
                args: None,
            },
            focus,
        )
        .pop();
    // Monaco's dynamic keybindings come after the defaults, so they win.
    rule.or(default)
}

/// [`command_binding`]'s label.
pub(crate) fn command_label(
    action: &dyn gpui::Action,
    id: Option<&str>,
    focus: &gpui::FocusHandle,
    window: &gpui::Window,
) -> Option<SharedString> {
    command_binding(action, id, focus, window).map(|binding| binding_label(&binding))
}

/// `binding`'s UI label on this platform.
pub(crate) fn binding_label(binding: &KeyBinding) -> SharedString {
    ui_label(&binding_keystrokes(binding), OperatingSystem::current())
        .unwrap_or_default()
        .into()
}

/// Monaco's `KeybindingLabel` widget (`.monaco-keybinding`): each key in a
/// keycap of `keybindingLabel.*` colors, the caps of a chord joined by the
/// platform's separator.
pub(crate) fn keycaps(label: &KeyLabel, colors: &crate::theme::EditorColors) -> gpui::AnyElement {
    use gpui::{IntoElement as _, ParentElement as _, Styled as _, div, px};
    let shadow = colors.widget_shadow.map(|color| gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(-1.)),
        blur_radius: px(0.),
        spread_radius: px(0.),
        inset: true,
    });
    let mut row = div()
        .flex()
        .items_center()
        .flex_none()
        .text_size(px(11.))
        .line_height(px(10.));
    let last_chord = label.chords.len().saturating_sub(1);
    for (chord_ix, chord) in label.chords.iter().enumerate() {
        if chord_ix > 0 {
            // `.monaco-keybinding-key-chord-separator`.
            row = row.child(div().w(px(6.)));
        }
        for (ix, key) in chord.iter().enumerate() {
            if ix > 0 && !label.separator.is_empty() {
                // `.monaco-keybinding-key-separator`.
                row = row.child(div().child(label.separator));
            }
            let first = chord_ix == 0 && ix == 0;
            let last = chord_ix == last_chord && ix + 1 == chord.len();
            // `.monaco-keybinding-key`: 11px, `3px 5px` padding, 3px
            // corners, `keybindingLabel.border` with the bottom in
            // `keybindingLabel.bottomBorder`, and an inset `widget.shadow`.
            let cap = div()
                .px(px(5.))
                .py(px(3.))
                .rounded_t(px(3.))
                .border_t_1()
                .border_l_1()
                .border_r_1()
                .border_color(colors.keybinding_label_border)
                .bg(colors.keybinding_label_background)
                .text_color(colors.keybinding_label_foreground)
                .when_some(shadow.clone(), |cap, shadow| cap.shadow(vec![shadow]))
                .child(SharedString::from(key.clone()));
            row = row.child(
                div()
                    .ml(px(if first { 0. } else { 2. }))
                    .mr(px(if last { 0. } else { 2. }))
                    .rounded(px(3.))
                    .border_b_1()
                    .border_color(colors.keybinding_label_bottom_border)
                    .child(cap),
            );
        }
    }
    row.into_any_element()
}

use gpui::prelude::FluentBuilder as _;

/// The width of `text` in the window's UI font at `size` (what Monaco's
/// menus measure with `getBoundingClientRect` to size themselves).
pub(crate) fn ui_text_width(
    text: &str,
    size: gpui::Pixels,
    weight: gpui::FontWeight,
    window: &mut gpui::Window,
) -> gpui::Pixels {
    if text.is_empty() {
        return gpui::px(0.);
    }
    let mut font = window.text_style().font();
    font.weight = weight;
    let run = gpui::TextRun {
        len: text.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(SharedString::from(text.to_string()), size, &[run], None)
        .width
}

/// The width [`keycaps`] draws `label` at.
pub(crate) fn keycaps_width(label: &KeyLabel, window: &mut gpui::Window) -> gpui::Pixels {
    use gpui::{FontWeight, px};
    let size = px(11.);
    let mut width = px(0.);
    let caps: usize = label.chords.iter().map(Vec::len).sum();
    for (chord_ix, chord) in label.chords.iter().enumerate() {
        if chord_ix > 0 {
            width += px(6.);
        }
        for (ix, key) in chord.iter().enumerate() {
            if ix > 0 && !label.separator.is_empty() {
                width += ui_text_width(label.separator, size, FontWeight::NORMAL, window);
            }
            // Padding and borders.
            width += ui_text_width(key, size, FontWeight::NORMAL, window) + px(12.);
        }
    }
    // 2px margins between caps.
    width + px(4.) * caps.saturating_sub(1) as f32
}

/// A keybinding's keycaps on this platform, for [`keycaps`].
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct KeyLabel {
    /// Per chord, its caps.
    pub chords: Vec<Vec<String>>,
    /// Shown between a chord's caps.
    pub separator: &'static str,
    /// The whole label as text.
    pub text: SharedString,
}

impl KeyLabel {
    /// `binding`'s caps on this platform.
    pub fn for_binding(binding: &KeyBinding) -> Self {
        Self::for_keystrokes(&binding_keystrokes(binding))
    }

    /// `keystrokes`' caps on this platform.
    pub fn for_keystrokes(keystrokes: &[Keystroke]) -> Self {
        let os = OperatingSystem::current();
        let (chords, separator) = ui_keycaps(keystrokes, os);
        Self {
            chords,
            separator,
            text: ui_label(keystrokes, os).unwrap_or_default().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OperatingSystem::{Linux, Macintosh, Windows};
    use super::*;

    fn keys(text: &str) -> Vec<Keystroke> {
        text.split_whitespace()
            .map(|k| Keystroke::parse(k).unwrap())
            .collect()
    }

    fn ui(os: OperatingSystem, text: &str) -> String {
        ui_label(&keys(text), os).unwrap()
    }

    // `keybindingLabels.test.ts` ("KeybindingLabels"), with Monaco's
    // `KeyMod.CtrlCmd` as `cmd` on macOS and `ctrl` elsewhere, and
    // `KeyMod.WinCtrl` as `ctrl` on macOS and the meta key (`cmd`, GPUI's
    // platform key) elsewhere.

    #[test]
    fn windows_us_label() {
        // no modifier
        assert_eq!(ui(Windows, "a"), "A");
        // one modifier
        assert_eq!(ui(Windows, "ctrl-a"), "Ctrl+A");
        assert_eq!(ui(Windows, "shift-a"), "Shift+A");
        assert_eq!(ui(Windows, "alt-a"), "Alt+A");
        assert_eq!(ui(Windows, "cmd-a"), "Windows+A");
        // two modifiers
        assert_eq!(ui(Windows, "ctrl-shift-a"), "Ctrl+Shift+A");
        assert_eq!(ui(Windows, "ctrl-alt-a"), "Ctrl+Alt+A");
        assert_eq!(ui(Windows, "ctrl-cmd-a"), "Ctrl+Windows+A");
        assert_eq!(ui(Windows, "shift-alt-a"), "Shift+Alt+A");
        assert_eq!(ui(Windows, "shift-cmd-a"), "Shift+Windows+A");
        assert_eq!(ui(Windows, "alt-cmd-a"), "Alt+Windows+A");
        // three modifiers
        assert_eq!(ui(Windows, "ctrl-shift-alt-a"), "Ctrl+Shift+Alt+A");
        assert_eq!(ui(Windows, "ctrl-shift-cmd-a"), "Ctrl+Shift+Windows+A");
        assert_eq!(ui(Windows, "ctrl-alt-cmd-a"), "Ctrl+Alt+Windows+A");
        assert_eq!(ui(Windows, "shift-alt-cmd-a"), "Shift+Alt+Windows+A");
        // four modifiers
        assert_eq!(
            ui(Windows, "ctrl-shift-alt-cmd-a"),
            "Ctrl+Shift+Alt+Windows+A"
        );
        // chord
        assert_eq!(ui(Windows, "ctrl-a ctrl-b"), "Ctrl+A Ctrl+B");
    }

    #[test]
    fn linux_us_label() {
        assert_eq!(ui(Linux, "a"), "A");
        assert_eq!(ui(Linux, "ctrl-a"), "Ctrl+A");
        assert_eq!(ui(Linux, "shift-a"), "Shift+A");
        assert_eq!(ui(Linux, "alt-a"), "Alt+A");
        assert_eq!(ui(Linux, "cmd-a"), "Super+A");
        assert_eq!(ui(Linux, "ctrl-shift-a"), "Ctrl+Shift+A");
        assert_eq!(ui(Linux, "ctrl-alt-a"), "Ctrl+Alt+A");
        assert_eq!(ui(Linux, "ctrl-cmd-a"), "Ctrl+Super+A");
        assert_eq!(ui(Linux, "shift-alt-a"), "Shift+Alt+A");
        assert_eq!(ui(Linux, "shift-cmd-a"), "Shift+Super+A");
        assert_eq!(ui(Linux, "alt-cmd-a"), "Alt+Super+A");
        assert_eq!(ui(Linux, "ctrl-shift-alt-a"), "Ctrl+Shift+Alt+A");
        assert_eq!(ui(Linux, "ctrl-shift-cmd-a"), "Ctrl+Shift+Super+A");
        assert_eq!(ui(Linux, "ctrl-alt-cmd-a"), "Ctrl+Alt+Super+A");
        assert_eq!(ui(Linux, "shift-alt-cmd-a"), "Shift+Alt+Super+A");
        assert_eq!(ui(Linux, "ctrl-shift-alt-cmd-a"), "Ctrl+Shift+Alt+Super+A");
        assert_eq!(ui(Linux, "ctrl-a ctrl-b"), "Ctrl+A Ctrl+B");
    }

    #[test]
    fn mac_us_label() {
        assert_eq!(ui(Macintosh, "a"), "A");
        // one modifier
        assert_eq!(ui(Macintosh, "cmd-a"), "⌘A");
        assert_eq!(ui(Macintosh, "shift-a"), "⇧A");
        assert_eq!(ui(Macintosh, "alt-a"), "⌥A");
        assert_eq!(ui(Macintosh, "ctrl-a"), "⌃A");
        // two modifiers
        assert_eq!(ui(Macintosh, "cmd-shift-a"), "⇧⌘A");
        assert_eq!(ui(Macintosh, "cmd-alt-a"), "⌥⌘A");
        assert_eq!(ui(Macintosh, "cmd-ctrl-a"), "⌃⌘A");
        assert_eq!(ui(Macintosh, "shift-alt-a"), "⇧⌥A");
        assert_eq!(ui(Macintosh, "shift-ctrl-a"), "⌃⇧A");
        assert_eq!(ui(Macintosh, "alt-ctrl-a"), "⌃⌥A");
        // three modifiers
        assert_eq!(ui(Macintosh, "cmd-shift-alt-a"), "⇧⌥⌘A");
        assert_eq!(ui(Macintosh, "cmd-shift-ctrl-a"), "⌃⇧⌘A");
        assert_eq!(ui(Macintosh, "cmd-alt-ctrl-a"), "⌃⌥⌘A");
        assert_eq!(ui(Macintosh, "shift-alt-ctrl-a"), "⌃⇧⌥A");
        // four modifiers
        assert_eq!(ui(Macintosh, "cmd-shift-alt-ctrl-a"), "⌃⇧⌥⌘A");
        // chord
        assert_eq!(ui(Macintosh, "cmd-a cmd-b"), "⌘A ⌘B");
        // special keys
        assert_eq!(ui(Macintosh, "left"), "←");
        assert_eq!(ui(Macintosh, "up"), "↑");
        assert_eq!(ui(Macintosh, "right"), "→");
        assert_eq!(ui(Macintosh, "down"), "↓");
    }

    #[test]
    fn aria_label_spells_out_modifiers() {
        let all = keys("ctrl-shift-alt-cmd-a");
        assert_eq!(
            aria_label(&all, Windows).unwrap(),
            "Control+Shift+Alt+Windows+A"
        );
        assert_eq!(
            aria_label(&all, Linux).unwrap(),
            "Control+Shift+Alt+Super+A"
        );
        assert_eq!(
            aria_label(&all, Macintosh).unwrap(),
            "Control+Shift+Option+Command+A"
        );
    }

    #[test]
    fn electron_accelerator_label() {
        let all = keys("ctrl-shift-alt-cmd-a");
        assert_eq!(
            electron_accelerator(&all, Windows).unwrap(),
            "Ctrl+Shift+Alt+Super+A"
        );
        assert_eq!(
            electron_accelerator(&all, Linux).unwrap(),
            "Ctrl+Shift+Alt+Super+A"
        );
        assert_eq!(
            electron_accelerator(&all, Macintosh).unwrap(),
            "Ctrl+Shift+Alt+Cmd+A"
        );
        // electron cannot handle chords
        assert_eq!(electron_accelerator(&keys("ctrl-a ctrl-b"), Windows), None);
        // electron cannot handle numpad keys
        assert_eq!(electron_accelerator(&keys("numpad1"), Windows), None);
        // special
        assert_eq!(
            electron_accelerator(&keys("left"), Macintosh).unwrap(),
            "Left"
        );
        assert_eq!(electron_accelerator(&keys("up"), Macintosh).unwrap(), "Up");
        assert_eq!(
            electron_accelerator(&keys("right"), Macintosh).unwrap(),
            "Right"
        );
        assert_eq!(
            electron_accelerator(&keys("down"), Macintosh).unwrap(),
            "Down"
        );
    }

    #[test]
    fn user_settings_label_is_lower_case() {
        let all = keys("ctrl-shift-alt-cmd-a");
        assert_eq!(
            user_settings_label(&all, Windows).unwrap(),
            "ctrl+shift+alt+win+a"
        );
        assert_eq!(
            user_settings_label(&all, Linux).unwrap(),
            "ctrl+shift+alt+meta+a"
        );
        assert_eq!(
            user_settings_label(&all, Macintosh).unwrap(),
            "ctrl+shift+alt+cmd+a"
        );
        assert_eq!(
            user_settings_label(&keys("ctrl-a ctrl-b"), Windows).unwrap(),
            "ctrl+a ctrl+b"
        );
    }

    /// "issue #91235: Do not end with a +".
    #[test]
    fn modifier_keys_alone_do_not_end_with_a_plus() {
        let ctrl_alt = [Keystroke {
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            key: "alt".into(),
            key_char: None,
        }];
        assert_eq!(aria_label(&ctrl_alt, Windows).unwrap(), "Control+Alt");
    }

    // `usLayoutResolvedKeybinding.test.ts`.

    #[test]
    fn resolve_user_binding_ctrl_comma_ctrl_slash() {
        assert_eq!(
            user_settings_label(&keys("ctrl-, ctrl-/"), Windows).unwrap(),
            "ctrl+, ctrl+/"
        );
        assert_eq!(ui(Windows, "ctrl-, ctrl-/"), "Ctrl+, Ctrl+/");
    }

    #[test]
    fn single_modifier_keys() {
        let ctrl = [Keystroke {
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            key: "control".into(),
            key_char: None,
        }];
        assert_eq!(ui_label(&ctrl, Windows).unwrap(), "Ctrl");
        assert_eq!(aria_label(&ctrl, Windows).unwrap(), "Control");
        assert_eq!(electron_accelerator(&ctrl, Windows), None);
        assert_eq!(user_settings_label(&ctrl, Windows).unwrap(), "ctrl");
        let meta = [Keystroke {
            modifiers: gpui::Modifiers {
                platform: true,
                ..Default::default()
            },
            key: "cmd".into(),
            key_char: None,
        }];
        assert_eq!(ui_label(&meta, Macintosh).unwrap(), "⌘");
        assert_eq!(aria_label(&meta, Macintosh).unwrap(), "Command");
        assert_eq!(user_settings_label(&meta, Linux).unwrap(), "meta");
    }

    /// The labels issue #251 names: function keys and letters capitalized,
    /// macOS modifiers in ⌃⇧⌥⌘ order.
    #[test]
    fn key_names_follow_keycodes() {
        assert_eq!(ui(Macintosh, "f12"), "F12");
        assert_eq!(ui(Macintosh, "shift-f12"), "⇧F12");
        assert_eq!(ui(Macintosh, "cmd-f2"), "⌘F2");
        assert_eq!(ui(Macintosh, "alt-shift-f"), "⇧⌥F");
        assert_eq!(ui(Macintosh, "shift-alt-o"), "⇧⌥O");
        assert_eq!(ui(Windows, "shift-alt-o"), "Shift+Alt+O");
        assert_eq!(ui(Windows, "enter"), "Enter");
        assert_eq!(ui(Windows, "escape"), "Escape");
        assert_eq!(ui(Windows, "pagedown"), "PageDown");
        assert_eq!(ui(Windows, "ctrl-up"), "Ctrl+UpArrow");
        assert_eq!(ui(Windows, "delete"), "Del");
        assert_eq!(ui(Windows, "ctrl-space"), "Ctrl+Space");
        assert_eq!(ui(Windows, "ctrl-`"), "Ctrl+`");
        assert_eq!(ui(Macintosh, "cmd-k cmd-c"), "⌘K ⌘C");
        assert_eq!(
            user_settings_label(&keys("delete"), Windows).unwrap(),
            "delete"
        );
    }

    #[test]
    fn keycaps_split_modifiers() {
        assert_eq!(
            ui_keycaps(&keys("cmd-shift-p"), Macintosh),
            (vec![vec!["⇧".to_string(), "⌘".into(), "P".into()]], "")
        );
        assert_eq!(
            ui_keycaps(&keys("ctrl-k ctrl-c"), Windows),
            (
                vec![
                    vec!["Ctrl".to_string(), "K".into()],
                    vec!["Ctrl".to_string(), "C".into()]
                ],
                "+"
            )
        );
    }
}
