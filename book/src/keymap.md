# Keybindings and commands

Every editor command is a GPUI action in `gpui_monaco::actions`:

- `CursorWordLeft`, `DeleteLines` and `MoveLinesUp`
- `AddSelectionToNextFindMatch` and `StartFind`
- `ToggleFold`, `GoToDefinition` and `TriggerSuggest`
- …and so on

`gpui_monaco::init(cx)` binds Monaco's default keys for the current platform. On macOS, for example:

- **Cmd+D:** add the next match to the selection
- **Alt+Up / Alt+Down:** move lines
- **Cmd+Alt+[ / Cmd+Alt+]:** fold and unfold
- **Alt+F8 / F8:** next problem (Monaco's `marker.next` and `marker.nextInFiles`, which act the same within one model)
- **Cmd+Shift+, / Cmd+Shift+.:** replace the value at the cursor with the previous or next one (`1` → `2`, `true` → `false`)
- **Ctrl+B / Ctrl+F / Ctrl+P / Ctrl+N, Ctrl+A / Ctrl+E (with Shift to select), Ctrl+H / Ctrl+D:** Monaco's Emacs-style keys
- **F12:** go to definition
- **Ctrl+Space (or Alt+Esc, Cmd+I):** trigger completion
- **F1 / Ctrl+G / Cmd+Shift+O:** command palette, go to line, go to symbol
- **Shift+F10:** context menu
- **Tab / Cmd+Right / Alt+] / Alt+[ / Alt+\\** with ghost text: accept, accept a word, next, previous, ask explicitly
- **Tab / Shift+Tab / Escape** in a snippet: next and previous tab stop, leave the snippet
- **Ctrl+Space (or Cmd+I)** in the suggest widget: show or hide the details
- **Shift+Enter / Shift+Tab** in the suggest widget: accept in the other insert mode
- **Up / Down (or Cmd+Up / Cmd+Down, Ctrl+P / Ctrl+N), PageUp / PageDown** in the suggest widget: previous and next suggestion, by item or page
- **Tab / Shift+Tab** with `tab_completion: On`: insert the best completion, then cycle through the others

Tab focus mode (Ctrl+M; Ctrl+Shift+M on macOS) makes Tab and Shift+Tab move focus out of the editor, for keyboard and screen reader users.

Font zoom (`FontZoomIn`, `FontZoomOut`, `FontZoomReset`) has no default key, as in Monaco. Run it from the command palette, or bind it yourself. The zoom level is shared by every editor; `set_editor_zoom_level` sets it directly.

All of these bindings live in the `CodeEditor` key context (`actions::KEY_CONTEXT`). Single-line editors also set `single_line`.

## Rebinding

GPUI uses the binding added last, so to override a key, bind it in the same context after `init`:

```rust,ignore
use gpui::KeyBinding;
use gpui_monaco::actions::{self, KEY_CONTEXT};

gpui_monaco::init(cx);
cx.bind_keys([
    KeyBinding::new("cmd-shift-k", actions::DeleteLines, Some(KEY_CONTEXT)),
    KeyBinding::new("ctrl-.", actions::TriggerSuggest, Some(KEY_CONTEXT)),
]);
```

To build a keymap from scratch, skip `init` and bind exactly what you want. `actions::default_key_bindings()` returns Monaco's bindings as a `Vec<KeyBinding>`, which you can filter.

## Your own commands

Monaco's `addAction` corresponds to GPUI's usual pattern: define an action, bind it in the editor's context, and handle it on an element that contains the editor. While the editor has focus, its key context is part of the dispatch path, and actions bubble up to your handler.

```rust,ignore
gpui::actions!(my_app, [FormatDocument]);

cx.bind_keys([KeyBinding::new("shift-alt-f", FormatDocument, Some("CodeEditor"))]);

// In your view's render:
div()
    .on_action(cx.listener(|this, _: &FormatDocument, _window, cx| {
        this.editor.update(cx, |editor, cx| {
            let model = editor.model().clone();
            model.update(cx, |model, cx| {
                let formatted = format(&model.text());
                model.set_value(&formatted, cx);
            });
        });
    }))
    .child(self.editor.clone())
```

You can also drive the editor directly. `CodeEditor` exposes many commands as methods: `undo`, `redo`, `fold`, `unfold`, `fold_all`, `go_to_definition`, `type_text`, `set_selections` and others.

## Commands by Monaco id

`editor.trigger(id, window, cx)` runs a command by its Monaco id, as Monaco's `trigger` does:
- **Labelled actions** such as `"editor.action.commentLine"` or `"actions.findWithSelection"`. These are the ones the command palette lists and `run_action` and `supported_actions` know.
- **Unlabelled commands** such as `"cursorDown"`, `"undo"`, `"deleteWordLeft"`, `"closeReferenceSearch"` or `"acceptAlternativeSelectedSuggestion"`.

It falls back to GPUI action names (`"code_editor::CursorDown"`). `DiffEditor::trigger` also takes the diff editor's ids (`"diffEditor.revert"`, `"diffEditor.toggleCollapseUnchangedRegions"`, `"editor.action.accessibleDiffViewer.next"`, …) and passes any others to the modified editor.
