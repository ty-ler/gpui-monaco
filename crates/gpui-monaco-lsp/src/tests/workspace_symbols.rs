use gpui::TestAppContext;
use gpui_monaco::providers::SymbolKind;
use serde_json::json;

use super::{p, range, start};

#[gpui::test]
async fn workspace_symbols_and_resolve(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, params| match method {
        "workspace/symbol" if params["query"] == "flat" => Some(json!([
            {
                "name": "Old",
                "kind": 5,
                "deprecated": true,
                "containerName": "lib",
                "location": { "uri": "file:///C:/proj/a.rs", "range": range((1, 4), (1, 7)) },
            },
        ])),
        "workspace/symbol" => Some(json!([
            {
                "name": "main",
                "kind": 12,
                "tags": [1],
                "location": { "uri": "file:///C:/proj/main.rs" },
                "data": { "id": 7 },
            },
        ])),
        "workspaceSymbol/resolve" => {
            let mut symbol = params.clone();
            symbol["location"]["range"] = range((2, 3), (2, 7));
            Some(symbol)
        }
        _ => None,
    })
    .await;

    let init = &server.messages("initialize")[0]["params"]["capabilities"]["workspace"];
    assert_eq!(
        init["symbol"]["resolveSupport"]["properties"],
        json!(["location.range"])
    );
    assert_eq!(init["symbol"]["symbolKind"]["valueSet"][25], 26);

    // A flat `SymbolInformation`: its uri normalized, `deprecated` kept.
    let flat = client.read_with(cx, |client, cx| client.workspace_symbols("flat", cx));
    cx.run_until_parked();
    let flat = flat.await.unwrap();
    assert_eq!(
        server.messages("workspace/symbol")[0]["params"]["query"],
        "flat"
    );
    assert_eq!(flat.len(), 1);
    assert_eq!(
        (
            flat[0].name.as_ref(),
            flat[0].kind,
            flat[0].deprecated,
            flat[0].container_name.as_deref(),
            flat[0].uri.as_ref(),
            flat[0].range.clone(),
        ),
        (
            "Old",
            SymbolKind::Class,
            true,
            Some("lib"),
            "file:///c%3A/proj/a.rs",
            Some(p(1, 4)..p(1, 7)),
        )
    );

    // A `WorkspaceSymbol` with only its document: resolving sends it back
    // as the server wrote it and fills in the range.
    let symbols = client.read_with(cx, |client, cx| client.workspace_symbols("", cx));
    cx.run_until_parked();
    let symbols = symbols.await.unwrap();
    let main = &symbols[0];
    assert_eq!(
        (
            main.kind,
            main.deprecated,
            main.uri.as_ref(),
            main.range.clone()
        ),
        (
            SymbolKind::Function,
            true,
            "file:///c%3A/proj/main.rs",
            None
        )
    );
    let resolved = client.read_with(cx, |client, cx| client.resolve_workspace_symbol(main, cx));
    cx.run_until_parked();
    let resolved = resolved.await.unwrap();
    let sent = &server.messages("workspaceSymbol/resolve")[0]["params"];
    assert_eq!(sent["data"], json!({ "id": 7 }));
    assert_eq!(
        sent["location"],
        json!({ "uri": "file:///C:/proj/main.rs" })
    );
    assert_eq!(resolved.range, Some(p(2, 3)..p(2, 7)));
    assert_eq!(resolved.name.as_ref(), "main");
}
