# Language features (providers)

gpui-monaco has no language server or other language intelligence built in. Your app supplies results through provider traits, as with Monaco's `monaco.languages.register*Provider`:

| Trait | Monaco | UI |
|---|---|---|
| `CompletionProvider` | `CompletionItemProvider` | suggest widget: fuzzy filtering, kind icons, details and docs |
| `HoverProvider` | `HoverProvider` | markdown hover on mouse rest |
| `DefinitionProvider` | `DefinitionProvider` | Cmd/Ctrl+hover link with the definition's code in a hover, Cmd/Ctrl+click, F12 and Alt+F12 (peek) |
| `ReferenceProvider` | `ReferenceProvider` | Shift+F12 (references in a peek view) |
| `DeclarationProvider` | `DeclarationProvider` | go to and peek declaration |
| `TypeDefinitionProvider` | `TypeDefinitionProvider` | go to and peek type definition |
| `ImplementationProvider` | `ImplementationProvider` | Cmd/Ctrl+F12 and Cmd/Ctrl+Shift+F12 (peek) |
| `RenameProvider` | `RenameProvider` | F2 (rename box) |
| `NewSymbolNamesProvider` | `NewSymbolNamesProvider` | suggested names in the rename box |
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

Besides `All` and a list of languages, `LanguageSelector::Filters` takes Monaco's `LanguageFilter`s. A filter sets any of a `language`, a uri `scheme` and a glob `pattern` for the path (each can be `*`). A model matches when every field that's set matches:

```rust,ignore
LanguageSelector::Filters(vec![LanguageFilter {
    scheme: Some("file".into()),
    pattern: Some("**/*.test.ts".into()),
    ..Default::default()
}])
```

Some providers can ask for new results without an edit, like Monaco's provider `onDidChange` events: `refresh_folding_ranges(cx)`, `refresh_code_lenses(cx)`, `refresh_inlay_hints(cx)` and `refresh_semantic_tokens(cx)`.

`SignatureHelpProvider::provide_signature_help_with_context` gets Monaco's `SignatureHelpContext`: the trigger, whether hints are already showing or requested (`is_retrigger`), and the hints showing (`active_signature_help`). A `DocumentRangeFormattingProvider` can implement `provide_document_ranges_formatting_edits` to format every selection in one request.

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

Completion is triggered by Ctrl+Space, by trigger characters (`suggest_on_trigger_characters`), or automatically after `quick_suggestions_delay` while typing a word. `quick_suggestions` turns that on separately for code, comments and strings, and can show the items as ghost text instead (`Inline`). By default, in code, the widget waits for inline completions and stays closed while ghost text shows (`OffWhenInlineCompletions`). When no provider has items, the editor suggests words from open documents (`word_based_suggestions`).

The widget shows each item's `label_detail` right after the label and its `label_description` at the right. The focused item's `detail` also shows at the right. Ctrl+Space while the list is open toggles the details panel with the item's `detail` and `documentation`; it is collapsed by default, as in Monaco. Ctrl+Alt+Space moves the focus into the details panel, so the arrow and page keys scroll it and Cmd/Ctrl+C copies it, and back. Drag the list's right or bottom edge to resize it.

With `tab_completion: On`, Tab at the end of a word inserts the best suggestion without showing the list. Tab and Shift+Tab then swap in the next or previous one, until the cursor moves.

Which item is selected first follows `suggest_selection`: the best match, or the item you picked last (for the same prefix). When the list opens by itself, `suggest.selection_mode` can leave it without a focused item (Monaco's `selectionMode`): Enter and Tab then type as usual, and the arrow keys or Ctrl+Space focus one. Cmd/Ctrl+/ shows how the focused item was scored, with the provider's `debug_display_name` (Monaco's explain mode), and "Reset Suggest Widget Size" forgets the resized size.

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
| Open Definition to the Side | Cmd/Ctrl+K F12, Cmd/Ctrl+Alt+click | `reveal_definition_aside` |
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
- **To the side:** Open Definition to the Side emits `CodeEditorEvent::OpenLocationToSide(location)` for the first result, even in this model, so the host can open it in an editor beside this one. (Standalone Monaco has no side-by-side editors and opens it in place; `go_to_location` does that.)
- **Cmd/Ctrl+click:** with `definition_link_opens_in_peek`, the definitions show in a peek view instead.
- **Escape** while a request is pending cancels it (Monaco's `editor.cancelOperation`, also for format, rename, paste and drop requests); `has_cancellable_operation` says whether one is.
- **Definition preview:** resting the mouse on a Cmd/Ctrl+hover link shows the definition's code in the hover, in the target model's language (Monaco's `goToDefinitionAtPosition`). It's the location's range, or for 8 lines or more the lines up to the next one indented like the first, without their common indentation. Several definitions say "Click to show N definitions." Locations in other models need `model_for_uri` to find them. Show Definition Preview Hover (`editor.action.showDefinitionPreviewHover`, `show_definition_preview_hover`, unbound) shows the same at the cursor until it moves or a key is pressed.

The peek view (Monaco's references widget) opens under the cursor's line:

- **Preview:** an editable preview of the selected result, with every match in that file highlighted.
- **Results list:** grouped by file, with the match in each line highlighted.
- **List keys:** Up and Down (or F4 and Shift+F4, and F12) move through results, Left and Right collapse and expand files, Enter goes to the result and Escape closes the view.
- **Focus:** the list has focus when it opens, or the preview with `peek_widget_default_focus: Editor`. Cmd/Ctrl+K F2 moves focus between them.
- **Resizing:** drag the line between the preview and the list, or the view's bottom edge (between 5 and 35 lines). The next peek view, in any editor, keeps the size.
- **Going to a result:** a result in this model moves the peek view there; one elsewhere closes it and emits `OpenLocation`. Double-clicking in the preview always closes it.

Hosts with locations of their own, such as a test explorer or a code lens that lists usages, show them the same way with Monaco's API commands:

- **`go_to_locations(position, locations, multiple, no_results_message, cx)`** (Monaco `editor.action.goToLocations`) moves the cursor to `position` and handles `locations` like a navigation command. `multiple` (a `GoToLocation`) overrides `goto_location.multiple_references`. Without locations, `no_results_message` shows, if given. The peek view's title is "Locations". `goto_location.multiple_tests` (Monaco `gotoLocation.multipleTests`) is there for a host's go to test commands to pass as `multiple`.
- **`peek_locations(position, locations, multiple, cx)`** (Monaco `editor.action.peekLocations`, also `editor.action.showReferences`) always peeks, even one location.
- **By id:** `trigger_with_payload("editor.action.goToLocations", json, …)` takes Monaco's JSON shapes (`{ "uri", "position": { "lineNumber", "column" }, "locations": [{ "uri", "range": { "startLineNumber", … } }], "multiple", "noResultsMessage", "openInPeek" }`, 1-based). `execute_command` and command ids in code lenses, code actions and completions take Monaco's positional arguments (uri, position, locations, multiple, noResultsMessage, openInPeek) and run in the focused editor showing the uri (or the first one). As in standalone Monaco, a uri no editor shows does nothing.

To preview other documents, the peek view looks models up by uri with `model_for_uri`. That covers models shown in an editor, models you `register_model`, and whatever your `set_model_resolver` callback returns, for example a model read from disk. Results it can't resolve show "no preview available".

## Signature help

Return a `SignatureHelp` with:

- the signatures
- `active_signature` and `active_parameter`
- the parameter labels, either as substrings of the signature label or as UTF-16 ranges into it

The widget highlights the active parameter and re-queries as the cursor moves. The defaults open it on `(` and `,`; override `trigger_characters` and `retrigger_characters` to change that. With several signatures, Up/Down or the widget's arrows go through them. `options.parameter_hints` turns opening while typing off (`enabled`) and stops them wrapping around (`cycle`).

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
2. **Rename box:** a box under the symbol shows the name, selected. Enter accepts (`AcceptRenameInput`); Escape (`CancelRenameInput`), clicking elsewhere, or a text change cancels.
   - **Suggestions:** with a `NewSymbolNamesProvider` registered (`register_new_symbol_name_provider`), the box gets a ✨ button that asks every provider for names (`NewSymbolNameTriggerKind::Invoke`, or `request_rename_suggestions`). Providers whose `supports_automatic_new_symbol_names_trigger_kind` is true are asked as soon as the box opens (`Automatic`). New, distinct names are listed above the earlier ones, up to 7 at a time, with ✨ on `AIGenerated` ones. Down and Up (`FocusNextRenameSuggestion`, `FocusPreviousRenameSuggestion`) put a suggestion in the box, and the current name back past either end. Typing stops the request and edits the name instead. Clicking a suggestion renames to it. `rename_suggestions` lists them.
   - **Preview:** Cmd/Ctrl+Enter (`AcceptRenameInputWithPreview`) previews the edits when a preview handler is set (see below); the box then says "Enter to Rename, Cmd/Ctrl+Enter to Preview".
3. **Edits:** `provide_rename_edits` returns a `WorkspaceEdit`, which is text edits by document uri (`None` for this one). Providers are asked in order until one answers; if none does, their reasons are shown inline.
4. **Apply:** `editor.apply_workspace_edit(edit, cx)` applies the edits. This document gets one undo step and the cursors follow the edits. Other documents are found with `model_for_uri`. If one can't be found, nothing is applied and "Rename failed to apply edits" is shown.

### Previewing edits

Standalone Monaco applies a previewed rename or code action right away; VS Code shows them in its Refactor Preview panel first. `set_workspace_edit_preview_handler(cx, handler)` (Monaco's `IBulkEditService.setPreviewHandler`) lets your app do the same. The handler gets the `WorkspaceEdit` and a label (`Renaming 'foo' to 'bar'`, or the code action's title) and returns a task with the edits to apply, or `None` to cancel. It runs for Cmd/Ctrl+Enter in the rename box and the code action menu; Enter never previews. Disposing the returned `Disposable` removes it, and `has_workspace_edit_preview_handler` says whether one is set.

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
- **Documentation:** `documentation` returns `(kind, Command)` pairs (Monaco `CodeActionProviderMetadata.documentation`). The menu's action bar links to the one whose kind best matches what a command asked for (`refactor` for Refactor), or else the first whose kind contains a returned action's. Clicking it runs the command through `execute_command`, or emits `CodeEditorEvent::ExecuteCommand` if none is registered.

As in Monaco:

- **Lightbulb:** 250ms after the cursor rests on code (`options.lightbulb`, default `OnCode`), the editor asks for actions (`Auto`). A lightbulb then shows in the indentation, on a neighboring line, or in the glyph margin. It's blue (`editorLightBulbAutoFix.foreground`) when a preferred quick fix exists. Clicking it opens the menu.
- **Menu:**
  - Actions are grouped under headers: Quick Fix, Extract, Inline, Rewrite, Move, Surround With, Source Action and More Actions.
  - Fixes for diagnostics come first, then preferred actions.
  - Up/Down, Enter and Escape work, and hovering focuses. Cmd/Ctrl+Enter (`CodeActionMenuPreview`, Monaco `previewSelectedCodeAction`) applies through the [preview handler](#previewing-edits); if it cancels, the action's command doesn't run either. Rows' tooltips say "Enter to Apply" (and "Cmd/Ctrl+Enter to Preview" for refactorings, source actions and actions with edits), or why the action is disabled.
  - The focused action's diagnostics are highlighted.
  - **Action bar:** below the list, the providers' documentation links and, for commands asking for a kind (Refactor, Source Action, …) when some actions are disabled, **Show Disabled** / **Hide Disabled**. Disabled actions are listed greyed out and can't be applied; the choice is kept for the next menu. The lightbulb's menu has no action bar. `code_action_menu_bar_actions` and `run_code_action_menu_bar_action` read and click it.
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
- **Your own commands:** `trigger_code_actions(filter, auto_apply, message, cx)`, or Monaco's `editor.action.codeAction` command with its arguments: `kind` (every action with a kind, source actions included, by default), `apply` (`"ifSingle"` by default, `"first"` or `"never"`) and `preferred`. Run it with `trigger_with_payload`, or bind the `TriggerCodeAction` action with those fields.

## Inlay hints

`provide_inlay_hints` gets a range covering the visible lines plus a screen above and below. It returns `InlayHint`s:

- a `position` and `label`: text, or `InlayHintLabelPart`s, each with its own `tooltip`, `location` and `command`;
- a `kind` (`Type` or `Parameter`), which picks the `editorInlayHint.*` colors;
- `padding_left` and `padding_right`;
- a `tooltip`;
- `text_edits`, applied when the hint is double-clicked.

`resolve_inlay_hint` can fill in a hint's tooltip, label or edits before its hover shows or it's clicked; it runs once per hint.

The editor shows them as injected text that moves with edits. As in Monaco:

- **Requests:** made again when you scroll past the covered lines, shortly after edits, and when you call `refresh_inlay_hints(cx)`.
- **Hover:** resting the mouse on a part shows the hint's and the part's tooltips, how to follow the part, and "Double-click to insert" when the hint has edits.
- **Clicks:** Cmd/Ctrl+click on a part opens its `location` like go to definition (other models through `CodeEditorEvent::OpenLocation`), or else runs its `command` (through `register_command`, else `CodeEditorEvent::ExecuteCommand`).
- **Options:** `options.inlay_hints.enabled` is `On`, `OnUnlessPressed` or `OffUnlessPressed` (Ctrl+Alt toggles), or `Off`. Line text is cut at `maximum_length` (43) with `…`. `font_size` and `font_family` change the hints' font.

Unlike Monaco, a part has no right-click menu, and its hover doesn't include the hover of its location.

## Document symbols

A `DocumentSymbolProvider` returns a document's outline as nested `DocumentSymbol`s (name, `SymbolKind`, range and selection range). `CodeEditor::document_symbols` returns them, merged across providers. Without a provider they come from the language's `tags.scm` query. The bundled languages have one for Rust, JavaScript, TypeScript, Markdown, JSON and CSS.

## Code lens

A `CodeLensProvider` puts clickable commands above lines, such as "3 references" or "Run test". `provide_code_lenses` can return lenses without a `command`; `resolve_code_lens` then fills them in when they scroll into view. Lenses on the same line show together, separated by `|`.

Clicking a lens runs its command through [`register_command`](editing.md) handlers. If no handler is registered, the editor emits `CodeEditorEvent::ExecuteCommand` for the host. "Show CodeLens Commands for Current Line" (Monaco's `codelens.showLensesInCurrentLine`) lists the cursor line's commands in the quick input instead. The `code_lens`, `code_lens_font_family` and `code_lens_font_size` options control them.

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
            ..Default::default()
        }))
    }
}
```

Typing asks for completions after a short pause. A completion keeps showing, without a new request, while what you type matches it. The rest shows as faded italic text after the cursor, and lines after the first go in a zone below.

To accept:
- Tab takes it all.
- Cmd/Ctrl+Right takes the next word, and `AcceptNextInlineLine` the next line.

Alt+] and Alt+[ show the other completions, and Alt+\\ asks explicitly. Escape or moving the cursor hides it.

Hovering the ghost text shows Monaco's toolbar: previous and next with a "1/3" count, Accept, Accept Word, and a "⋯" menu with Accept Line, Always Show Toolbar and the list's `commands`. With `inline_suggest.show_toolbar: Always` it stays above the ghost text instead.

Ghost text is colored like code (`syntax_highlighting_enabled`, faded to 70%). The toolbar's count needs all completions, so showing it asks explicitly.

Providers can also:
- hear how each completion ended through `handle_end_of_lifetime` (accepted, rejected with Escape, or ignored) and `handle_rejection`;
- set a `group_id`, and list in `yields_to_group_ids` the groups they defer to. A provider is asked only after those groups answer, and not at all if one of them has a visible completion.

The `inline_suggest` options control this; see [Options](options.md). With `suggest.preview`, the suggest widget's selected item shows as ghost text too.

### Inline edits

A completion with `is_inline_edit` is an inline edit, Monaco's "next edit suggestion": it can change text anywhere, not only at the cursor. Its `range` may span lines:

```rust,ignore
InlineCompletion {
    insert_text: "let color = 1;".into(),
    range: Some(PointUtf16::new(0, 0)..PointUtf16::new(0, 15)),
    is_inline_edit: true,
    ..Default::default()
}
```

The editor diffs `range` against `insert_text` and shows the changes where they are, the way Monaco picks:
- a changed word in a box below it, and a changed line in a box below the line;
- inserted text in place, and inserted lines pushing the text below down;
- deleted text marked in place;
- several changed lines in a card beside them, if it fits (else below them).

An inline edit shows only while no completion is visible at the cursor. It follows typing: typing its own text moves it along, and typing inside it drops it. Once other typing moves it, it shows collapsed, as a small marker, until you jump to it (or always, with `inline_suggest.edits.show_collapsed`).

A pill in the gutter marks it, or points up or down while it's scrolled away. Tab jumps to it when the cursor is away from it, and accepts it once there. Escape rejects it. Clicking the pill accepts it (or jumps there when it points away), and hovering it shows a menu: Go To / Accept, Reject, Show Collapsed, the list's `commands`, and the item's `gutter_menu_link_action`. `display_name` titles the menu.

`show_range` limits where any item shows: only while the cursor is in that range. Providers see in `InlineCompletionContext::include_inline_edits` whether inline edits are wanted (`inline_suggest.edits.enabled`).

While an inline edit is scrolled away, a box near the cursor previews its line (with `inline_suggest.edits.long_distance_hint_context_line_count` lines around it) and says "Tab to jump"; clicking it jumps too. `show_long_distance_hint` turns it off. The pill beside the edit shows its line number, like the line numbers it covers, and pulses while you hover the edit. Accepted text fades out over a second, unless `App::reduce_motion` is set.

Other kinds of items are inline edits too:
- `jump_to_position` suggests moving the cursor rather than editing: a "Tab to jump" label marks the place, and jumping there accepts it.
- `hint` shows `content` at its `range` instead of the diff (as code or as a plain label, per its `style`). Tab jumps to the hint, then accepts the edit, leaving the cursor where it is.
- `uri` names another model the edit applies to. The editor diffs it against that model (found with `model_for_uri`, or your model resolver) and shows the long-distance hint, with the file's name and "Tab to open". Tab, or accepting, emits `CodeEditorEvent::OpenLocation` for you to open or focus an editor for that model, and moves the edit there: right away if one is open, else when its editor first shows. Accept it there as usual.

An `alternative_action` is a second way to take an inline edit, like VS Code's "Rename" for a changed symbol. The edit shows as a word replacement with the action's label beside it; Shift+Tab, clicking the label or the gutter menu's entry runs its command instead of the edit (`editor.action.inlineSuggest.commitAlternativeAction`), and `handle_end_of_lifetime` gets `Accepted { alternative_action: true }`.

With `show_inline_edit_menu`, ghost text gets the gutter pill and menu as well.

Undo and redo work as in Monaco: undoing an inline edit you just accepted doesn't ask for it again; undoing back to where one showed asks only for inline edits (`include_inline_completions` is false), to show it again; and undo, redo and accepting a word keep the completion that was showing. Accepting asks again even when the text didn't change.

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

## Drop and paste edits

Monaco's `DocumentDropEditProvider` and `DocumentPasteEditProvider` decide how dropped files or pasted text get inserted. A provider returns `DocumentDropOrPasteEdit`s, each with:

- a title for the picker;
- a hierarchical `kind`, such as `text.plain` or `uri.path.absolute`;
- the text or snippet to insert;
- additional edits;
- `yield_to`, the edits it should rank below.

The editor has built-in providers for plain text, paths and HTML. When other providers handle a paste, the first edit is applied. If there are alternatives, a button (or Cmd/Ctrl+.) switches between them. "Paste As..." (`paste_as`) lists every edit.

A paste provider can also prepare data when text is copied or cut, as Monaco's `prepareDocumentPaste` does (with `paste_as.enabled` on). `prepare_document_paste` gets the copied ranges (whole lines for empty selections) and returns a `DataTransfer`, for example the imports the copied code needs; `copy_mime_types` names its MIME types. When that copy is pasted, in this editor or another, the paste waits for the data, adds it to the paste's `DataTransfer` (higher priority providers winning) and asks the providers that handle it. Text copied elsewhere, or before the latest copy, has no such data.

```rust
register_document_paste_edit_provider(cx, LanguageSelector::All, MyProvider);
```

## Hover from decorations and markers

The hover also shows `hover_message` text from [decorations](decorations.md) and messages from [diagnostics](diagnostics.md) under the mouse, with no provider needed.

Each part of the hover has a copy button, and the hover can be resized from its corner.

## Hover verbosity

A provider can offer more or less detail, like Monaco's `HoverVerbosityRequest`. Set `can_increase_verbosity` or `can_decrease_verbosity` on the `Hover`, and the part shows + and − controls. Clicking one, or running `editor.action.increaseHoverVerbosityLevel` / `decreaseHoverVerbosityLevel`, calls `provide_hover_with_context` with the change and the previous hover:

```rust,ignore
impl HoverProvider for MyHover {
    fn provide_hover(&self, context: &ProviderContext, position: PointUtf16, cx: &mut App)
        -> Task<anyhow::Result<Option<Hover>>> {
        self.provide_hover_with_context(context, position, &HoverContext::default(), cx)
    }

    fn provide_hover_with_context(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        hover_context: &HoverContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let level = hover_context
            .verbosity_request
            .as_ref()
            .map_or(0, |request| request.verbosity_delta);
        Task::ready(Ok(Some(Hover {
            contents: vec![format!("detail level {level}").into()],
            can_increase_verbosity: level < 2,
            can_decrease_verbosity: level > 0,
            ..Default::default()
        })))
    }
}
```

The playground's `examples/playground/src/providers.rs` has small demo implementations of every trait.
