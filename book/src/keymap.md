# Keybindings and commands

Every editor command is a GPUI action in `gpui_monaco::actions`:

- `CursorWordLeft`, `DeleteLines` and `MoveLinesUp`
- `AddSelectionToNextFindMatch` and `StartFind`
- `ToggleFold`, `GoToDefinition` and `TriggerSuggest`
- …and so on

`gpui_monaco::init(cx)` binds Monaco's default keys for the current platform. On macOS, for example:

- **Cmd+D:** add the next match to the selection
- **Alt+Up / Alt+Down:** move lines, reindenting them by the language's rules (`auto_indent: Full`)
- **Alt+Backspace:** delete the word to the left, or both characters of an auto-closed pair around the cursor
- **F7 / Shift+F7:** move to the next / previous highlighted occurrence
- **Cmd+Alt+[ / Cmd+Alt+]:** fold and unfold
- **Alt+F8 / F8:** next problem (Monaco's `marker.next` and `marker.nextInFiles`, which act the same within one model)
- **Cmd+Shift+, / Cmd+Shift+.:** replace the value at the cursor with the previous or next one (`1` → `2`, `true` → `false`)
- **Ctrl+B / Ctrl+F / Ctrl+P / Ctrl+N, Ctrl+A / Ctrl+E (with Shift to select), Ctrl+H / Ctrl+D:** Monaco's Emacs-style keys
- **F12:** go to definition
- **Cmd+K F12:** open the definition to the side (`CodeEditorEvent::OpenLocationToSide` for the host)
- **Cmd+K F2** with a peek view open: move focus between its preview and its results list
- **Enter / Cmd+Enter / Escape** in the rename box: rename, rename with a preview, cancel; **Up / Down** pick a suggested name
- **Enter / Cmd+Enter** in the code action menu: apply, apply with a preview
- **Ctrl+Space (or Alt+Esc, Cmd+I):** trigger completion
- **F1 / Ctrl+G / Cmd+Shift+O:** command palette, go to line, go to symbol
- **Shift+F10:** context menu
- **Tab / Cmd+Right / Alt+] / Alt+[** with ghost text: accept, accept a word, next, previous (asking explicitly, `editor.action.inlineSuggest.trigger`, has no key, as in Monaco)
- **Tab / Escape** with an inline edit: jump to it while the cursor is away from it (or open its file, for a cross-file edit), then accept it; reject it
- **Shift+Tab** with an inline edit that has an alternative action: run it instead of the edit (`editor.action.inlineSuggest.commitAlternativeAction`)
- **Tab / Shift+Tab / Escape** in a snippet: next and previous tab stop, leave the snippet
- **Ctrl+Space (or Cmd+I)** in the suggest widget: show or hide the details, or focus the first suggestion if none is (Monaco's `focusSuggestion`, see `suggest.selection_mode`)
- **Ctrl+Alt+Space** in the suggest widget: move the focus to the details panel and back (Monaco's `toggleSuggestionFocus`); there, Up / Down and PageUp / PageDown scroll it and Cmd/Ctrl+C copies it
- **Ctrl+Enter** in the find or replace input: insert a line break
- **Alt+Enter** while the find widget shows (from the editor or its inputs): select every match, keeping the widget open and focusing the editor
- In the find widget: **Enter / Shift+Enter** in the find input go to the next and previous match; **Enter** in the replace input replaces, and on macOS **Cmd+Enter** there replaces all (Shift+Enter there inserts a line break, as Monaco's textarea does). **Cmd/Ctrl+Down** in either input returns the focus to the editor. **Tab / Shift+Tab** move through the inputs, toggles and buttons in Monaco's order, and **Space / Enter** press the focused one
- **Cmd/Ctrl+D, Cmd/Ctrl+K Cmd/Ctrl+D, Cmd/Ctrl+Shift+L** in the find input: act on the editor, searching for the find widget's string (Monaco's `editorFocus`). From the editor they make their text the find widget's search string, so F3 finds it
- **Cmd+/** in the suggest widget: explain mode, which shows how the focused suggestion was scored in the details (Monaco's `toggleExplainMode`)
- **Shift+Enter / Shift+Tab** in the suggest widget: accept in the other insert mode
- **Up / Down (or Cmd+Up / Cmd+Down, Ctrl+P / Ctrl+N), PageUp / PageDown** in the suggest widget: previous and next suggestion, by item or page
- **Cmd+K Cmd+I** with a hover showing: focus it. Arrows scroll it, PageUp / PageDown (or Alt+Up / Alt+Down) page, Home / End (or Cmd+Up / Cmd+Down) go to the top and bottom, and Escape closes it
- **Tab / Shift+Tab** with `tab_completion: On`: insert the best completion, then cycle through the others
- **Cmd+Shift+Space:** parameter hints. While they show, **Escape / Shift+Escape** close them, and with several signatures **Up / Down (or Alt+Up / Alt+Down, Ctrl+P / Ctrl+N)** go to the previous and next one
- **Shift+Escape:** Monaco's second Escape. It closes the suggest widget, parameter hints, the problem zone, the reference peek and the find widget, leaves snippet mode and linked editing, then removes the other cursors and the selection. Unlike Escape, it leaves ghost text, the hover, inline messages and the selection anchor alone
- **Ctrl+Left / Ctrl+Right** on Windows with a screen reader: Monaco's accessibility word navigation (`cursorWordAccessibilityLeft`/`Right`)

Tab focus mode (Ctrl+M; Ctrl+Shift+M on macOS) makes Tab and Shift+Tab move focus out of the editor, for keyboard and screen reader users.

Font zoom (`FontZoomIn`, `FontZoomOut`, `FontZoomReset`) has no default key, as in Monaco. Run it from the command palette, or bind it yourself. The zoom level is shared by every editor; `set_editor_zoom_level` sets it directly.

All of these bindings live in the `CodeEditor` key context (`actions::KEY_CONTEXT`). As in Monaco, an action's precondition is part of its keybindings: in a read-only editor Cmd+/ does nothing (the key reaches your app), F12 needs a definition provider, and Shift+Alt+O needs a code action provider declaring `source.organizeImports`. `trigger` and the command palette follow the same preconditions.

## Context keys

The editor sets Monaco's context keys on its key context, so `KeybindingRule::when` and your own `KeyBinding`s can test them with GPUI's predicate syntax:

```rust,ignore
KeybindingRule::new("cmd-k cmd-r", "my.runSnippet")
    .when("CodeEditor && editorTextFocus && !editorReadonly && editorLangId == rust")
```

They include Monaco's `EditorContextKeys` (`editorTextFocus`, `editorFocus`, `editorReadonly`, `editorLangId`, `editorHasSelection`, `editorHasMultipleSelections`, `editorHas…Provider`, `editorSelectionHasDiagnostics`, `canUndo`, `isComposing`, …) and the contributions' keys (`suggestWidgetVisible`, `suggestWidgetHasFocusedSuggestion`, `inSnippetMode`, `hasNextTabstop`, `messageVisible`, `hasWordHighlights`, `renameInputVisible`, `LinkedEditingInputVisible`, `findWidgetVisible`, `findInputFocussed`, `referenceSearchVisible`, `selectionAnchorSet`, `foldingEnabled`, `markersNavigationVisible`, `parameterHintsVisible`, `inlineSuggestionVisible`, …). `CodeEditor::context_keys(cx)` returns them all. Boolean keys are present while true; string and number keys are `key = value` entries, which GPUI compares as identifiers. Monaco's `supportedCodeAction` lists the code action providers' kinds separated by spaces, which predicates can't match, so each kind the built-in actions test also has its own key (`supportedCodeAction_quickfix`, `supportedCodeAction_refactor`, `supportedCodeAction_source`, `supportedCodeAction_source_organizeImports`, `supportedCodeAction_source_fixAll`).

Your own keys work like Monaco's `createContextKey`:

```rust,ignore
let in_review = editor.update(cx, |editor, cx| editor.create_context_key("inReview", false, cx));
in_review.set(true, cx); // `CodeEditor && inReview` bindings now apply
in_review.reset(cx);
```

The editor also keeps its older snake_case keys (`single_line`, `suggest_widget_visible`, `parameter_hints_visible`, `inline_suggestion_visible`, …), which the default bindings use.

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

Monaco's per-editor `addCommand(keybinding, handler, context)` and `addAction` bind keys in that editor only, until the returned `Disposable` is disposed:

```rust,ignore
let (id, command) = editor.update(cx, |editor, cx| {
    editor.add_command(
        "cmd-k cmd-m",
        |editor, window, cx| { /* … */ },
        Some("inReview"), // Monaco's `context`: a predicate over the context keys
        cx,
    )
})?;
editor.update(cx, |editor, cx| editor.trigger(&id, window, cx))?; // also by its id

let action = editor.update(cx, |editor, cx| {
    editor.add_action(
        EditorActionDescriptor::new("my.upper", "Make Upper", |editor, window, cx| { /* … */ })
            .keybinding("ctrl-alt-u"),
        cx,
    )
});
action.dispose(cx); // gone from the palette, the context menu and the keymap
```

You can also use GPUI's usual pattern: define an action, bind it in the editor's context, and handle it on an element that contains the editor. While the editor has focus, its key context is part of the dispatch path, and actions bubble up to your handler.

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

## Global commands, actions and keybinding rules

Monaco's global `monaco.editor.addCommand`, `addEditorAction` and `addKeybindingRule(s)` are free functions. Each returns a `Disposable` whose `dispose(cx)` removes what it added; dropping it keeps the registration.

```rust,ignore
use gpui_monaco::{EditorActionDescriptor, KeybindingRule};

// A command, for keybindings and providers' `Command`s. Handlers for one id
// stack: disposing the newest brings back the one before.
let command = gpui_monaco::add_command(cx, "my.save", |args, cx| save(args, cx));

// An action in every editor, now and later: by id, keybinding and context
// menu (not the command palette, as in Monaco).
let action = gpui_monaco::add_editor_action(
    cx,
    EditorActionDescriptor::new("my.upper", "Make Upper", |editor, window, cx| { /* … */ })
        .keybinding("ctrl-alt-u")
        .context_menu("1_modification", 1.5),
);

// Keystrokes for a command: an editor action, a Monaco command such as
// "cursorDown", a registered command (with its argument) or a GPUI action.
let rules = gpui_monaco::add_keybinding_rules(
    cx,
    vec![
        KeybindingRule::new("cmd-s", "my.save").args(serde_json::json!({ "all": false })),
        KeybindingRule::unbind("cmd-k cmd-x"), // Monaco `command: null`
    ],
)?;

rules.dispose(cx); // removes exactly those bindings
```

A rule's `when` is a GPUI key context predicate; without one it applies in the editor (`"CodeEditor"`). Rules are bound to the `RunCommand` action, which the focused editor runs.

## Commands by Monaco id

`editor.trigger(id, window, cx)` runs a command by its Monaco id, as Monaco's `trigger` does. Every command Monaco registers for the editor resolves (a test checks the list from Monaco's source):
- **Labelled actions** such as `"editor.action.commentLine"` or `"actions.findWithSelection"`. These are the ones the command palette lists and `run_action` and `supported_actions` know.
- **Unlabelled commands** such as `"cursorDown"`, `"undo"`, `"deleteWordLeft"`, `"closeReferenceSearch"`, `"acceptSelectedSuggestion"`, `"leaveSnippet"`, `"cancelLinkedEditingInput"`, `"leaveEditorMessage"` or `"acceptRenameInput"`.
- **Handler commands** such as `"type"`, `"paste"`, `"cut"`, `"compositionStart"` and their `default:` ids.
- **Aliases** such as `"editor.action.goToDeclaration"`, `"editor.action.previewDeclaration"` or `"acceptSelectedSuggestionOnEnter"`.
- **Your commands** from `add_command`, `register_command` and `add_editor_action`.

A command whose precondition fails does nothing, as in Monaco.

It falls back to GPUI action names (`"code_editor::CursorDown"`). `DiffEditor::trigger` also takes the diff editor's ids (`"diffEditor.revert"`, `"diffEditor.switchSide"`, `"diffEditor.toggleCollapseUnchangedRegions"`, `"editor.action.accessibleDiffViewer.next"`, …) and passes any others to the modified editor.

`trigger_with_payload(id, payload, window, cx)` passes Monaco's payload, as JSON, to commands that take arguments:

```rust,ignore
editor.trigger_with_payload(
    "editor.actions.findWithArgs",
    serde_json::json!({ "searchString": "todo", "isCaseSensitive": true }),
    window,
    cx,
)?;
editor.trigger_with_payload(
    "editor.action.codeAction",
    serde_json::json!({ "kind": "refactor.extract", "apply": "first" }),
    window,
    cx,
)?;
editor.trigger_with_payload("type", serde_json::json!({ "text": "hello" }), window, cx)?;
editor.trigger_with_payload(
    "cursorMove",
    serde_json::json!({ "to": "down", "by": "wrappedLine", "value": 3, "select": true }),
    window,
    cx,
)?;
editor.trigger_with_payload("editor.fold", serde_json::json!({ "levels": 2, "selectionLines": [0] }), window, cx)?;
editor.trigger_with_payload("revealLine", serde_json::json!({ "lineNumber": 40, "at": "center" }), window, cx)?;
```

The same arguments are the fields of the actions (`FindWithArgs`, `TriggerCodeAction`, `CursorMove`, `EditorScroll`, `Fold`, `Refactor`, `ShowHover`, …), which can be bound to keys, and of `RunCommand`'s `args` in keybinding rules.

## Edit commands

Monaco's `executeCommand(source, command)` runs an `ICommand`: it adds its edits for the text as it is, then says where the cursor goes. Implement `EditorCommand`:

```rust,ignore
struct Wrap(Selection, Option<TrackedSelection>);

impl EditorCommand for Wrap {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditOperationBuilder) {
        let range = self.0.range();
        builder.add_edit_operation(range.start..range.start, "(");
        builder.add_edit_operation(range.end..range.end, ")");
        self.1 = Some(builder.track_selection(self.0, None));
    }

    fn compute_cursor_state(&self, data: &CursorStateComputerData) -> Selection {
        data.tracked_selection(self.1.unwrap())
    }
}

editor.update(cx, |editor, cx| {
    let selection = editor.selections()[0];
    editor.execute_command(Some("my-extension"), Wrap(selection, None), cx);
});
```

`execute_commands` runs one command per cursor (`None` leaves one alone) as a single edit.
