use gpui::TestAppContext;
use gpui_monaco::providers::SymbolKind;
use serde_json::{Value, json};

use super::{model, p, range, start};

fn server_item(name: &str, uri: &str, row: u32) -> Value {
    json!({
        "name": name,
        "kind": 12,
        "detail": format!("fn {name}()"),
        "uri": uri,
        "range": range((row, 0), (row + 2, 1)),
        "selectionRange": range((row, 3), (row, 3 + name.len() as u32)),
        "data": { "name": name },
    })
}

#[gpui::test]
async fn call_hierarchy(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, params| match method {
        "textDocument/prepareCallHierarchy" => {
            let mut item = server_item("main", "file:///project/main.rs", 0);
            item["tags"] = json!([1]);
            Some(json!([item]))
        }
        "callHierarchy/incomingCalls" => Some(json!([{
            "from": server_item("caller", "file:///C:/proj/a.rs", 4),
            "fromRanges": [range((5, 4), (5, 8)), range((6, 4), (6, 8))],
        }])),
        "callHierarchy/outgoingCalls" => {
            assert_eq!(params["item"]["data"], json!({ "name": "main" }));
            Some(json!([{
                "to": server_item("helper", "file:///project/main.rs", 10),
                "fromRanges": [range((1, 4), (1, 10))],
            }]))
        }
        _ => None,
    })
    .await;
    assert_eq!(
        server.messages("initialize")[0]["params"]["capabilities"]["textDocument"]["callHierarchy"],
        json!({ "dynamicRegistration": true })
    );

    // Edits reach the server before the request.
    let model = model(cx, "fn main() {}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    model.update(cx, |m, cx| m.edit([(0..0, "// x\n")], cx).unwrap());
    let items = client.update(cx, |client, cx| {
        client.prepare_call_hierarchy(&model, p(1, 4), cx)
    });
    cx.run_until_parked();
    let items = items.await.unwrap();
    assert_eq!(
        server.methods().last().map(String::as_str),
        Some("textDocument/prepareCallHierarchy")
    );
    assert_eq!(server.messages("textDocument/didChange").len(), 1);
    let prepare = &server.messages("textDocument/prepareCallHierarchy")[0]["params"];
    assert_eq!(prepare["textDocument"]["uri"], "file:///project/main.rs");
    assert_eq!(prepare["position"], json!({ "line": 1, "character": 4 }));
    let main = &items[0];
    assert_eq!(
        (
            main.name.as_ref(),
            main.kind,
            main.deprecated,
            main.detail.as_deref(),
            main.uri.as_ref(),
            main.range.clone(),
            main.selection_range.clone(),
        ),
        (
            "main",
            SymbolKind::Function,
            true,
            Some("fn main()"),
            "file:///project/main.rs",
            p(0, 0)..p(2, 1),
            p(0, 3)..p(0, 7),
        )
    );

    // The server's item goes back as it sent it; the callers' uris are
    // normalized.
    let incoming = client.read_with(cx, |client, cx| client.incoming_calls(main, cx));
    cx.run_until_parked();
    let incoming = incoming.await.unwrap();
    let sent = &server.messages("callHierarchy/incomingCalls")[0]["params"]["item"];
    assert_eq!(sent["tags"], json!([1]));
    assert_eq!(sent["data"], json!({ "name": "main" }));
    assert_eq!(incoming[0].from.name.as_ref(), "caller");
    assert_eq!(incoming[0].from.uri.as_ref(), "file:///c%3A/proj/a.rs");
    assert_eq!(
        incoming[0].from_ranges,
        vec![p(5, 4)..p(5, 8), p(6, 4)..p(6, 8)]
    );

    let outgoing = client.read_with(cx, |client, cx| client.outgoing_calls(main, cx));
    cx.run_until_parked();
    let outgoing = outgoing.await.unwrap();
    assert_eq!(outgoing[0].to.name.as_ref(), "helper");
    assert_eq!(outgoing[0].to.selection_range, p(10, 3)..p(10, 9));
    assert_eq!(outgoing[0].from_ranges, vec![p(1, 4)..p(1, 10)]);

    // A model that isn't attached asks nothing.
    client.update(cx, |client, cx| client.detach(&model, cx));
    let none = client.update(cx, |client, cx| {
        client.prepare_call_hierarchy(&model, p(0, 0), cx)
    });
    cx.run_until_parked();
    assert!(none.await.unwrap().is_empty());
    assert_eq!(
        server.messages("textDocument/prepareCallHierarchy").len(),
        1
    );
}

/// The bridge registers a call hierarchy provider for the editor's Peek
/// Call Hierarchy, which sends the server's item back as it came.
#[gpui::test]
async fn call_hierarchy_provider(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, _| match method {
        "textDocument/prepareCallHierarchy" => {
            Some(json!([server_item("main", "file:///project/main.rs", 0)]))
        }
        "callHierarchy/incomingCalls" => Some(json!([{
            "from": server_item("caller", "file:///project/main.rs", 4),
            "fromRanges": [range((5, 4), (5, 8))],
        }])),
        _ => None,
    })
    .await;
    let model = model(cx, "fn main() {}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let provider = cx.update(|cx| {
        let providers = gpui_monaco::providers::call_hierarchy_providers(cx, model.read(cx));
        assert_eq!(providers.len(), 1);
        providers[0].clone()
    });
    let context = model.read_with(cx, |m, _| m.provider_context());
    let items = cx.update(|cx| provider.prepare_call_hierarchy(&context, p(0, 4), cx));
    cx.run_until_parked();
    let items = items.await.unwrap();
    assert_eq!(
        (items[0].name.as_ref(), items[0].uri.as_deref()),
        ("main", Some("file:///project/main.rs"))
    );
    let incoming = cx.update(|cx| provider.provide_incoming_calls(&items[0], cx));
    cx.run_until_parked();
    let incoming = incoming.await.unwrap();
    assert_eq!(incoming[0].from.name.as_ref(), "caller");
    assert_eq!(incoming[0].from_ranges, vec![p(5, 4)..p(5, 8)]);
    let sent = &server.messages("callHierarchy/incomingCalls")[0]["params"]["item"];
    assert_eq!(sent["data"], json!({ "name": "main" }));
    assert_eq!(sent["detail"], "fn main()");
}
