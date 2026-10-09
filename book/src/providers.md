# Language features (providers)

gpui-monaco has no language server or other language intelligence built in. Your app supplies results through provider traits, as with Monaco's `monaco.languages.register*Provider`:

| Trait | Monaco | UI |
|---|---|---|
| `CompletionProvider` | `CompletionItemProvider` | suggest widget: fuzzy filtering, kind icons, details and docs |
| `HoverProvider` | `HoverProvider` | markdown hover on mouse rest |
| `DefinitionProvider` | `DefinitionProvider` | Cmd/Ctrl+hover link, Cmd/Ctrl+click and F12 |
| `SignatureHelpProvider` | `SignatureHelpProvider` | parameter hints while typing a call |

Providers are app-wide and registered per language. Each returns a GPUI `Task`, so it can answer right away (`Task::ready`) or asynchronously, for example by forwarding to an LSP server.

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

## Go to definition

Return `Location`s:

- **Same model:** `uri: None`, or the model's own uri. The editor selects the location.
- **Another model:** the editor emits `CodeEditorEvent::OpenLocation(location)`. Open the target model in an editor, then call `editor.go_to_location(location, cx)`.

## Signature help

Return a `SignatureHelp` with:

- the signatures
- `active_signature` and `active_parameter`
- the parameter labels, either as substrings of the signature label or as UTF-16 ranges into it

The widget highlights the active parameter and re-queries as the cursor moves. The defaults open it on `(` and `,`; override `trigger_characters` and `retrigger_characters` to change that.

## Hover from decorations and markers

The hover also shows `hover_message` text from [decorations](decorations.md) and messages from [diagnostics](diagnostics.md) under the mouse, with no provider needed.

The playground's `examples/playground/src/providers.rs` has small demo implementations of every trait.
