# Language features (providers)

gpui-monaco has no language server or other language intelligence built in. Your app supplies results through provider traits, as with Monaco's `monaco.languages.register*Provider`:

| Trait | Monaco | UI |
|---|---|---|
| `CompletionProvider` | `CompletionItemProvider` | suggest widget: fuzzy filtering, kind icons, details and docs |
| `HoverProvider` | `HoverProvider` | markdown hover on mouse rest |
| `DefinitionProvider` | `DefinitionProvider` | Cmd/Ctrl+hover link, Cmd/Ctrl+click, F12 and Alt+F12 (peek) |
| `ReferenceProvider` | `ReferenceProvider` | Shift+F12 (references in a peek view) |
| `DeclarationProvider` | `DeclarationProvider` | go to and peek declaration |
| `TypeDefinitionProvider` | `TypeDefinitionProvider` | go to and peek type definition |
| `ImplementationProvider` | `ImplementationProvider` | Cmd/Ctrl+F12 and Cmd/Ctrl+Shift+F12 (peek) |
| `RenameProvider` | `RenameProvider` | F2 (rename box) |
| `CodeActionProvider` | `CodeActionProvider` | the lightbulb, Cmd/Ctrl+. and the refactor, source, organize imports, fix all and auto fix commands |
| `SignatureHelpProvider` | `SignatureHelpProvider` | parameter hints while typing a call |
| `DocumentHighlightProvider` | `DocumentHighlightProvider` | occurrence highlights of the symbol at the cursor (textual occurrences without one) |
| `DocumentFormattingProvider` | `DocumentFormattingEditProvider` | Format Document (Shift+Alt+F; Ctrl+Shift+I on Linux) |
| `DocumentRangeFormattingProvider` | `DocumentRangeFormattingEditProvider` | Format Selection (Cmd/Ctrl+K Cmd/Ctrl+F), format on paste, and Format Document without a document formatter |
| `OnTypeFormattingProvider` | `OnTypeFormattingEditProvider` | format on type, after its trigger characters |

Providers are app-wide and registered per language. Each returns a GPUI `Task`, so it can answer right away (`Task::ready`) or asynchronously, for example by forwarding to an LSP server. The [`gpui-monaco-lsp`](lsp.md) crate does that for you.

```rust,ignore
use gpui_monaco::providers::*;

struct MyHover;

impl HoverProvider for MyHover {
    fn provide_hover(
        &self,
        context: &ProviderContext,   // uri, language id and a text snapshot
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let line = context.snapshot.line(position.row);
        Task::ready(Ok(Some(Hover {
            contents: vec![format!("`{}`", line.trim()).into()],
            range: None,   // defaults to the word under the mouse
        })))
    }
}

let registration = register_hover_provider(
    cx,
    LanguageSelector::Languages(vec!["rust".into()]),   // or LanguageSelector::All
    MyHover,
);
// Later: unregister_provider(cx, registration);
```

If several providers match a model, the editor queries all of them. Hover contents and definition locations are concatenated, completion lists are merged, and the first signature help result wins.

## Completion

```rust,ignore
impl CompletionProvider for MyCompletions {
    fn trigger_characters(&self) -> Vec<char> { vec!['.'] }

    fn provide_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        trigger: CompletionTrigger,   // Invoke, TriggerCharacter(c) or Incomplete
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        let mut item = CompletionItem::new("push", CompletionItemKind::Method);
        item.detail = Some("fn push(&mut self, value: T)".into());
        item.documentation = Some("Appends an element.".into());
        Task::ready(Ok(CompletionList { items: vec![item], incomplete: false }))
    }
}
```

Completion is triggered by Ctrl+Space, by trigger characters, or automatically while typing in code, unless `quick_suggestions` is off.

Filtering and ranking use Monaco's `fuzzyScore`, and the editor re-filters as you type without asking the provider again. If you return `incomplete: true`, the editor asks again as the word grows.

On accept, the editor inserts `insert_text`, falling back to the label, over `range`, falling back to the word before the cursor. It then applies any `additional_text_edits`. Typing a character in `commit_characters` accepts the item and inserts the character.

## Go to definition, references and the peek view

Definition, declaration, type definition, implementation and reference providers all return `Location`s:

- **Same model:** `uri: None`, or the model's own uri.
- **Another model:** its uri.

The commands are Monaco's:

| Command | Keys | Method |
|---|---|---|
| Go to Definition | F12, Cmd/Ctrl+click | `go_to_definition` |
| Peek Definition | Alt+F12 (Ctrl+Shift+F10 on Linux) | `peek(LocationKind::Definition)` |
| Go to References | Shift+F12 | `go_to(LocationKind::References)` |
| Go to Implementations | Cmd/Ctrl+F12 | `go_to(LocationKind::Implementation)` |
| Peek Implementations | Cmd/Ctrl+Shift+F12 | `peek(LocationKind::Implementation)` |
| Go to / Peek Declaration, Type Definition, Peek References | (unbound) | `go_to` and `peek` |

Results from every matching provider are merged, sorted by uri and position, and deduplicated. Then:

- **One result:** it's selected, revealed near the top and briefly highlighted (`editor.symbolHighlightBackground`). In another model, the editor emits `CodeEditorEvent::OpenLocation(location)`; open that model in an editor and call `go_to_location(location, cx)` there.
- **Several results:** `options.goto_location` decides, per command, as in Monaco's `gotoLocation.multiple*`:
  - **`Peek`** (the default) opens the peek view.
  - **`GotoAndPeek`** goes to the first result and opens the peek view.
  - **`Goto`** goes to the first; F4 steps through the rest.
- **No results:** an inline message such as "No definition found for 'foo'".
- **Already there:** if the only result is where the cursor is, the command's alternative runs instead. Definition, declaration and type definition fall back to references, so F12 on a definition shows its references.
- **Stale results:** they're dropped if the text or the cursor changed in the meantime.

The peek view (Monaco's references widget) opens under the cursor's line:

- **Preview:** an editable preview of the selected result, with every match in that file highlighted.
- **Results list:** grouped by file, with the match in each line highlighted.
- **List keys:** Up and Down (or F4 and Shift+F4) move through results, Left and Right collapse and expand files, Enter goes to the result and Escape closes the view.
- **Going to a result:** a result in this model moves the peek view there; one elsewhere closes it and emits `OpenLocation`. Double-clicking in the preview always closes it.

To preview other documents, the peek view looks models up by uri with `model_for_uri`. That covers models shown in an editor, models you `register_model`, and whatever your `set_model_resolver` callback returns, for example a model read from disk. Results it can't resolve show "no preview available".

## Signature help

Return a `SignatureHelp` with:

- the signatures
- `active_signature` and `active_parameter`
- the parameter labels, either as substrings of the signature label or as UTF-16 ranges into it

The widget highlights the active parameter and re-queries as the cursor moves. The defaults open it on `(` and `,`; override `trigger_characters` and `retrigger_characters` to change that.

## Formatting

Formatters return `TextEdit`s, which are `(Range<PointUtf16>, String)` pairs in the text's current coordinates. The editor applies them the way Monaco does:

- **Minimal edits:** each edit is reduced to the characters that actually change (Monaco `computeMoreMinimalEdits`). A formatter can return a whole-document replacement and cursors, decorations and markers in unchanged text stay put.
- **One undo step:** the edits are applied as a single undo step, and the primary cursor keeps its place on screen.
- **Cancellation:** if the text or the cursors change before a Format Document or Format Selection result arrives, it's dropped.
- **Format Selection:** formats each selection, and an empty selection formats its line. Ranges whose edits would collide are formatted again as one.
- **Format on type:** with `options.format_on_type`, typing one of `auto_format_trigger_characters` formats at the cursor. It needs a single empty selection, and is dropped if the text at or above the cursor's line changes first.
- **Format on paste:** `options.format_on_paste` formats the pasted range with the range formatter.

Both options are off by default, as in Monaco. You can also call `editor.format_document(cx)` and `editor.format_selection(cx)` directly.

## Rename

F2 (`editor.rename(cx)`) renames the symbol at the cursor, as in Monaco:

1. **Resolve:** providers that implement `resolve_rename_location` say what to rename and its current name; an error's message explains why it can't be renamed. Without one, the word at the cursor is used.
2. **Rename box:** a box under the symbol shows the name, selected. Enter accepts; Escape, clicking elsewhere, or a text change cancels.
3. **Edits:** `provide_rename_edits` returns a `WorkspaceEdit`, which is text edits by document uri (`None` for this one). Providers are asked in order until one answers; if none does, their reasons are shown inline.
4. **Apply:** `editor.apply_workspace_edit(edit, cx)` applies the edits. This document gets one undo step and the cursors follow the edits. Other documents are found with `model_for_uri`. If one can't be found, nothing is applied and "Rename failed to apply edits" is shown.

## Code actions

`provide_code_actions` gets the range (the selection) and a `CodeActionContext`:

- **`only`:** the kind a command wants.
- **`trigger`:** `Invoke` or `Auto`.
- **`markers`:** the markers touching the range.

It returns `CodeAction`s:

- `title` and `kind`. Kinds are hierarchical, like `quickfix` or `refactor.extract`.
- `edit` (a `WorkspaceEdit`) and/or `command`. The edit is applied first, then the command runs through `execute_command`. Register commands with `register_command`.
- `diagnostics`, `is_preferred`, and `disabled` (a reason).
- **Lazy edits:** leave `edit` out and implement `resolve_code_action` to compute it on apply. `provided_kinds` lets commands skip providers that can't help.

As in Monaco:

- **Lightbulb:** 250ms after the cursor rests on code (`options.lightbulb`, default `OnCode`), the editor asks for actions (`Auto`). A lightbulb then shows in the indentation, on a neighboring line, or in the glyph margin. It's blue (`editorLightBulbAutoFix.foreground`) when a preferred quick fix exists. Clicking it opens the menu.
- **Menu:**
  - Actions are grouped under headers: Quick Fix, Extract, Inline, Rewrite, Move, Surround With, Source Action and More Actions.
  - Fixes for diagnostics come first, then preferred actions.
  - Up/Down, Enter and Escape work, and hovering focuses.
  - The focused action's diagnostics are highlighted.
- **Commands:**

| Command | Keys | Asks for | Applies without the menu |
|---|---|---|---|
| Quick Fix (`quick_fix`) | Cmd/Ctrl+. | everything but source actions | never |
| Refactor (`refactor`) | Ctrl+Shift+R | `refactor` | never |
| Source Action (`source_action`) | – | `source` | never |
| Organize Imports (`organize_imports`) | Shift+Alt+O | `source.organizeImports` | if it's the only one |
| Fix All (`fix_all`) | – | `source.fixAll` | if it's the only one |
| Auto Fix (`auto_fix`) | Cmd+Alt+. (Shift+Alt+. elsewhere) | preferred `quickfix` | if it's the only one |

- **No actions:** with none available, a message says so, such as "No code actions available". If the action that would have been applied is disabled, its reason is shown instead.
- **Your own commands:** `trigger_code_actions(filter, auto_apply, message, cx)`.

## Hover from decorations and markers

The hover also shows `hover_message` text from [decorations](decorations.md) and messages from [diagnostics](diagnostics.md) under the mouse, with no provider needed.

The playground's `examples/playground/src/providers.rs` has small demo implementations of every trait.
