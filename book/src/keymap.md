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
- **F8:** next problem
- **F12:** go to definition
- **Ctrl+Space:** trigger completion
- **Tab / Shift+Tab / Escape** in a snippet: next and previous tab stop, leave the snippet
- **Ctrl+Space (or Cmd+I)** in the suggest widget: show or hide the details
- **Shift+Enter / Shift+Tab** in the suggest widget: accept in the other insert mode
- **Tab / Shift+Tab** with `tab_completion: On`: insert the best completion, then cycle through the others

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
