//! Inline completions (`textDocument/inlineCompletion`).

use std::sync::Mutex;

use gpui::{Entity, TestAppContext};
use gpui_monaco::providers::*;
use serde_json::{Value, json};

use super::{Server, capabilities, model, p, range, start, start_with};
use crate::LspClient;

async fn start_inline(
    cx: &mut TestAppContext,
    provider: Value,
    handler: impl Fn(&str, &Value) -> Option<Value> + Send + 'static,
) -> (Entity<LspClient>, Server) {
    let mut all = capabilities();
    all["inlineCompletionProvider"] = provider;
    start_with(cx, all, handler).await
}

#[gpui::test]
async fn inline_completions_reach_the_server(cx: &mut TestAppContext) {
    let results = Mutex::new(vec![
        json!(null),
        json!([{ "insertText": "plain" }]),
        json!({ "items": [
            {
                "insertText": "len()",
                "filterText": "len",
                "range": range((0, 2), (0, 3)),
                "command": { "title": "Accepted", "command": "fake.doIt", "arguments": [1] },
            },
            { "insertText": { "kind": "snippet", "value": "len(${1:x})" } },
            { "insertText": "old(${1})", "insertTextFormat": 2 },
            { "range": range((0, 0), (0, 1)) },
        ] }),
    ]);
    let (client, server) = start_inline(cx, json!({}), move |method, _| {
        (method == "textDocument/inlineCompletion").then(|| results.lock().unwrap().pop().unwrap())
    })
    .await;
    assert!(client.read_with(cx, |client, _| client.supports_inline_completions()));
    let capabilities = &server.messages("initialize")[0]["params"]["capabilities"];
    assert_eq!(
        capabilities["textDocument"]["inlineCompletion"],
        json!({ "dynamicRegistration": false })
    );

    let model = model(cx, "x.l\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let providers = cx.update(|cx| inline_completions_providers(cx, Some("rust")));
    assert_eq!(providers.len(), 1);
    let provider = providers[0].clone();
    let context = model.read_with(cx, |m, _| m.provider_context());

    // A list, with the trigger kind and the suggest widget's selection.
    let completion_context = InlineCompletionContext {
        trigger_kind: InlineCompletionTriggerKind::Explicit,
        selected_suggestion_info: Some(SelectedSuggestionInfo {
            range: p(0, 2)..p(0, 3),
            text: "len".into(),
            completion_kind: CompletionItemKind::Method,
            is_snippet_text: false,
        }),
        ..Default::default()
    };
    let task = cx.update(|cx| {
        provider.provide_inline_completions(&context, p(0, 3), completion_context, cx)
    });
    cx.run_until_parked();
    let list = task.await.unwrap();
    let request = &server.messages("textDocument/inlineCompletion")[0]["params"];
    assert_eq!(
        *request,
        json!({
            "textDocument": { "uri": "file:///project/main.rs" },
            "position": { "line": 0, "character": 3 },
            "context": {
                "triggerKind": 1,
                "selectedCompletionInfo": { "range": range((0, 2), (0, 3)), "text": "len" },
            },
        })
    );
    // The item without `insertText` is dropped.
    assert_eq!(list.items.len(), 3);
    assert_eq!(list.items[0].insert_text, "len()");
    assert!(!list.items[0].is_snippet);
    assert_eq!(list.items[0].range, Some(p(0, 2)..p(0, 3)));
    let command = list.items[0].command.as_ref().unwrap();
    assert_eq!(command.id.as_ref(), "fake.doIt");
    assert_eq!(command.arguments, vec![json!(1)]);
    assert_eq!(list.items[1].insert_text, "len(${1:x})");
    assert!(list.items[1].is_snippet);
    assert!(list.items[2].is_snippet);

    // An array, and an automatic request without a selection.
    let task = cx.update(|cx| {
        provider.provide_inline_completions(&context, p(0, 3), Default::default(), cx)
    });
    cx.run_until_parked();
    let list = task.await.unwrap();
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].insert_text, "plain");
    assert_eq!(list.items[0].range, None);
    let request = &server.messages("textDocument/inlineCompletion")[1]["params"];
    assert_eq!(request["context"], json!({ "triggerKind": 2 }));

    // `null` is no completions.
    let task = cx.update(|cx| {
        provider.provide_inline_completions(&context, p(0, 3), Default::default(), cx)
    });
    cx.run_until_parked();
    assert!(task.await.unwrap().items.is_empty());

    // Shutting down unregisters it.
    client.update(cx, |client, cx| client.shutdown(cx)).detach();
    assert!(cx.update(|cx| inline_completions_providers(cx, Some("rust")).is_empty()));
}

#[gpui::test]
async fn inline_completions_need_the_capability(cx: &mut TestAppContext) {
    let (client, _server) = start(cx, |_, _| None).await;
    assert!(!client.read_with(cx, |client, _| client.supports_inline_completions()));
    assert!(cx.update(|cx| inline_completions_providers(cx, Some("rust")).is_empty()));
    drop(client);

    let (client, _server) = start_inline(cx, json!(false), |_, _| None).await;
    assert!(!client.read_with(cx, |client, _| client.supports_inline_completions()));
    assert!(cx.update(|cx| inline_completions_providers(cx, Some("rust")).is_empty()));
}
