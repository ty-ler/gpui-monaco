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

- **`attach(model)`:** sends `textDocument/didOpen`, then sends each edit as an incremental `didChange`, or the full text if the server asks for that. The model needs a uri (`TextModel::with_uri`) and is opened with its language id.
- **Before each request:** the bridge sends any edits the server hasn't seen yet, so a completion right after typing sees the new text.
- **Closing:** `detach(model)`, or releasing the model, sends `didClose`. Detaching also clears the server's markers.
- **`did_save(model)`:** sends `didSave` if the server wants it.

Positions need no conversion: both the editor and LSP use 0-based lines and UTF-16 columns.

## Features

| Server capability | Editor feature |
|---|---|
| `completionProvider` | The suggest widget, with the server's trigger characters. Snippet text is inserted as plain text until snippets land (#55). |
| `hoverProvider` | Hovers. Markdown and code blocks render as markdown; plain text is escaped. |
| `definitionProvider` | Go to definition. Locations in other documents come back with their uri, as `OpenLocation`. |
| `referencesProvider`, `declarationProvider`, `typeDefinitionProvider`, `implementationProvider` | Go to and peek references, declarations, type definitions and implementations. |
| `documentHighlightProvider` | Occurrence highlights, with read and write kinds. |
| `signatureHelpProvider` | Parameter hints, with the server's trigger and retrigger characters. |
| `documentFormattingProvider`, `documentRangeFormattingProvider` | Format Document, Format Selection and format on paste, with the model's tab size and spaces setting. |
| `documentOnTypeFormattingProvider` | Format on type, with the server's trigger characters. |
| `codeActionProvider`, `executeCommandProvider` | Code actions and the lightbulb, with `codeAction/resolve` for edits when the server supports it. The server's commands are registered with `register_command` and run with `workspace/executeCommand`. |
| `selectionRangeProvider` | Expand and shrink selection. |
| `foldingRangeProvider` | Folding ranges, with their kinds. |
| `inlayHintProvider` | Inlay hints; `workspace/inlayHint/refresh` calls `refresh_inlay_hints`. |
| `renameProvider` | Rename (F2), with `prepareRename` when the server supports it. Edits to other documents need their models (`model_for_uri`). |
| `textDocument/publishDiagnostics` | Markers owned by `lsp:<name>`, shown as squiggles and in the overview ruler. |

Providers are registered for `options.languages`. An empty list registers them for every language. Either way they only answer for attached models. When the client is dropped, shuts down or its server exits, the providers are unregistered.

The client also answers the requests a server sends:

- **`workspace/configuration`:** from `options.settings`, looked up by dotted section.
- **`workspace/applyEdit`:** applied to attached models.
- **`workspace/workspaceFolders`:** the root.
- **Progress and capability registration:** acknowledged.

It emits `LspClientEvent::ShowMessage` for messages meant for the user, and `LspClientEvent::Exited` when the server stops.

## Other transports and requests

- **Other transports:** `LspClient::from_transport(reader, writer, options, cx)` connects over any async byte streams, such as a socket or an in-process server.
- **Other requests:** `client.connection()` sends requests the bridge doesn't cover, for example `connection.request::<lsp_types::request::Formatting>(params)`.
- **Cancellation:** dropping a request's future sends `$/cancelRequest`.
