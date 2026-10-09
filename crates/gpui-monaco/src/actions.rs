//! Editor commands as GPUI actions, and Monaco's default keybindings.
//!
//! Every binding lives in the [`KEY_CONTEXT`] key context, so hosts can
//! override any of them by binding the same keystroke in that context after
//! [`crate::init`], or skip [`default_key_bindings`] entirely and bind their
//! own.

use gpui::{App, KeyBinding, actions};

/// The key context the editor element establishes.
pub const KEY_CONTEXT: &str = "CodeEditor";

actions!(
    code_editor,
    [
        CursorLeft,
        CursorLeftSelect,
        CursorRight,
        CursorRightSelect,
        CursorUp,
        CursorUpSelect,
        CursorDown,
        CursorDownSelect,
        CursorWordLeft,
        CursorWordLeftSelect,
        CursorWordRight,
        CursorWordRightSelect,
        CursorWordPartLeft,
        CursorWordPartLeftSelect,
        CursorWordPartRight,
        CursorWordPartRightSelect,
        CursorHome,
        CursorHomeSelect,
        CursorEnd,
        CursorEndSelect,
        CursorTop,
        CursorTopSelect,
        CursorBottom,
        CursorBottomSelect,
        CursorPageUp,
        CursorPageUpSelect,
        CursorPageDown,
        CursorPageDownSelect,
        SelectAll,
        ExpandLineSelection,
        AddCursorAbove,
        AddCursorBelow,
        RemoveSecondaryCursors,
        DeleteLeft,
        DeleteRight,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteAllLeft,
        Tab,
        Outdent,
        Newline,
        IndentLines,
        OutdentLines,
        MoveLinesUp,
        MoveLinesDown,
        CopyLinesUp,
        CopyLinesDown,
        DeleteLines,
        InsertLineAfter,
        InsertLineBefore,
        Undo,
        Redo,
        Copy,
        Cut,
        Paste,
        ShowCharacterPalette,
    ]
);

/// Monaco's default keybindings for the current platform.
pub fn default_key_bindings() -> Vec<KeyBinding> {
    let ctx = Some(KEY_CONTEXT);
    let mut bindings = vec![
        KeyBinding::new("left", CursorLeft, ctx),
        KeyBinding::new("shift-left", CursorLeftSelect, ctx),
        KeyBinding::new("right", CursorRight, ctx),
        KeyBinding::new("shift-right", CursorRightSelect, ctx),
        KeyBinding::new("up", CursorUp, ctx),
        KeyBinding::new("shift-up", CursorUpSelect, ctx),
        KeyBinding::new("down", CursorDown, ctx),
        KeyBinding::new("shift-down", CursorDownSelect, ctx),
        KeyBinding::new("home", CursorHome, ctx),
        KeyBinding::new("shift-home", CursorHomeSelect, ctx),
        KeyBinding::new("end", CursorEnd, ctx),
        KeyBinding::new("shift-end", CursorEndSelect, ctx),
        KeyBinding::new("pageup", CursorPageUp, ctx),
        KeyBinding::new("shift-pageup", CursorPageUpSelect, ctx),
        KeyBinding::new("pagedown", CursorPageDown, ctx),
        KeyBinding::new("shift-pagedown", CursorPageDownSelect, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("secondary-l", ExpandLineSelection, ctx),
        KeyBinding::new("escape", RemoveSecondaryCursors, ctx),
        KeyBinding::new("backspace", DeleteLeft, ctx),
        KeyBinding::new("shift-backspace", DeleteLeft, ctx),
        KeyBinding::new("delete", DeleteRight, ctx),
        KeyBinding::new("tab", Tab, ctx),
        KeyBinding::new("shift-tab", Outdent, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
        KeyBinding::new("secondary-]", IndentLines, ctx),
        KeyBinding::new("secondary-[", OutdentLines, ctx),
        KeyBinding::new("alt-up", MoveLinesUp, ctx),
        KeyBinding::new("alt-down", MoveLinesDown, ctx),
        KeyBinding::new("secondary-shift-k", DeleteLines, ctx),
        KeyBinding::new("secondary-enter", InsertLineAfter, ctx),
        KeyBinding::new("secondary-shift-enter", InsertLineBefore, ctx),
        KeyBinding::new("secondary-z", Undo, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
    ];

    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("alt-left", CursorWordLeft, ctx),
            KeyBinding::new("shift-alt-left", CursorWordLeftSelect, ctx),
            KeyBinding::new("alt-right", CursorWordRight, ctx),
            KeyBinding::new("shift-alt-right", CursorWordRightSelect, ctx),
            KeyBinding::new("ctrl-alt-left", CursorWordPartLeft, ctx),
            KeyBinding::new("ctrl-shift-alt-left", CursorWordPartLeftSelect, ctx),
            KeyBinding::new("ctrl-alt-right", CursorWordPartRight, ctx),
            KeyBinding::new("ctrl-shift-alt-right", CursorWordPartRightSelect, ctx),
            KeyBinding::new("cmd-left", CursorHome, ctx),
            KeyBinding::new("cmd-shift-left", CursorHomeSelect, ctx),
            KeyBinding::new("cmd-right", CursorEnd, ctx),
            KeyBinding::new("cmd-shift-right", CursorEndSelect, ctx),
            KeyBinding::new("cmd-up", CursorTop, ctx),
            KeyBinding::new("cmd-shift-up", CursorTopSelect, ctx),
            KeyBinding::new("cmd-down", CursorBottom, ctx),
            KeyBinding::new("cmd-shift-down", CursorBottomSelect, ctx),
            KeyBinding::new("cmd-home", CursorTop, ctx),
            KeyBinding::new("cmd-end", CursorBottom, ctx),
            KeyBinding::new("shift-alt-up", CopyLinesUp, ctx),
            KeyBinding::new("shift-alt-down", CopyLinesDown, ctx),
            KeyBinding::new("cmd-alt-up", AddCursorAbove, ctx),
            KeyBinding::new("cmd-alt-down", AddCursorBelow, ctx),
            KeyBinding::new("alt-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("alt-delete", DeleteWordRight, ctx),
            KeyBinding::new("cmd-backspace", DeleteAllLeft, ctx),
            KeyBinding::new("ctrl-h", DeleteLeft, ctx),
            KeyBinding::new("ctrl-d", DeleteRight, ctx),
            KeyBinding::new("cmd-shift-z", Redo, ctx),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-left", CursorWordLeft, ctx),
            KeyBinding::new("ctrl-shift-left", CursorWordLeftSelect, ctx),
            KeyBinding::new("ctrl-right", CursorWordRight, ctx),
            KeyBinding::new("ctrl-shift-right", CursorWordRightSelect, ctx),
            KeyBinding::new("alt-shift-left", CursorWordPartLeftSelect, ctx),
            KeyBinding::new("alt-shift-right", CursorWordPartRightSelect, ctx),
            KeyBinding::new("ctrl-home", CursorTop, ctx),
            KeyBinding::new("ctrl-shift-home", CursorTopSelect, ctx),
            KeyBinding::new("ctrl-end", CursorBottom, ctx),
            KeyBinding::new("ctrl-shift-end", CursorBottomSelect, ctx),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("ctrl-delete", DeleteWordRight, ctx),
            KeyBinding::new("ctrl-y", Redo, ctx),
            KeyBinding::new("ctrl-shift-z", Redo, ctx),
            KeyBinding::new("shift-delete", Cut, ctx),
            KeyBinding::new("ctrl-insert", Copy, ctx),
            KeyBinding::new("shift-insert", Paste, ctx),
        ]);
        if cfg!(target_os = "windows") {
            bindings.extend([
                KeyBinding::new("ctrl-alt-up", AddCursorAbove, ctx),
                KeyBinding::new("ctrl-alt-down", AddCursorBelow, ctx),
                KeyBinding::new("shift-alt-up", CopyLinesUp, ctx),
                KeyBinding::new("shift-alt-down", CopyLinesDown, ctx),
            ]);
        } else {
            bindings.extend([
                KeyBinding::new("shift-alt-up", AddCursorAbove, ctx),
                KeyBinding::new("shift-alt-down", AddCursorBelow, ctx),
                KeyBinding::new("ctrl-shift-alt-up", CopyLinesUp, ctx),
                KeyBinding::new("ctrl-shift-alt-down", CopyLinesDown, ctx),
            ]);
        }
    }
    bindings
}

/// Binds [`default_key_bindings`].
pub fn bind_default_keys(cx: &mut App) {
    cx.bind_keys(default_key_bindings());
}
