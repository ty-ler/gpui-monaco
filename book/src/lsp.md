# Language servers (LSP)

The `gpui-monaco-lsp` crate connects [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) servers to the editor, the way `monaco-languageclient` does for Monaco. An `LspClient` starts a server and then does three things:

- It registers [providers](providers.md) for the features the server supports, such as completion, hover, go to definition, code actions, code lenses, links, colors and linked editing (see [Features](#features)).
- It turns the server's diagnostics into [markers](diagnostics.md).
- It keeps the models you attach in sync with the server.

```rust,ignore
use gpui_monaco_lsp::{LanguageServerBinary, LspClient, LspClientOptions};

let model = cx.new(|_| {
    TextModel::new(&text)
        .with_language("rust")
        .with_uri("file:///home/me/project/src/main.rs")
});
let start = LspClient::start(
    LanguageServerBinary { path: "rust-analyzer".into(), ..Default::default() },
    LspClientOptions {
        name: "rust-analyzer".into(),
        languages: vec!["rust".into()],
        root_uri: Some("file:///home/me/project".into()),
        ..Default::default()
    },
    cx,
);
cx.spawn(async move |cx| {
    let client = start.await?;
    client.update(cx, |client, cx| client.attach(&model, cx))??;
    anyhow::Ok(())
})
.detach();
```

Try it in the playground:

```bash
PLAYGROUND_LSP=rust-analyzer PLAYGROUND_FILE=src/main.rs cargo run -p playground
```

## Documents

- **`attach(model)`:** sends `textDocument/didOpen`, then sends each edit as an incremental `didChange`, or the full text if the server asks for that. The model is opened with its language id and registered (`register_model`), so the server's edits and locations find it.
- **Document uris:** a document is named by the model's `resource()`, as `monaco-languageclient` sends `model.uri.toString()`. That is the uri in Monaco's normalized form: `file:///my project/a.rs` is sent as `file:///my%20project/a.rs`, and `file:///C:/a.ts` as `file:///c%3A/a.ts`. A model without a uri is sent as `inmemory://model/N`. Uris from the server are normalized too before they're compared, so a server that writes them its own way still reaches the right model.
- **Before each request:** the bridge sends any edits the server hasn't seen yet, so a completion right after typing sees the new text.
- **Closing:** `detach(model)`, or releasing the model, sends `didClose`. Detaching also clears the server's markers.
- **Saving:** before writing the file, call `will_save(model, reason)`, which sends `willSave` if the server wants it, and await `will_save_wait_until(model, reason)`, which asks for the server's edits (organize imports or fix all on save, for example) and applies them to the model as one undo step. A server that fails or takes longer than `WILL_SAVE_WAIT_UNTIL_TIMEOUT` (1.5s, VS Code's budget for a save participant) doesn't hold up the save; its edits are dropped, and so are edits that come after the document changed. After writing, `did_save(model)` sends `didSave` if the server wants it.

Positions need no conversion: both the editor and LSP use 0-based lines and UTF-16 columns.

## Features

| Server capability | Editor feature |
|---|---|
| `semanticTokensProvider` | Semantic tokens: full, delta and range requests, with `workspace/semanticTokens/refresh`. |
| `completionProvider` | The suggest widget, with the server's trigger characters, insert-and-replace edits, deprecated tags, commands and `completionItem/resolve`. Snippet items are expanded as snippets, and `insertTextMode: asIs` keeps their indentation. |
| `hoverProvider` | Hovers. Markdown and code blocks render as markdown; plain text is escaped. Plain-text completion and signature documentation and inlay hint tooltips stay `Documentation::PlainText`. |
| `definitionProvider` | Go to definition. Locations in other documents come back with their uri, as `OpenLocation`. |
| `referencesProvider`, `declarationProvider`, `typeDefinitionProvider`, `implementationProvider` | Go to and peek references, declarations, type definitions and implementations. |
| `documentHighlightProvider` | Occurrence highlights, with read and write kinds. |
| `signatureHelpProvider` | Parameter hints, with the server's trigger and retrigger characters. The request's context has the editor's `isRetrigger` (hints showing or pending) and `activeSignatureHelp` (with the overload the user chose). |
| `documentFormattingProvider`, `documentRangeFormattingProvider` | Format Document, Format Selection and format on paste, with the model's tab size and spaces setting. |
| `documentOnTypeFormattingProvider` | Format on type, with the server's trigger characters. |
| `codeActionProvider`, `executeCommandProvider` | Code actions and the lightbulb, with `codeAction/resolve` for edits when the server supports it. The request sends the server's diagnostics back as it published them, with numeric codes, related information and `data`. The server's commands are registered with `register_command` and run with `workspace/executeCommand`. |
| `selectionRangeProvider` | Expand and shrink selection. |
| `foldingRangeProvider` | Folding ranges, with their kinds. |
| `inlayHintProvider` | Inlay hints; `workspace/inlayHint/refresh` calls `refresh_inlay_hints`. |
| `renameProvider` | Rename (F2), with `prepareRename` when the server supports it. Edits to other documents need their models (`model_for_uri`). |
| `codeLensProvider` | Code lenses, with `codeLens/resolve` for lenses without a command when the server supports it; `workspace/codeLens/refresh` calls `refresh_code_lenses`. |
| `documentLinkProvider` | Links (Ctrl/Cmd+click), with the server's tooltips and `documentLink/resolve` for links without a target when the server supports it. |
| `colorProvider` | Color decorators and the color picker, with the server's colors and presentations. Like any color provider, it replaces the editor's own hex and `rgb()` detection for the languages it's registered for. |
| `linkedEditingRangeProvider` | Linked editing (`linkedEditing: true`), such as renaming an HTML tag pair, with the server's word pattern. |
| `diagnosticProvider` | Pull diagnostics, as `vscode-languageclient` does them: `textDocument/diagnostic` for each attached document when it's attached and 200ms after its last edit, with the server's `identifier` and the last report's `previousResultId`. A new pull cancels the pending one. An `unchanged` report keeps the markers, and related documents' reports go to their uris. With `interFileDependencies`, an edit pulls every attached document. With `workspaceDiagnostics`, `workspace/diagnostic` is sent at start and again 2 seconds after each response, for documents that aren't attached. `workspace/diagnostic/refresh` pulls everything again. The markers are the same as pushed diagnostics'. |
| `textDocument/publishDiagnostics` | Markers owned by `lsp:<name>`, shown as squiggles and in the overview ruler. Diagnostics for documents that aren't attached go to the marker service too (`set_markers_for_uri`): `model_markers` lists them, and a model opened later shows them at once. |

Providers are registered for `options.languages`. An empty list registers them for every language. Either way they only answer for attached models. When the client is dropped, shuts down or its server exits, the providers are unregistered.

The client also answers the requests a server sends:

- **`workspace/configuration`:** from `options.settings`, looked up by dotted section. `client.set_settings(settings)` replaces them while the server runs and sends `workspace/didChangeConfiguration` with `settings: null`, as `vscode-languageclient` does, so the server asks for them again.
- **`workspace/applyEdit`:** applied with `gpui_monaco::apply_workspace_edit`, the bulk edit service without an editor. Text edits go to any model `model_for_uri` finds, attached or not, each as one undo step. File operations go to the host's `set_workspace_file_edit_handler`, and fail without one. Edits whose change annotation needs confirmation go through the preview handler, if one is set. As in `vscode-languageclient`, an edit for a version of an attached document the server no longer has isn't applied. The response says whether the edit was applied; if it failed, the error is the `failureReason`.
- **`workspace/workspaceFolders`:** the root.
- **Progress and capability registration:** acknowledged.

It emits `LspClientEvent::ShowMessage` for messages meant for the user, and `LspClientEvent::Exited` when the server stops. A `window/showMessageRequest` is `LspClientEvent::ShowMessageRequest`, with the server's `actions` (extra properties included) and a `respond` handle: call `respond.respond(Some(action))` with the action the user picked, or `respond(None)` if they dismissed the prompt. The server waits for the answer; if every clone of `respond` is dropped unanswered, it gets `null`.

## Client APIs without editor UI

Some server features have no editor UI in gpui-monaco, as in Monaco. `LspClient` has typed methods for them, so a host can build its own UI. Their results use the editor's types: positions are `PointUtf16`, kinds are `SymbolKind` and uris are normalized like `TextModel::resource()`. Each one returns nothing when the server doesn't support the feature or has stopped.

- **Workspace symbols (`workspaceSymbolProvider`):** `workspace_symbols(query, cx)` sends `workspace/symbol` and returns `WorkspaceSymbol`s, from either flat `SymbolInformation` or `WorkspaceSymbol` results. A symbol whose `range` is `None` only names its document; `resolve_workspace_symbol(&symbol, cx)` sends `workspaceSymbol/resolve` to fill it in, as VS Code does before opening a symbol picked from "Go to Symbol in Workspace".
- **Call hierarchy (`callHierarchyProvider`):** `prepare_call_hierarchy(&model, position, cx)` sends `textDocument/prepareCallHierarchy` for an attached model, after syncing its edits, and returns `CallHierarchyItem`s. `incoming_calls(&item, cx)` and `outgoing_calls(&item, cx)` send the server's item back as it sent it and return `IncomingCall`s (the caller and where it makes the calls) and `OutgoingCall`s (the callee and where the item calls it, in the item's own document).
- **Type hierarchy (`typeHierarchyProvider`):** `prepare_type_hierarchy(&model, position, cx)` sends `textDocument/prepareTypeHierarchy` and returns `TypeHierarchyItem`s; `supertypes(&item, cx)` and `subtypes(&item, cx)` send the server's item back as it sent it. `lsp-types` has no `typeHierarchyProvider`, so `supports_type_hierarchy()` reads it from `raw_capabilities()`, the capabilities as the server sent them.
- **Inline values (`inlineValueProvider`):** for hosts that embed a debugger. `inline_values(&model, range, InlineValueContext { frame_id, stopped_location }, cx)` sends `textDocument/inlineValue` for the rows in `range` while execution is paused, and returns `InlineValue`s: text to show, a variable to look up or an expression to evaluate, each with its range. Showing them is up to the host, for example as injected text after the line, as VS Code's debugger does. `workspace/inlineValue/refresh` emits `LspClientEvent::InlineValuesRefresh`, so the host asks again.

## Other transports and requests

- **Other transports:** `LspClient::from_transport(reader, writer, options, cx)` connects over any async byte streams, such as a socket or an in-process server.
- **Other requests:** `client.connection()` sends requests the bridge doesn't cover, for example `connection.request::<lsp_types::request::Formatting>(params)`.
- **Cancellation:** dropping a request's future sends `$/cancelRequest`.
