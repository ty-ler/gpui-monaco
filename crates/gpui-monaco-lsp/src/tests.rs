//! The bridge against an in-process fake server.

mod pull_diagnostics;

use std::sync::{Arc, Mutex};

use futures_lite::io::{AsyncWriteExt, BufReader};
use gpui::{AppContext as _, Entity, Task, TestAppContext};
use gpui_monaco::TextModel;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use serde_json::{Value, json};

use crate::features::LspFeatures;
use crate::rpc::read_message;
use crate::{LspClient, LspClientEvent, LspClientOptions};

mod call_hierarchy;
mod type_hierarchy;
mod workspace_symbols;

/// Answers a request, or `None` to never answer it.
type Handler = Box<dyn Fn(&str, &Value) -> Option<Value> + Send>;

struct Server {
    /// Every message the client sent, in order.
    log: Arc<Mutex<Vec<Value>>>,
    /// Messages to send the client.
    push: async_channel::Sender<Value>,
    _task: Task<()>,
}

impl Server {
    fn messages(&self, method: &str) -> Vec<Value> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m["method"] == method)
            .cloned()
            .collect()
    }

    fn methods(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| m["method"].as_str().map(str::to_string))
            .collect()
    }

    /// The client's response to the server's request `id`.
    fn response(&self, id: u32) -> Value {
        self.responses()
            .into_iter()
            .find(|m| m["id"] == id)
            .unwrap_or(Value::Null)
    }

    /// Sends the client a notification.
    fn notify(&self, method: &str, params: Value) {
        self.push
            .try_send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .unwrap();
    }

    /// Sends the client a request.
    fn request(&self, id: u32, method: &str, params: Value) {
        self.push
            .try_send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .unwrap();
    }

    fn responses(&self) -> Vec<Value> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.get("method").is_none())
            .cloned()
            .collect()
    }
}

fn capabilities() -> Value {
    json!({
        "textDocumentSync": 2,
        "completionProvider": { "triggerCharacters": ["."] },
        "hoverProvider": true,
        "definitionProvider": true,
        "documentHighlightProvider": true,
        "signatureHelpProvider": { "triggerCharacters": ["("], "retriggerCharacters": [","] },
        "documentFormattingProvider": true,
        "referencesProvider": true,
        "renameProvider": { "prepareProvider": true },
        "inlayHintProvider": true,
        "foldingRangeProvider": true,
        "selectionRangeProvider": true,
        "codeActionProvider": { "codeActionKinds": ["quickfix", "refactor"], "resolveProvider": true },
        "executeCommandProvider": { "commands": ["fake.doIt"] },
        "declarationProvider": true,
        "typeDefinitionProvider": true,
        "implementationProvider": true,
        "documentRangeFormattingProvider": true,
        "documentOnTypeFormattingProvider": { "firstTriggerCharacter": "}", "moreTriggerCharacter": [";"] },
        "codeLensProvider": { "resolveProvider": true },
        "documentLinkProvider": { "resolveProvider": true },
        "colorProvider": true,
        "linkedEditingRangeProvider": true,
        "workspaceSymbolProvider": { "resolveProvider": true },
        "callHierarchyProvider": true,
        "typeHierarchyProvider": true,
    })
}

async fn start(
    cx: &mut TestAppContext,
    handler: impl Fn(&str, &Value) -> Option<Value> + Send + 'static,
) -> (Entity<LspClient>, Server) {
    start_with(cx, capabilities(), handler).await
}

/// [`start`] with other server capabilities.
async fn start_with(
    cx: &mut TestAppContext,
    capabilities: Value,
    handler: impl Fn(&str, &Value) -> Option<Value> + Send + 'static,
) -> (Entity<LspClient>, Server) {
    let (client_reader, mut server_writer) = piper::pipe(1 << 20);
    let (server_reader, client_writer) = piper::pipe(1 << 20);
    let log = Arc::new(Mutex::new(Vec::new()));
    let (push, pushed) = async_channel::unbounded::<Value>();
    let handler: Handler = Box::new(move |method, params| match method {
        "initialize" => Some(json!({
            "capabilities": capabilities,
            "serverInfo": { "name": "fake" },
        })),
        "shutdown" => Some(Value::Null),
        _ => handler(method, params),
    });
    let task_log = log.clone();
    let task = cx.background_executor.spawn(async move {
        let mut reader = BufReader::new(server_reader);
        enum Next {
            FromClient(Option<Value>),
            ToClient(Value),
        }
        loop {
            let next = futures_lite::future::or(
                async { Next::FromClient(read_message(&mut reader).await.ok().flatten()) },
                async {
                    match pushed.recv().await {
                        Ok(message) => Next::ToClient(message),
                        Err(_) => std::future::pending().await,
                    }
                },
            )
            .await;
            let reply = match next {
                Next::FromClient(None) => return,
                Next::FromClient(Some(message)) => {
                    task_log.lock().unwrap().push(message.clone());
                    match (&message["id"], message["method"].as_str()) {
                        (Value::Number(_), Some(method)) => handler(method, &message["params"])
                            .map(|result| json!({ "jsonrpc": "2.0", "id": message["id"], "result": result })),
                        _ => None,
                    }
                }
                Next::ToClient(message) => Some(message),
            };
            if let Some(reply) = reply {
                let body = serde_json::to_vec(&reply).unwrap();
                let header = format!("Content-Length: {}\r\n\r\n", body.len());
                server_writer.write_all(header.as_bytes()).await.unwrap();
                server_writer.write_all(&body).await.unwrap();
            }
        }
    });
    let options = LspClientOptions {
        name: "fake".into(),
        languages: vec!["rust".into()],
        root_uri: Some("file:///project".into()),
        settings: json!({ "demo": { "option": 42 } }),
        ..Default::default()
    };
    let client = cx
        .update(|cx| LspClient::from_transport(client_reader, client_writer, options, cx))
        .await
        .unwrap();
    cx.run_until_parked();
    (
        client,
        Server {
            log,
            push,
            _task: task,
        },
    )
}

fn model(cx: &mut TestAppContext, text: &str) -> Entity<TextModel> {
    let text = text.to_string();
    cx.new(|_| {
        TextModel::new(&text)
            .with_uri("file:///project/main.rs")
            .with_language("rust")
    })
}

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

#[gpui::test]
async fn initializes_and_syncs_documents(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    assert_eq!(server.methods()[..2], ["initialize", "initialized"]);
    let init = &server.messages("initialize")[0]["params"];
    assert_eq!(init["rootUri"], "file:///project");
    assert_eq!(
        init["capabilities"]["general"]["positionEncodings"],
        json!(["utf-16"])
    );
    client.read_with(cx, |client, _| {
        assert_eq!(client.server_info().unwrap().name, "fake");
        assert!(client.is_running());
    });

    let model = model(cx, "fn main() {}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    cx.run_until_parked();
    let open = &server.messages("textDocument/didOpen")[0]["params"]["textDocument"];
    assert_eq!(
        (
            open["languageId"].as_str(),
            open["version"].as_i64(),
            open["text"].as_str()
        ),
        (Some("rust"), Some(0), Some("fn main() {}\n"))
    );

    // Incremental changes, in the order the server should apply them.
    model.update(cx, |m, cx| {
        m.edit([(3..7, "start"), (11..11, " x ")], cx).unwrap()
    });
    cx.run_until_parked();
    let change = &server.messages("textDocument/didChange")[0]["params"];
    assert_eq!(change["textDocument"]["version"], 1);
    assert_eq!(
        change["contentChanges"],
        json!([
            { "range": { "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 11 } }, "text": " x " },
            { "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 7 } }, "text": "start" },
        ])
    );

    // Detaching closes it; releasing an attached model does too.
    client.update(cx, |client, cx| client.detach(&model, cx));
    cx.run_until_parked();
    assert_eq!(server.messages("textDocument/didClose").len(), 1);
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    drop(model);
    // Released entities are dropped when effects flush.
    cx.update(|_| {});
    cx.run_until_parked();
    assert_eq!(server.messages("textDocument/didClose").len(), 2);
}

#[gpui::test]
async fn features_forward_requests(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, params| {
        let other = "file:///project/lib.rs";
        Some(match method {
            "textDocument/hover" => json!({
                "contents": { "kind": "markdown", "value": "**main**" },
                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 7 } },
            }),
            "textDocument/completion" => {
                assert_eq!(params["context"]["triggerKind"], 2);
                assert_eq!(params["context"]["triggerCharacter"], ".");
                json!({ "isIncomplete": true, "items": [{ "label": "len", "kind": 2 }] })
            }
            "textDocument/selectionRange" => json!([{
                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 7 } },
                "parent": { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 0 } } },
            }]),
            "textDocument/foldingRange" => json!([
                { "startLine": 0, "endLine": 2, "kind": "imports" },
            ]),
            "textDocument/inlayHint" => json!([
                {
                    "position": { "line": 0, "character": 7 },
                    "label": [
                        { "value": ": " },
                        { "value": "Foo", "tooltip": "a type", "location": { "uri": "file:///project/lib.rs", "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 3 } } } },
                    ],
                    "kind": 1,
                    "paddingLeft": true,
                    "textEdits": [{ "range": { "start": { "line": 0, "character": 7 }, "end": { "line": 0, "character": 7 } }, "newText": ": Foo" }],
                },
            ]),
            "textDocument/codeAction" => {
                assert_eq!(params["context"]["only"], json!(["quickfix"]));
                assert_eq!(params["context"]["triggerKind"], 1);
                assert_eq!(params["context"]["diagnostics"][0]["message"], "unused");
                json!([
                    { "title": "Run it", "command": "fake.doIt", "arguments": [1] },
                    {
                        "title": "Remove it",
                        "kind": "quickfix",
                        "isPreferred": true,
                        "data": { "id": 7 },
                    },
                ])
            }
            "codeAction/resolve" => {
                assert_eq!(params["data"]["id"], 7);
                json!({
                    "title": "Remove it",
                    "kind": "quickfix",
                    "edit": { "changes": { "file:///project/main.rs": [
                        { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "newText": "" },
                    ] } },
                })
            }
            "workspace/executeCommand" => {
                assert_eq!(params["command"], "fake.doIt");
                assert_eq!(params["arguments"], json!([1]));
                Value::Null
            }
            "textDocument/prepareRename" => json!({
                "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 7 } },
                "placeholder": "main",
            }),
            "textDocument/rename" => {
                assert_eq!(params["newName"], "start");
                json!({ "documentChanges": [{
                    "textDocument": { "uri": other, "version": 1 },
                    "edits": [{ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 4 } }, "newText": "start" }],
                }] })
            }
            "textDocument/references" => {
                assert_eq!(params["context"]["includeDeclaration"], true);
                json!([{ "uri": other, "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 1 } } }])
            }
            "textDocument/declaration" | "textDocument/typeDefinition" | "textDocument/implementation" => json!({
                "uri": "file:///project/main.rs",
                "range": { "start": { "line": 0, "character": 1 }, "end": { "line": 0, "character": 2 } },
            }),
            "textDocument/definition" => json!([
                { "uri": "file:///project/main.rs", "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 2 } } },
                { "uri": other, "range": { "start": { "line": 4, "character": 1 }, "end": { "line": 4, "character": 5 } } },
            ]),
            "textDocument/documentHighlight" => json!([
                { "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 7 } }, "kind": 3 },
            ]),
            "textDocument/formatting" | "textDocument/rangeFormatting" | "textDocument/onTypeFormatting" => {
                assert_eq!(params["options"]["tabSize"], 4);
                assert_eq!(params["options"]["insertSpaces"], true);
                let column = match method {
                    "textDocument/formatting" => 0,
                    "textDocument/rangeFormatting" => params["range"]["start"]["character"].as_u64().unwrap(),
                    _ => {
                        assert_eq!(params["ch"], ";");
                        params["position"]["character"].as_u64().unwrap()
                    }
                };
                json!([{ "range": { "start": { "line": 0, "character": column }, "end": { "line": 0, "character": column } }, "newText": method }])
            }
            "textDocument/signatureHelp" => {
                assert_eq!(params["context"]["isRetrigger"], true);
                json!({
                    "signatures": [{ "label": "f(a, b)", "parameters": [{ "label": [2, 3] }, { "label": "b" }] }],
                    "activeSignature": 0,
                    "activeParameter": 1,
                })
            }
            _ => return None,
        })
    })
    .await;
    let model = model(cx, "fn main() {}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let features =
        cx.update(|cx| LspFeatures::new(client.downgrade(), client.read(cx).capabilities()));
    assert_eq!(CompletionProvider::trigger_characters(&features), vec!['.']);
    assert_eq!(
        SignatureHelpProvider::retrigger_characters(&features),
        vec![',']
    );

    // An edit right before a request is synced first.
    model.update(cx, |m, cx| m.edit([(0..0, "// \n")], cx).unwrap());
    let context = model.read_with(cx, |m, _| m.provider_context());
    let hover = cx.update(|cx| features.provide_hover(&context, p(1, 4), cx));
    let hover = hover.await.unwrap().unwrap();
    assert_eq!(hover.contents, vec!["**main**".into()]);
    assert_eq!(hover.range, Some(p(0, 3)..p(0, 7)));
    let methods = server.methods();
    let change = methods
        .iter()
        .position(|m| m == "textDocument/didChange")
        .unwrap();
    let request = methods
        .iter()
        .position(|m| m == "textDocument/hover")
        .unwrap();
    assert!(change < request, "{methods:?}");

    let list = cx
        .update(|cx| {
            features.provide_completions(
                &context,
                p(1, 4),
                CompletionTrigger::TriggerCharacter('.'),
                cx,
            )
        })
        .await
        .unwrap();
    assert!(list.incomplete);
    assert_eq!(list.items[0].label, "len");
    assert_eq!(list.items[0].kind, CompletionItemKind::Method);

    let locations = cx
        .update(|cx| features.provide_definition(&context, p(1, 4), cx))
        .await
        .unwrap();
    assert_eq!(locations[0].uri, None);
    assert_eq!(locations[1].uri.as_deref(), Some("file:///project/lib.rs"));
    assert_eq!(locations[1].range, p(4, 1)..p(4, 5));

    let highlights = cx
        .update(|cx| features.provide_document_highlights(&context, p(1, 4), cx))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(highlights[0].kind, DocumentHighlightKind::Write);

    let help = cx
        .update(|cx| {
            features.provide_signature_help(
                &context,
                p(1, 4),
                SignatureHelpTrigger::ContentChange,
                cx,
            )
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(help.active_parameter, 1);
    assert_eq!(
        help.signatures[0].parameters[0].label,
        ParameterLabel::Range(2..3)
    );

    // Code actions: bare commands and actions, resolved for their edit.
    assert_eq!(
        features.provided_kinds(),
        Some(vec!["quickfix".into(), "refactor".into()])
    );
    let actions = cx
        .update(|cx| {
            features.provide_code_actions(
                &context,
                p(0, 0)..p(0, 3),
                &CodeActionContext {
                    only: Some("quickfix".into()),
                    trigger: CodeActionTrigger::Invoke,
                    markers: vec![gpui_monaco::Marker {
                        severity: gpui_monaco::MarkerSeverity::Warning,
                        message: "unused".into(),
                        range: p(0, 0)..p(0, 3),
                        source: None,
                        code: None,
                        ..Default::default()
                    }],
                },
                cx,
            )
        })
        .await
        .unwrap();
    assert_eq!(actions[0].command.as_ref().unwrap().id, "fake.doIt");
    assert!(actions[1].is_preferred && actions[1].edit.is_none());
    let resolved = cx
        .update(|cx| features.resolve_code_action(&actions[1], cx))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(resolved.edit.unwrap().edits[0].1[0].0, p(0, 0)..p(0, 3));
    // Server commands run on the server.
    cx.update(|cx| gpui_monaco::execute_command(cx, actions[0].command.as_ref().unwrap()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(server.messages("workspace/executeCommand").len(), 1);

    // Selection ranges, with parents flattened.
    let selection = cx
        .update(|cx| features.provide_selection_ranges(&context, &[p(0, 4)], cx))
        .await
        .unwrap();
    assert_eq!(selection, [vec![p(0, 3)..p(0, 7), p(0, 0)..p(1, 0)]]);

    // Folding ranges.
    let folds = cx
        .update(|cx| features.provide_folding_ranges(&context, &Default::default(), cx))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        folds,
        [FoldingRange {
            start_row: 0,
            end_row: 2,
            kind: Some(FoldingRangeKind::Imports),
        }]
    );

    // Inlay hints, with their label parts.
    let hints = cx
        .update(|cx| features.provide_inlay_hints(&context, p(0, 0)..p(1, 0), cx))
        .await
        .unwrap();
    assert_eq!(
        (
            hints[0].position,
            hints[0].label.text(),
            hints[0].label.parts().len(),
            hints[0].kind,
            hints[0].padding_left
        ),
        (p(0, 7), ": Foo".into(), 2, Some(InlayHintKind::Type), true)
    );
    let foo = &hints[0].label.parts()[1];
    assert_eq!(foo.tooltip.as_ref().map(|t| t.as_str()), Some("a type"));
    assert_eq!(
        foo.location
            .as_ref()
            .map(|l| (l.uri.as_deref(), l.range.clone())),
        Some((Some("file:///project/lib.rs"), p(2, 0)..p(2, 3)))
    );
    assert_eq!(
        hints[0].text_edits,
        vec![(p(0, 7)..p(0, 7), ": Foo".to_string())]
    );

    // Rename.
    let location = cx
        .update(|cx| features.resolve_rename_location(&context, p(1, 4), cx))
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (location.range, location.text.as_ref()),
        (p(0, 3)..p(0, 7), "main")
    );
    let edit = cx
        .update(|cx| features.provide_rename_edits(&context, p(1, 4), "start", cx))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        edit.edits,
        vec![(
            Some("file:///project/lib.rs".into()),
            vec![(p(0, 0)..p(0, 4), "start".to_string())]
        )]
    );

    // The other location requests.
    let references = cx
        .update(|cx| {
            features.provide_references(
                &context,
                p(1, 4),
                ReferenceContext {
                    include_declaration: true,
                },
                cx,
            )
        })
        .await
        .unwrap();
    assert_eq!(references[0].uri.as_deref(), Some("file:///project/lib.rs"));
    for kind in ["declaration", "type definition", "implementation"] {
        let task = cx.update(|cx| match kind {
            "declaration" => features.provide_declaration(&context, p(1, 4), cx),
            "type definition" => features.provide_type_definition(&context, p(1, 4), cx),
            _ => features.provide_implementation(&context, p(1, 4), cx),
        });
        let locations = task.await.unwrap();
        assert_eq!(locations[0].uri, None, "{kind}");
        assert_eq!(locations[0].range, p(0, 1)..p(0, 2), "{kind}");
    }

    // Formatting, with the model's options.
    let options = FormattingOptions {
        tab_size: 4,
        insert_spaces: true,
    };
    assert_eq!(features.auto_format_trigger_characters(), vec!['}', ';']);
    let edits = cx
        .update(|cx| features.provide_document_formatting_edits(&context, options, cx))
        .await
        .unwrap();
    assert_eq!(
        edits.edits,
        vec![(p(0, 0)..p(0, 0), "textDocument/formatting".to_string())]
    );
    let edits = cx
        .update(|cx| {
            features.provide_document_range_formatting_edits(
                &context,
                p(0, 2)..p(1, 0),
                options,
                cx,
            )
        })
        .await
        .unwrap();
    assert_eq!(edits.edits[0].0, p(0, 2)..p(0, 2));
    let edits = cx
        .update(|cx| features.provide_on_type_formatting_edits(&context, p(1, 3), ';', options, cx))
        .await
        .unwrap();
    assert_eq!(edits.edits[0].1, "textDocument/onTypeFormatting");

    // Models that aren't attached get nothing, without asking the server.
    let other = cx.new(|_| TextModel::new("x").with_uri("file:///elsewhere.rs"));
    let context = other.read_with(cx, |m, _| m.provider_context());
    let before = server.methods().len();
    let hover = cx.update(|cx| features.provide_hover(&context, p(0, 0), cx));
    assert_eq!(hover.await.unwrap(), None);
    cx.run_until_parked();
    assert_eq!(server.methods().len(), before);
}

#[gpui::test]
async fn diagnostics_and_server_requests(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    let model = model(cx, "let x = 1;\nlet y = 2;\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let sink = messages.clone();
    cx.update(|cx| {
        cx.subscribe(&client, move |_, event: &LspClientEvent, _| {
            sink.lock().unwrap().push(format!("{event:?}"));
        })
        .detach();
    });

    // Diagnostics become markers owned by the server.
    server
        .push
        .try_send(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///project/main.rs",
                "diagnostics": [{
                    "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 5 } },
                    "severity": 2,
                    "source": "fake",
                    "message": "unused y",
                }],
            },
        }))
        .unwrap();
    cx.run_until_parked();
    let markers = model.read_with(cx, |m, _| m.markers());
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].message, "unused y");
    assert_eq!(markers[0].range, p(1, 4)..p(1, 5));

    // Settings by section, and an edit to an attached document.
    let request = |id: u32, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    server
        .push
        .try_send(request(
            1,
            "workspace/configuration",
            json!({ "items": [{ "section": "demo.option" }, { "section": "missing" }] }),
        ))
        .unwrap();
    server
        .push
        .try_send(request(
            2,
            "workspace/applyEdit",
            json!({ "edit": { "changes": { "file:///project/main.rs": [
                { "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 5 } }, "newText": "_y" },
                { "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "newText": "_x" },
            ] } } }),
        ))
        .unwrap();
    server
        .push
        .try_send(request(3, "custom/unknown", Value::Null))
        .unwrap();
    server
        .push
        .try_send(json!({
            "jsonrpc": "2.0",
            "method": "window/showMessage",
            "params": { "type": 1, "message": "boom" },
        }))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(server.response(1)["result"], json!([42, null]));
    assert_eq!(server.response(2)["result"]["applied"], true);
    assert_eq!(server.response(3)["error"]["code"], -32601);
    assert_eq!(
        model.read_with(cx, |m, _| m.text()),
        "let _x = 1;\nlet _y = 2;\n"
    );
    assert!(messages.lock().unwrap()[0].contains("boom"));

    // Detaching clears the server's markers.
    client.update(cx, |client, cx| client.detach(&model, cx));
    assert!(model.read_with(cx, |m, _| m.markers()).is_empty());
}

#[gpui::test]
async fn dropped_requests_are_cancelled(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    let model = model(cx, "x");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let features =
        cx.update(|cx| LspFeatures::new(client.downgrade(), client.read(cx).capabilities()));
    let context = model.read_with(cx, |m, _| m.provider_context());
    let hover = cx.update(|cx| features.provide_hover(&context, p(0, 0), cx));
    cx.run_until_parked();
    drop(hover);
    cx.run_until_parked();
    let hover_id = server.messages("textDocument/hover")[0]["id"].clone();
    let cancel = server.messages("$/cancelRequest");
    assert_eq!(cancel[0]["params"]["id"], hover_id);
}

#[gpui::test]
async fn signature_help_context_reaches_the_server(cx: &mut TestAppContext) {
    // Monaco `ParameterHintsModel.doTrigger`: `isRetrigger` is whether
    // hints show or are pending, and `activeSignatureHelp` carries the
    // overload the user chose.
    let (client, server) = start(cx, |method, _| {
        (method == "textDocument/signatureHelp").then(|| {
            json!({
                "signatures": [{ "label": "f(a)" }, { "label": "f(a, b)" }],
                "activeSignature": 0,
            })
        })
    })
    .await;
    let model = model(cx, "f(1, 2)\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let features =
        cx.update(|cx| LspFeatures::new(client.downgrade(), client.read(cx).capabilities()));
    let context = model.read_with(cx, |m, _| m.provider_context());
    let showing = SignatureHelp {
        signatures: vec![
            SignatureInformation {
                label: "f(a)".into(),
                documentation: Some(Documentation::PlainText("one".into())),
                parameters: vec![ParameterInformation {
                    label: ParameterLabel::Range(2..3),
                    documentation: None,
                }],
                active_parameter: None,
            },
            SignatureInformation {
                label: "f(a, b)".into(),
                documentation: Some(Documentation::Markdown("**two**".into())),
                parameters: vec![ParameterInformation {
                    label: ParameterLabel::Text("b".into()),
                    documentation: None,
                }],
                active_parameter: Some(1),
            },
        ],
        active_signature: 1,
        active_parameter: 0,
    };
    // `(` isn't a retrigger character, but hints are showing.
    let help_context = SignatureHelpContext {
        trigger: SignatureHelpTrigger::TriggerCharacter('('),
        is_retrigger: true,
        active_signature_help: Some(showing),
    };
    let help = cx
        .update(|cx| {
            features.provide_signature_help_with_context(&context, p(0, 5), &help_context, cx)
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(help.signatures.len(), 2);
    let sent = &server.messages("textDocument/signatureHelp")[0]["params"]["context"];
    assert_eq!(sent["triggerKind"], 2);
    assert_eq!(sent["triggerCharacter"], "(");
    assert_eq!(sent["isRetrigger"], true);
    assert_eq!(
        sent["activeSignatureHelp"],
        json!({
            "signatures": [
                { "label": "f(a)", "documentation": "one", "parameters": [{ "label": [2, 3] }] },
                {
                    "label": "f(a, b)",
                    "documentation": { "kind": "markdown", "value": "**two**" },
                    "parameters": [{ "label": "b" }],
                    "activeParameter": 1,
                },
            ],
            "activeSignature": 1,
            "activeParameter": 0,
        })
    );

    // An explicit request with nothing showing.
    let help_context = SignatureHelpContext {
        trigger: SignatureHelpTrigger::Invoke,
        is_retrigger: false,
        active_signature_help: None,
    };
    cx.update(|cx| {
        features.provide_signature_help_with_context(&context, p(0, 5), &help_context, cx)
    })
    .await
    .unwrap();
    let sent = &server.messages("textDocument/signatureHelp")[1]["params"]["context"];
    assert_eq!(sent["triggerKind"], 1);
    assert_eq!(sent["isRetrigger"], false);
    assert!(sent.get("activeSignatureHelp").is_none_or(Value::is_null));
}

#[gpui::test]
async fn shutdown_and_exit(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    let registered =
        |cx: &mut TestAppContext| cx.update(|cx| hover_providers(cx, Some("rust")).len());
    assert_eq!(registered(cx), 1);
    // Registered for the client's languages only.
    assert_eq!(cx.update(|cx| hover_providers(cx, Some("python")).len()), 0);
    client
        .update(cx, |client, cx| client.shutdown(cx))
        .await
        .unwrap();
    cx.run_until_parked();
    assert_eq!(server.methods()[2..], ["shutdown", "exit"]);
    assert_eq!(registered(cx), 0);

    // The server going away is reported.
    let exited = Arc::new(Mutex::new(false));
    let flag = exited.clone();
    cx.update(|cx| {
        cx.subscribe(&client, move |_, event: &LspClientEvent, _| {
            if matches!(event, LspClientEvent::Exited) {
                *flag.lock().unwrap() = true;
            }
        })
        .detach();
    });
    drop(server);
    cx.run_until_parked();
    assert!(*exited.lock().unwrap());
    assert!(!client.read_with(cx, |client, _| client.is_running()));
}

fn range(start: (u32, u32), end: (u32, u32)) -> Value {
    json!({
        "start": { "line": start.0, "character": start.1 },
        "end": { "line": end.0, "character": end.1 },
    })
}

/// Monaco-languageclient identifies documents by `model.uri.toString()`:
/// a path with a space or a drive letter is sent normalized, and the
/// server's uris are compared normalized (`vscode-uri` writes `C:` as
/// `c%3A`).
#[gpui::test]
async fn documents_are_identified_by_normalized_uris(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, params| {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        match method {
            "textDocument/hover" => Some(json!({ "contents": format!("hover {uri}") })),
            // The server writes the drive letter its own way.
            "textDocument/definition" => Some(json!([
                { "uri": "file:///C:/proj/a.rs", "range": range((0, 0), (0, 1)) },
                { "uri": "file:///c%3A/proj/b.rs", "range": range((0, 0), (0, 1)) },
            ])),
            _ => None,
        }
    })
    .await;
    let spaced = cx.new(|_| {
        TextModel::new("fn main() {}\n")
            .with_uri("file:///project/my project/main.rs")
            .with_language("rust")
    });
    let drive = cx.new(|_| {
        TextModel::new("let x = 1;\n")
            .with_uri("file:///C:/proj/a.rs")
            .with_language("rust")
    });
    let unnamed = cx.new(|_| TextModel::new("x").with_language("rust"));
    for model in [&spaced, &drive, &unnamed] {
        client
            .update(cx, |client, cx| client.attach(model, cx))
            .unwrap();
        assert!(client.read_with(cx, |client, cx| client.is_attached(model, cx)));
    }
    cx.run_until_parked();
    let unnamed_uri = unnamed.read_with(cx, |m, _| m.resource().to_string());
    assert!(unnamed_uri.starts_with("inmemory://model/"));
    let opened: Vec<_> = server
        .messages("textDocument/didOpen")
        .iter()
        .map(|m| m["params"]["textDocument"]["uri"].clone())
        .collect();
    assert_eq!(
        opened,
        vec![
            json!("file:///project/my%20project/main.rs"),
            json!("file:///c%3A/proj/a.rs"),
            json!(unnamed_uri),
        ]
    );

    // Requests name the documents the same way, a model without a uri too.
    let features =
        cx.update(|cx| LspFeatures::new(client.downgrade(), client.read(cx).capabilities()));
    for (model, uri) in [
        (&spaced, "file:///project/my%20project/main.rs"),
        (&unnamed, unnamed_uri.as_str()),
    ] {
        let context = model.read_with(cx, |m, _| m.provider_context());
        let hover = cx.update(|cx| features.provide_hover(&context, p(0, 0), cx));
        cx.run_until_parked();
        let hover = hover.await.unwrap().unwrap();
        assert_eq!(hover.contents[0].value.as_ref(), format!("hover {uri}"));
    }

    // A location in the queried document has no uri, however the server
    // writes it; others are normalized.
    let context = drive.read_with(cx, |m, _| m.provider_context());
    let definition = cx.update(|cx| features.provide_definition(&context, p(0, 4), cx));
    cx.run_until_parked();
    let definition = definition.await.unwrap();
    assert_eq!(definition[0].uri, None);
    assert_eq!(definition[1].uri.as_deref(), Some("file:///c%3A/proj/b.rs"));

    // Diagnostics published for the server's form reach the model.
    server.notify(
        "textDocument/publishDiagnostics",
        json!({
            "uri": "file:///c%3A/proj/a.rs",
            "diagnostics": [{ "range": range((0, 4), (0, 5)), "message": "unused x" }],
        }),
    );
    server.notify(
        "textDocument/publishDiagnostics",
        json!({
            "uri": "file:///project/my%20project/main.rs",
            "diagnostics": [{ "range": range((0, 3), (0, 7)), "message": "main" }],
        }),
    );
    cx.run_until_parked();
    assert_eq!(
        drive.read_with(cx, |m, _| m.markers())[0].message,
        "unused x"
    );
    assert_eq!(spaced.read_with(cx, |m, _| m.markers())[0].message, "main");

    // Edits sync and close with the same uri.
    drive.update(cx, |m, cx| m.edit([(0..0, "// ")], cx).unwrap());
    client.update(cx, |client, cx| client.detach(&drive, cx));
    cx.run_until_parked();
    assert_eq!(
        server.messages("textDocument/didChange")[0]["params"]["textDocument"]["uri"],
        "file:///c%3A/proj/a.rs"
    );
    assert_eq!(
        server.messages("textDocument/didClose")[0]["params"]["textDocument"]["uri"],
        "file:///c%3A/proj/a.rs"
    );
}

/// `vscode-languageclient` writes diagnostics into a collection keyed by
/// uri, which reaches the marker service whether or not the document is
/// open.
#[gpui::test]
async fn diagnostics_reach_documents_that_are_not_attached(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    cx.update(gpui_monaco::init);
    let owner = client.read_with(cx, |client, _| client.markers_owner());
    server.notify(
        "textDocument/publishDiagnostics",
        json!({
            "uri": "file:///project/other%20file.rs",
            "diagnostics": [{ "range": range((1, 0), (1, 3)), "message": "elsewhere" }],
        }),
    );
    cx.run_until_parked();
    let listed = cx.update(|cx| {
        gpui_monaco::model_markers(
            cx,
            &gpui_monaco::MarkerFilter {
                owner: Some(owner.clone()),
                ..Default::default()
            },
        )
    });
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].resource.as_ref(),
        "file:///project/other%20file.rs"
    );

    // A model opened later is decorated at once.
    let model = cx
        .update(|cx| {
            gpui_monaco::create_model(
                cx,
                "a\nfoo\n",
                Some("rust"),
                Some("file:///project/other file.rs"),
            )
        })
        .unwrap();
    let markers = model.read_with(cx, |m, _| m.markers());
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].range, p(1, 0)..p(1, 3));

    // An empty publish clears them.
    server.notify(
        "textDocument/publishDiagnostics",
        json!({ "uri": "file:///project/other%20file.rs", "diagnostics": [] }),
    );
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.markers()).is_empty());
}

/// `workspace/applyEdit` goes through the bulk edit service: file
/// operations go to the host, documents that aren't attached are edited,
/// failures say why, and stale versions aren't applied.
#[gpui::test]
async fn apply_edit_goes_through_the_bulk_edit_service(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    cx.update(gpui_monaco::init);
    let model = model(cx, "let x = 1;\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let other = cx
        .update(|cx| {
            gpui_monaco::create_model(cx, "b\n", Some("rust"), Some("file:///project/b c.rs"))
        })
        .unwrap();
    cx.run_until_parked();

    // A file operation without a handler fails, with the reason.
    let create = json!({ "edit": { "documentChanges": [
        { "kind": "create", "uri": "file:///project/new.rs" },
        { "textDocument": { "uri": "file:///project/b%20c.rs", "version": null },
          "edits": [{ "range": range((0, 0), (0, 0)), "newText": "// " }] },
    ] } });
    server.request(1, "workspace/applyEdit", create.clone());
    cx.run_until_parked();
    let response = server.response(1)["result"].clone();
    assert_eq!(response["applied"], false);
    assert!(
        response["failureReason"]
            .as_str()
            .unwrap()
            .contains("file edit handler")
    );
    assert_eq!(other.read_with(cx, |m, _| m.text()), "b\n");

    // With one, the file is created and the document that isn't attached
    // (and that the server names with `%20`) is edited.
    let created = Arc::new(Mutex::new(Vec::new()));
    let sink = created.clone();
    let _handler = cx.update(|cx| {
        gpui_monaco::set_workspace_file_edit_handler(cx, move |edits, _| {
            sink.lock().unwrap().extend(edits);
            Task::ready(Ok(()))
        })
    });
    server.request(2, "workspace/applyEdit", create);
    cx.run_until_parked();
    assert_eq!(server.response(2)["result"]["applied"], true);
    assert_eq!(
        created.lock().unwrap()[0].new_uri.as_deref(),
        Some("file:///project/new.rs")
    );
    assert_eq!(other.read_with(cx, |m, _| m.text()), "// b\n");

    // An edit that fails isn't reported as applied.
    server.request(
        3,
        "workspace/applyEdit",
        json!({ "edit": { "changes": { "file:///project/main.rs": [
            { "range": range((0, 0), (0, 5)), "newText": "a" },
            { "range": range((0, 2), (0, 7)), "newText": "b" },
        ] } } }),
    );
    // A document nobody has fails as Monaco's "model not found".
    server.request(
        4,
        "workspace/applyEdit",
        json!({ "edit": { "changes": { "file:///project/missing.rs": [
            { "range": range((0, 0), (0, 0)), "newText": "a" },
        ] } } }),
    );
    // An edit for a version of an attached document the server no longer
    // has isn't applied (`vscode-languageclient`).
    server.request(
        5,
        "workspace/applyEdit",
        json!({ "edit": { "documentChanges": [
            { "textDocument": { "uri": "file:///project/main.rs", "version": 7 },
              "edits": [{ "range": range((0, 0), (0, 0)), "newText": "a" }] },
        ] } }),
    );
    cx.run_until_parked();
    for id in [3, 4, 5] {
        assert_eq!(server.response(id)["result"]["applied"], false, "{id}");
    }
    assert!(
        server.response(4)["result"]["failureReason"]
            .as_str()
            .unwrap()
            .contains("model not found")
    );
    assert_eq!(model.read_with(cx, |m, _| m.text()), "let x = 1;\n");

    // The current version is applied.
    server.request(
        6,
        "workspace/applyEdit",
        json!({ "edit": { "documentChanges": [
            { "textDocument": { "uri": "file:///project/main.rs", "version": 0 },
              "edits": [{ "range": range((0, 4), (0, 5)), "newText": "y" }] },
        ] } }),
    );
    cx.run_until_parked();
    assert_eq!(server.response(6)["result"]["applied"], true);
    assert_eq!(model.read_with(cx, |m, _| m.text()), "let y = 1;\n");
}

/// Code action requests send the diagnostics as the server published
/// them (`vscode-languageclient` keeps them in its collection): numeric
/// codes stay numbers, and related information and data go along.
#[gpui::test]
async fn code_actions_send_the_published_diagnostics(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, _| {
        (method == "textDocument/codeAction").then(|| json!([]))
    })
    .await;
    let model = model(cx, "let x = 1;\nlet y = 2;\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let published = json!({
        "range": range((1, 4), (1, 5)),
        "severity": 1,
        "code": 2304,
        "source": "ts",
        "message": "Cannot find name 'y'.",
        "relatedInformation": [{
            "location": { "uri": "file:///project/lib.rs", "range": range((0, 0), (0, 1)) },
            "message": "declared here",
        }],
        "data": { "fix": 1 },
    });
    server.notify(
        "textDocument/publishDiagnostics",
        json!({ "uri": "file:///project/main.rs", "diagnostics": [published.clone()] }),
    );
    cx.run_until_parked();
    // The marker moves with an edit before the request.
    model.update(cx, |m, cx| m.edit([(0..0, "\n")], cx).unwrap());
    let markers = model.read_with(cx, |m, _| m.markers());
    assert_eq!(markers[0].range, p(2, 4)..p(2, 5));
    let features =
        cx.update(|cx| LspFeatures::new(client.downgrade(), client.read(cx).capabilities()));
    let context = model.read_with(cx, |m, _| m.provider_context());
    let actions = cx.update(|cx| {
        features.provide_code_actions(
            &context,
            p(2, 4)..p(2, 4),
            &CodeActionContext {
                only: None,
                trigger: CodeActionTrigger::Invoke,
                markers: markers.clone(),
            },
            cx,
        )
    });
    cx.run_until_parked();
    actions.await.unwrap();
    let request = &server.messages("textDocument/codeAction")[0]["params"];
    assert_eq!(request["context"]["diagnostics"], json!([published]));

    // A marker the server didn't publish keeps its related information.
    let marker = gpui_monaco::Marker {
        message: "mine".into(),
        range: p(0, 0)..p(0, 1),
        code: Some("7".into()),
        related_information: vec![gpui_monaco::RelatedInformation {
            uri: Some("file:///project/lib.rs".into()),
            range: p(1, 0)..p(1, 2),
            message: "there".into(),
        }],
        ..Default::default()
    };
    let diagnostics = client.read_with(cx, |client, _| {
        client.lsp_diagnostics("file:///project/main.rs", &[marker])
    });
    assert_eq!(
        serde_json::to_value(&diagnostics[0]).unwrap()["relatedInformation"],
        json!([{
            "location": { "uri": "file:///project/lib.rs", "range": range((1, 0), (1, 2)) },
            "message": "there",
        }])
    );
    assert_eq!(serde_json::to_value(&diagnostics[0]).unwrap()["code"], "7");
}

/// `monaco-languageclient` registers code lens, document link, color and
/// linked editing range providers (with `codeLens/resolve` and
/// `documentLink/resolve`), advertises them, and maps
/// `workspace/codeLens/refresh` to the code lens providers' change event.
#[gpui::test]
async fn code_lenses_links_colors_and_linked_editing(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, params| {
        Some(match method {
            "textDocument/codeLens" => json!([
                { "range": range((0, 3), (0, 7)), "data": { "lens": 1 } },
                { "range": range((1, 0), (1, 2)), "command": { "title": "Run", "command": "fake.doIt", "arguments": [2] } },
            ]),
            "codeLens/resolve" => {
                assert_eq!(params["data"], json!({ "lens": 1 }));
                assert_eq!(params["range"], range((0, 3), (0, 7)));
                json!({
                    "range": params["range"],
                    "command": { "title": "3 references", "command": "fake.doIt" },
                    "data": params["data"],
                })
            }
            "textDocument/documentLink" => json!([
                { "range": range((0, 0), (0, 2)), "target": "file:///C:/a%20b.rs", "tooltip": "Open a b" },
                { "range": range((1, 0), (1, 2)), "data": { "link": 2 } },
            ]),
            "documentLink/resolve" => {
                assert_eq!(params["data"], json!({ "link": 2 }));
                json!({ "range": params["range"], "target": "https://example.com/" })
            }
            "textDocument/documentColor" => json!([
                { "range": range((0, 3), (0, 7)), "color": { "red": 1.0, "green": 0.5, "blue": 0.0, "alpha": 1.0 } },
            ]),
            "textDocument/colorPresentation" => {
                assert_eq!(params["color"], json!({ "red": 1.0, "green": 0.5, "blue": 0.0, "alpha": 1.0 }));
                assert_eq!(params["range"], range((0, 3), (0, 7)));
                json!([
                    { "label": "#ff8000" },
                    {
                        "label": "rgb(255, 128, 0)",
                        "textEdit": { "range": range((0, 3), (0, 7)), "newText": "rgb(255, 128, 0)" },
                        "additionalTextEdits": [{ "range": range((0, 0), (0, 0)), "newText": "use rgb;\n" }],
                    },
                ])
            }
            "textDocument/linkedEditingRange" => {
                if params["position"]["line"] == 1 {
                    Value::Null
                } else {
                    json!({ "ranges": [range((0, 1), (0, 4)), range((0, 7), (0, 10))], "wordPattern": "[a-z]+" })
                }
            }
            _ => return None,
        })
    })
    .await;

    // The client says it supports them.
    let init = &server.messages("initialize")[0]["params"]["capabilities"];
    assert_eq!(init["textDocument"]["codeLens"], json!({}));
    assert_eq!(init["textDocument"]["documentLink"]["tooltipSupport"], true);
    assert_eq!(init["textDocument"]["colorProvider"], json!({}));
    assert_eq!(init["textDocument"]["linkedEditingRange"], json!({}));
    assert_eq!(init["workspace"]["codeLens"]["refreshSupport"], true);

    // The providers are registered for the client's languages.
    cx.update(|cx| {
        assert_eq!(code_lens_providers(cx, Some("rust")).len(), 1);
        assert_eq!(link_providers(cx, Some("rust")).len(), 1);
        assert_eq!(color_providers(cx, Some("rust")).len(), 1);
        assert_eq!(linked_editing_range_providers(cx, Some("rust")).len(), 1);
    });

    let model = model(cx, "<div></div>\nab\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let features =
        cx.update(|cx| LspFeatures::new(client.downgrade(), client.read(cx).capabilities()));
    let context = model.read_with(cx, |m, _| m.provider_context());

    // Code lenses, a lens without a command resolved with its data.
    let lenses = cx.update(|cx| features.provide_code_lenses(&context, cx));
    let lenses = lenses.await.unwrap();
    assert_eq!(lenses.len(), 2);
    assert_eq!(lenses[0].range, p(0, 3)..p(0, 7));
    assert_eq!(lenses[0].command, None);
    let run = lenses[1].command.as_ref().unwrap();
    assert_eq!((run.id.as_ref(), run.title.as_ref()), ("fake.doIt", "Run"));
    assert_eq!(run.arguments, vec![json!(2)]);
    let resolved = cx
        .update(|cx| features.resolve_code_lens(&context, &lenses[0], cx))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(resolved.range, p(0, 3)..p(0, 7));
    assert_eq!(resolved.command.unwrap().title, "3 references");
    // A lens from elsewhere has nothing to send back.
    assert!(
        cx.update(|cx| features.resolve_code_lens(&context, &CodeLens::default(), cx))
            .is_none()
    );

    // Document links, targets normalized and a link without one resolved.
    let links = cx.update(|cx| features.provide_links(&context, cx));
    let links = links.await.unwrap();
    assert_eq!(links[0].range, p(0, 0)..p(0, 2));
    assert_eq!(links[0].url.as_deref(), Some("file:///c%3A/a%20b.rs"));
    assert_eq!(links[0].tooltip.as_deref(), Some("Open a b"));
    assert_eq!(links[1].url, None);
    let resolved = cx
        .update(|cx| features.resolve_link(&links[1], cx))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(resolved.url.as_deref(), Some("https://example.com/"));
    assert_eq!(resolved.range, p(1, 0)..p(1, 2));

    // Colors and their presentations.
    let colors = cx.update(|cx| features.provide_document_colors(&context, cx));
    let colors = colors.await.unwrap();
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0].range, p(0, 3)..p(0, 7));
    let color = &colors[0].color;
    assert_eq!(
        (color.red, color.green, color.blue, color.alpha),
        (1.0, 0.5, 0.0, 1.0)
    );
    let presentations =
        cx.update(|cx| features.provide_color_presentations(&context, &colors[0], cx));
    let presentations = presentations.await.unwrap();
    assert_eq!(presentations[0].label, "#ff8000");
    assert_eq!(presentations[0].text_edit, None);
    assert_eq!(
        presentations[1].text_edit,
        Some((p(0, 3)..p(0, 7), "rgb(255, 128, 0)".to_string()))
    );
    assert_eq!(
        presentations[1].additional_text_edits,
        vec![(p(0, 0)..p(0, 0), "use rgb;\n".to_string())]
    );

    // Linked editing ranges, with the server's word pattern.
    let linked = cx.update(|cx| features.provide_linked_editing_ranges(&context, p(0, 2), cx));
    let linked = linked.await.unwrap().unwrap();
    assert_eq!(linked.ranges, vec![p(0, 1)..p(0, 4), p(0, 7)..p(0, 10)]);
    let pattern = linked.word_pattern.unwrap();
    assert_eq!(pattern.source, "[a-z]+");
    assert!(pattern.is_match("div"));
    let none = cx.update(|cx| features.provide_linked_editing_ranges(&context, p(1, 0), cx));
    assert!(none.await.unwrap().is_none());
    let sent = &server.messages("textDocument/linkedEditingRange")[0]["params"];
    assert_eq!(sent["position"], json!({ "line": 0, "character": 2 }));
    assert_eq!(sent["textDocument"]["uri"], "file:///project/main.rs");

    // `workspace/codeLens/refresh` is answered, not refused.
    server.request(9, "workspace/codeLens/refresh", Value::Null);
    cx.run_until_parked();
    let response = server.response(9);
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"], Value::Null);

    // Shutting down unregisters them.
    client
        .update(cx, |client, cx| client.shutdown(cx))
        .await
        .unwrap();
    cx.update(|cx| {
        assert!(code_lens_providers(cx, Some("rust")).is_empty());
        assert!(link_providers(cx, Some("rust")).is_empty());
        assert!(color_providers(cx, Some("rust")).is_empty());
        assert!(linked_editing_range_providers(cx, Some("rust")).is_empty());
    });
}

#[gpui::test]
async fn settings_changes_reach_the_server(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    let initialize = &server.messages("initialize")[0];
    assert_eq!(
        initialize["params"]["capabilities"]["workspace"]["didChangeConfiguration"],
        json!({})
    );
    client.update(cx, |client, _| {
        client.set_settings(json!({ "demo": { "option": 7 } }))
    });
    cx.run_until_parked();
    // Pull-model servers are told to ask again, as `vscode-languageclient` does.
    let changed = server.messages("workspace/didChangeConfiguration");
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["params"], json!({ "settings": null }));
    server.request(
        1,
        "workspace/configuration",
        json!({ "items": [{ "section": "demo.option" }] }),
    );
    cx.run_until_parked();
    assert_eq!(server.response(1)["result"], json!([7]));
    assert_eq!(
        client.read_with(cx, |client, _| client.settings().clone()),
        json!({ "demo": { "option": 7 } })
    );
}

#[gpui::test]
async fn message_requests_answer_with_the_chosen_action(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    let initialize = &server.messages("initialize")[0];
    assert_eq!(
        initialize["params"]["capabilities"]["window"]["showMessage"],
        json!({ "messageActionItem": { "additionalPropertiesSupport": true } })
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let sink = requests.clone();
    let keep = Arc::new(Mutex::new(true));
    let keeping = keep.clone();
    cx.update(|cx| {
        cx.subscribe(&client, move |_, event: &LspClientEvent, _| {
            if let LspClientEvent::ShowMessageRequest {
                kind,
                message,
                actions,
                respond,
            } = event
                && *keeping.lock().unwrap()
            {
                sink.lock().unwrap().push((
                    *kind,
                    message.clone(),
                    actions.clone(),
                    respond.clone(),
                ));
            }
        })
        .detach();
    });
    server.request(
        1,
        "window/showMessageRequest",
        json!({
            "type": 3,
            "message": "Reload workspace?",
            "actions": [{ "title": "Reload", "id": 7 }, { "title": "Later" }],
        }),
    );
    cx.run_until_parked();
    // The server waits for the user.
    assert_eq!(server.response(1), Value::Null);
    let (kind, message, actions, respond) = requests.lock().unwrap().pop().unwrap();
    assert_eq!(kind, lsp_types::MessageType::INFO);
    assert_eq!(message, "Reload workspace?");
    assert_eq!(actions.len(), 2);
    assert_eq!(
        actions[0].properties["id"],
        lsp_types::MessageActionItemProperty::Integer(7)
    );
    // The chosen action goes back with its properties; later answers don't.
    respond.respond(Some(actions[0].clone()));
    respond.respond(None);
    drop(respond);
    cx.run_until_parked();
    assert_eq!(
        server.response(1)["result"],
        json!({ "title": "Reload", "id": 7 })
    );
    assert_eq!(server.responses().len(), 1);

    // A request nobody keeps is dismissed.
    *keep.lock().unwrap() = false;
    server.request(
        2,
        "window/showMessageRequest",
        json!({ "type": 1, "message": "Oops" }),
    );
    cx.run_until_parked();
    assert_eq!(server.response(2)["result"], Value::Null);
    assert!(server.response(2).get("result").is_some());
}

#[gpui::test]
async fn will_save_and_will_save_wait_until(cx: &mut TestAppContext) {
    let mut capabilities = capabilities();
    capabilities["textDocumentSync"] =
        json!({ "openClose": true, "change": 2, "willSave": true, "willSaveWaitUntil": true });
    let (client, server) = start_with(cx, capabilities, |method, params| {
        match (method, params["reason"].as_u64()) {
            // A slow server for saves after a delay.
            ("textDocument/willSaveWaitUntil", Some(2)) => None,
            ("textDocument/willSaveWaitUntil", _) => Some(json!([{
                "range": range((0, 0), (0, 0)),
                "newText": "use a;\n",
            }])),
            _ => None,
        }
    })
    .await;
    let initialize = &server.messages("initialize")[0];
    let synchronization = &initialize["params"]["capabilities"]["textDocument"]["synchronization"];
    assert_eq!(synchronization["willSave"], true);
    assert_eq!(synchronization["willSaveWaitUntil"], true);
    let model = model(cx, "fn main() {}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    model.update(cx, |model, cx| model.edit([(0..0, "// x\n")], cx).unwrap());

    // The server sees the edit before it hears about the save.
    client.update(cx, |client, cx| {
        client.will_save(&model, lsp_types::TextDocumentSaveReason::MANUAL, cx)
    });
    cx.run_until_parked();
    let methods = server.methods();
    let will_save = methods
        .iter()
        .position(|m| m == "textDocument/willSave")
        .unwrap();
    assert_eq!(methods[will_save - 1], "textDocument/didChange");
    assert_eq!(
        server.messages("textDocument/willSave")[0]["params"]["reason"],
        1
    );

    // The server's edits are applied before the task resolves.
    client
        .update(cx, |client, cx| {
            client.will_save_wait_until(&model, lsp_types::TextDocumentSaveReason::MANUAL, cx)
        })
        .await;
    assert_eq!(
        model.read_with(cx, |m, _| m.text()),
        "use a;\n// x\nfn main() {}\n"
    );

    // A server that doesn't answer in time doesn't hold up the save.
    let task = client.update(cx, |client, cx| {
        client.will_save_wait_until(&model, lsp_types::TextDocumentSaveReason::AFTER_DELAY, cx)
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(crate::WILL_SAVE_WAIT_UNTIL_TIMEOUT);
    task.await;
    cx.run_until_parked();
    assert_eq!(server.messages("$/cancelRequest").len(), 1);
    assert_eq!(
        model.read_with(cx, |m, _| m.text()),
        "use a;\n// x\nfn main() {}\n"
    );

    // Servers that don't ask for them get neither.
    let (client, server) = start(cx, |_, _| None).await;
    let other = cx.new(|_| TextModel::new("x").with_uri("file:///project/other.rs"));
    client
        .update(cx, |client, cx| client.attach(&other, cx))
        .unwrap();
    client.update(cx, |client, cx| {
        client.will_save(&other, lsp_types::TextDocumentSaveReason::MANUAL, cx)
    });
    client
        .update(cx, |client, cx| {
            client.will_save_wait_until(&other, lsp_types::TextDocumentSaveReason::MANUAL, cx)
        })
        .await;
    cx.run_until_parked();
    assert!(server.messages("textDocument/willSave").is_empty());
    assert!(server.messages("textDocument/willSaveWaitUntil").is_empty());
}
