# Language servers (LSP)

The `gpui-monaco-lsp` crate connects [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) servers to the editor, the way `monaco-languageclient` does for Monaco. An `LspClient` starts a server and then does three things:

- It registers [providers](providers.md) for the features the server supports: completion, hover, go to definition, document highlights and signature help.
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
- **`did_save(model)`:** sends `didSave` if the server wants it.

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
| `textDocument/publishDiagnostics` | Markers owned by `lsp:<name>`, shown as squiggles and in the overview ruler. Diagnostics for documents that aren't attached go to the marker service too (`set_markers_for_uri`): `model_markers` lists them, and a model opened later shows them at once. |

Providers are registered for `options.languages`. An empty list registers them for every language. Either way they only answer for attached models. When the client is dropped, shuts down or its server exits, the providers are unregistered.

The client also answers the requests a server sends:

- **`workspace/configuration`:** from `options.settings`, looked up by dotted section.
- **`workspace/applyEdit`:** applied with `gpui_monaco::apply_workspace_edit`, the bulk edit service without an editor. Text edits go to any model `model_for_uri` finds, attached or not, each as one undo step. File operations go to the host's `set_workspace_file_edit_handler`, and fail without one. Edits whose change annotation needs confirmation go through the preview handler, if one is set. As in `vscode-languageclient`, an edit for a version of an attached document the server no longer has isn't applied. The response says whether the edit was applied; if it failed, the error is the `failureReason`.
- **`workspace/workspaceFolders`:** the root.
- **Progress and capability registration:** acknowledged.

It emits `LspClientEvent::ShowMessage` for messages meant for the user, and `LspClientEvent::Exited` when the server stops.

## Other transports and requests

- **Other transports:** `LspClient::from_transport(reader, writer, options, cx)` connects over any async byte streams, such as a socket or an in-process server.
- **Other requests:** `client.connection()` sends requests the bridge doesn't cover, for example `connection.request::<lsp_types::request::Formatting>(params)`.
- **Cancellation:** dropping a request's future sends `$/cancelRequest`.
