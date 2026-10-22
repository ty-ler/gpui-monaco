use gpui::TestAppContext;
use gpui_monaco::providers::SymbolKind;
use serde_json::{Value, json};

use super::{model, p, range, start};

fn server_item(name: &str, uri: &str, row: u32) -> Value {
    json!({
        "name": name,
        "kind": 5,
        "uri": uri,
        "range": range((row, 0), (row + 3, 1)),
        "selectionRange": range((row, 6), (row, 6 + name.len() as u32)),
        "data": { "name": name },
    })
}

#[gpui::test]
async fn type_hierarchy(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, params| match method {
        "textDocument/prepareTypeHierarchy" => {
            let mut item = server_item("Dog", "file:///project/main.rs", 2);
            // A list, as the spec has it (`lsp-types` 0.97 can't read it).
            item["tags"] = json!([1]);
            item["detail"] = json!("animals");
            Some(json!([item]))
        }
        "typeHierarchy/supertypes" => {
            assert_eq!(params["item"]["data"], json!({ "name": "Dog" }));
            Some(json!([server_item("Animal", "file:///C:/proj/a.rs", 0)]))
        }
        "typeHierarchy/subtypes" => Some(Value::Null),
        _ => None,
    })
    .await;
    assert!(client.read_with(cx, |client, _| client.supports_type_hierarchy()));
    assert_eq!(
        server.messages("initialize")[0]["params"]["capabilities"]["textDocument"]["typeHierarchy"],
        json!({ "dynamicRegistration": true })
    );

    let model = model(cx, "fn main() {}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let items = client.update(cx, |client, cx| {
        client.prepare_type_hierarchy(&model, p(2, 7), cx)
    });
    cx.run_until_parked();
    let items = items.await.unwrap();
    let prepare = &server.messages("textDocument/prepareTypeHierarchy")[0]["params"];
    assert_eq!(prepare["textDocument"]["uri"], "file:///project/main.rs");
    assert_eq!(prepare["position"], json!({ "line": 2, "character": 7 }));
    let dog = &items[0];
    assert_eq!(
        (
            dog.name.as_ref(),
            dog.kind,
            dog.deprecated,
            dog.detail.as_deref(),
            dog.uri.as_ref(),
            dog.range.clone(),
            dog.selection_range.clone(),
        ),
        (
            "Dog",
            SymbolKind::Class,
            true,
            Some("animals"),
            "file:///project/main.rs",
            p(2, 0)..p(5, 1),
            p(2, 6)..p(2, 9),
        )
    );

    // The server's item goes back as it sent it, tags and all.
    let supertypes = client.read_with(cx, |client, cx| client.supertypes(dog, cx));
    cx.run_until_parked();
    let supertypes = supertypes.await.unwrap();
    let sent = &server.messages("typeHierarchy/supertypes")[0]["params"]["item"];
    assert_eq!(sent["tags"], json!([1]));
    assert_eq!(*sent, *dog.server_item());
    assert_eq!(supertypes[0].name.as_ref(), "Animal");
    assert_eq!(supertypes[0].uri.as_ref(), "file:///c%3A/proj/a.rs");
    assert!(!supertypes[0].deprecated);

    let subtypes = client.read_with(cx, |client, cx| client.subtypes(dog, cx));
    cx.run_until_parked();
    assert!(subtypes.await.unwrap().is_empty());
    assert_eq!(server.messages("typeHierarchy/subtypes").len(), 1);
}

/// The bridge registers a type hierarchy provider for the editor's Peek
/// Type Hierarchy (from the raw capabilities, which `lsp-types` lacks),
/// which sends the server's item back as it came.
#[gpui::test]
async fn type_hierarchy_provider(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, _| match method {
        "textDocument/prepareTypeHierarchy" => {
            let mut item = server_item("Dog", "file:///project/main.rs", 2);
            item["tags"] = json!([1]);
            Some(json!([item]))
        }
        "typeHierarchy/supertypes" => {
            Some(json!([server_item("Animal", "file:///project/main.rs", 0)]))
        }
        _ => None,
    })
    .await;
    let model = model(cx, "fn main() {}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let provider = cx.update(|cx| {
        let providers = gpui_monaco::providers::type_hierarchy_providers(cx, model.read(cx));
        assert_eq!(providers.len(), 1);
        providers[0].clone()
    });
    let context = model.read_with(cx, |m, _| m.provider_context());
    let items = cx.update(|cx| provider.prepare_type_hierarchy(&context, p(2, 7), cx));
    cx.run_until_parked();
    let items = items.await.unwrap();
    assert_eq!(
        (items[0].name.as_ref(), items[0].kind),
        ("Dog", SymbolKind::Class)
    );
    assert!(items[0].deprecated);
    let supertypes = cx.update(|cx| provider.provide_supertypes(&items[0], cx));
    cx.run_until_parked();
    assert_eq!(supertypes.await.unwrap()[0].name.as_ref(), "Animal");
    let sent = &server.messages("typeHierarchy/supertypes")[0]["params"]["item"];
    assert_eq!(sent["tags"], json!([1]));
    assert_eq!(sent["data"], json!({ "name": "Dog" }));
}
