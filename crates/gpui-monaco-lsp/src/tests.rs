//! The bridge against an in-process fake server.

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
    })
}

async fn start(
    cx: &mut TestAppContext,
    handler: impl Fn(&str, &Value) -> Option<Value> + Send + 'static,
) -> (Entity<LspClient>, Server) {
    let (client_reader, mut server_writer) = piper::pipe(1 << 20);
    let (server_reader, client_writer) = piper::pipe(1 << 20);
    let log = Arc::new(Mutex::new(Vec::new()));
    let (push, pushed) = async_channel::unbounded::<Value>();
    let handler: Handler = Box::new(move |method, params| match method {
        "initialize" => Some(json!({
            "capabilities": capabilities(),
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
                { "position": { "line": 0, "character": 7 }, "label": [{ "value": ": " }, { "value": "Foo" }], "kind": 1, "paddingLeft": true },
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
        .update(|cx| features.provide_folding_ranges(&context, cx))
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

    // Inlay hints, with label parts joined.
    let hints = cx
        .update(|cx| features.provide_inlay_hints(&context, p(0, 0)..p(1, 0), cx))
        .await
        .unwrap();
    assert_eq!(
        (
            hints[0].position,
            hints[0].label.as_ref(),
            hints[0].kind,
            hints[0].padding_left
        ),
        (p(0, 7), ": Foo", Some(InlayHintKind::Type), true)
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
        edits,
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
    assert_eq!(edits[0].0, p(0, 2)..p(0, 2));
    let edits = cx
        .update(|cx| features.provide_on_type_formatting_edits(&context, p(1, 3), ';', options, cx))
        .await
        .unwrap();
    assert_eq!(edits[0].1, "textDocument/onTypeFormatting");

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
    let responses = server.responses();
    assert_eq!(responses[0]["result"], json!([42, null]));
    assert_eq!(responses[1]["result"]["applied"], true);
    assert_eq!(responses[2]["error"]["code"], -32601);
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
