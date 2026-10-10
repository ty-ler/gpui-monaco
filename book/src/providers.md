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
| `SelectionRangeProvider` | `SelectionRangeProvider` | expand and shrink selection |
| `FoldingRangeProvider` | `FoldingRangeProvider` | folding ranges, with kinds (comment, imports, region) |
| `InlayHintsProvider` | `InlayHintsProvider` | inline type and parameter hints |
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

Completion is triggered by Ctrl+Space, by trigger characters (`suggest_on_trigger_characters`), or automatically after `quick_suggestions_delay` while typing a word. `quick_suggestions` turns that on separately for code, comments and strings. When no provider has items, the editor suggests words from open documents (`word_based_suggestions`).

The widget shows each item's `label_detail` right after the label and its `label_description` at the right. The focused item's `detail` also shows at the right. Ctrl+Space while the list is open toggles the details panel with the item's `detail` and `documentation`; it is collapsed by default, as in Monaco. Drag the list's right or bottom edge to resize it.

With `tab_completion: On`, Tab at the end of a word inserts the best suggestion without showing the list. Tab and Shift+Tab then swap in the next or previous one, until the cursor moves.

Which item is selected first follows `suggest_selection`: the best match, or the item you picked last (for the same prefix).

Filtering and ranking use Monaco's `fuzzyScore`, and the editor re-filters as you type without asking the provider again. If you return `incomplete: true`, the editor asks again as the word grows.

On accept, the editor inserts `insert_text`, falling back to the label. It is expanded as a [snippet](editing.md#snippets) if `insert_text_is_snippet` is set, and its lines are indented like the cursor's line unless `keep_whitespace` is set. In insert mode it replaces `range` (by default the word before the cursor); in replace mode (`suggest.insert_mode`) it replaces `replace_range` (by default the whole word). Shift+Enter and Shift+Tab accept in the other mode. The editor applies any `additional_text_edits`, then runs the item's `command`. Typing a character in `commit_characters` accepts the item and inserts the character.

`resolve_completion_item` fills in an item lazily, e.g. its docs or an import. The editor resolves the focused item. If you accept an item before it resolves, additional edits that arrive later are still applied, unless the text changed before them. Items tagged `CompletionItemTag::Deprecated` are struck through, or hidden with `suggest.show_deprecated: false`.

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

## Inlay hints

`provide_inlay_hints` gets a range covering the visible lines plus a screen above and below. It returns `InlayHint`s:

- a `position` and `label`;
- a `kind` (`Type` or `Parameter`), which picks the `editorInlayHint.*` colors;
- `padding_left` and `padding_right`;
- a `tooltip`.

The editor shows them as injected text that moves with edits. As in Monaco:

- **Requests:** made again when you scroll past the covered lines, shortly after edits, and when you call `refresh_inlay_hints(cx)`.
- **Options:** `options.inlay_hints.enabled` is `On`, `OnUnlessPressed` or `OffUnlessPressed` (Ctrl+Alt toggles), or `Off`. Line text is cut at `maximum_length` (43) with `…`.

Unlike Monaco, hints use the editor's font size, and their label parts, tooltips and edits aren't interactive yet.

## Document symbols

A `DocumentSymbolProvider` returns a document's outline as nested `DocumentSymbol`s (name, `SymbolKind`, range and selection range). `CodeEditor::document_symbols` returns them, merged across providers. Without a provider they come from the language's `tags.scm` query. The bundled languages have one for Rust, JavaScript, TypeScript, Markdown, JSON and CSS.

## Code lens

A `CodeLensProvider` puts clickable commands above lines, such as "3 references" or "Run test". `provide_code_lenses` can return lenses without a `command`; `resolve_code_lens` then fills them in when they scroll into view. Lenses on the same line show together, separated by `|`.

Clicking a lens runs its command through [`register_command`](editing.md) handlers. If no handler is registered, the editor emits `CodeEditorEvent::ExecuteCommand` for the host. The `code_lens`, `code_lens_font_family` and `code_lens_font_size` options control them.

## Linked editing

With `linked_editing` on, ranges that belong together are edited together. In HTML, renaming an element's start tag renames its end tag. Without a provider, tag pairs come from the syntax tree. A `LinkedEditingRangeProvider` supplies ranges for anything else, optionally with a `word_pattern` that ends linked editing when the text no longer matches it.

The linked ranges are highlighted (`editor.linkedEditingBackground`). Escape ends it, and Cmd/Ctrl+Shift+F2 starts it even with the option off.

## Inline completions

An `InlineCompletionsProvider` offers ghost text at the cursor, such as AI completions:

```rust,ignore
impl InlineCompletionsProvider for MyCompletions {
    fn provide_inline_completions(&self, context: &ProviderContext, position: PointUtf16,
        completion_context: InlineCompletionContext, cx: &mut App)
        -> Task<anyhow::Result<InlineCompletionList>> {
        // Replace from the line start with the whole suggested line.
        Task::ready(Ok(InlineCompletionList {
            items: vec![InlineCompletion {
                insert_text: "println!(\"hi\");".into(),
                range: Some(PointUtf16::new(position.row, 0)..position),
                ..Default::default()
            }],
            suppress_suggestions: None,
        }))
    }
}
```

Typing asks for completions after a short pause. A completion keeps showing, without a new request, while what you type matches it. The rest shows as faded italic text after the cursor, and lines after the first go in a zone below.

To accept:
- Tab takes it all.
- Cmd/Ctrl+Right takes the next word, and `AcceptNextInlineLine` the next line.

Alt+] and Alt+[ show the other completions, and Alt+\\ asks explicitly. Escape or moving the cursor hides it.

The `inline_suggest` options control this; see [Options](options.md). With `suggest.preview`, the suggest widget's selected item shows as ghost text too.

## Semantic tokens

A `DocumentSemanticTokensProvider` colors identifiers by meaning, such as parameters, read-only variables and macros. It works as in LSP: a legend of token types and modifiers, plus five numbers per token. It can return all tokens, or edits against the result it returned last (`last_result_id`). A `DocumentRangeSemanticTokensProvider` colors the visible lines until a full result arrives.

The editor asks for the whole document right away, and again after a pause in typing. Meanwhile the tokens move with the text, as in Monaco. Call `refresh_semantic_tokens` when they change without an edit.

Themes style the tokens:
- VS Code themes through `semanticTokenColors`, else their TextMate rules, using VS Code's default scope for each token type.
- Monaco-style themes match `type.modifier` against their rules.

Only the properties a style sets override the syntax highlighting. Semantic coloring is on when `semantic_highlighting` says so: `ConfiguredByTheme` (the default) follows the theme's `semanticHighlighting`, which Monaco's built-in themes leave off.

## Links

The editor finds URLs (`http`, `https` and `file`) itself, using Monaco's link computer, and underlines them. A `LinkProvider` adds other links, such as issue numbers. Where links overlap, the later provider's link wins over the built-in one.

```rust,ignore
impl LinkProvider for Issues {
    fn provide_links(&self, context: &ProviderContext, cx: &mut App)
        -> Task<anyhow::Result<Vec<DocumentLink>>> { /* … */ }

    // Optional: fill in `url` when the link is followed.
    fn resolve_link(&self, link: &DocumentLink, cx: &mut App)
        -> Option<Task<anyhow::Result<DocumentLink>>> { None }
}
```

Holding Cmd/Ctrl over a link highlights it. Cmd/Ctrl+click, or the `OpenLink` action, emits `CodeEditorEvent::OpenLink(url)`; the editor never opens anything itself. Links are recomputed a second after edits. Turn them off with the `links` option.

## Colors

`register_color_provider` registers a `DocumentColorProvider` (Monaco `registerColorProvider`). It has two jobs:

- `provide_document_colors` returns the colors and their ranges, with components in `0..=1`.
- `provide_color_presentations` returns the ways to write a color, such as `rgb(…)` or `#rrggbb`.

The editor draws a swatch before each color, up to `color_decorators_limit`.

Without color providers, a built-in finder decorates hex, `rgb()`, `rgba()`, `hsl()` and `hsla()` colors in any language. `default_color_decorators` controls it:

- **Auto:** only when no provider found colors.
- **Always:** alongside providers.
- **Never:** off.

Hovering a color, or clicking its swatch, opens the color picker; `color_decorators_activated_on` picks which.
- Letting go of the saturation box or a strip writes the color using the provider's first presentation, or the one written like the original text.
- Clicking the header cycles through the presentations, and clicking the original color restores it.
- `ShowOrFocusStandaloneColorPicker` opens a standalone picker at the cursor. Enter inserts its color (or replaces the color under the cursor), and Escape closes it.

The `gpui_monaco::color` module has Monaco's `Color` for conversions and CSS formatting.

## Hover from decorations and markers

The hover also shows `hover_message` text from [decorations](decorations.md) and messages from [diagnostics](diagnostics.md) under the mouse, with no provider needed.

The playground's `examples/playground/src/providers.rs` has small demo implementations of every trait.
