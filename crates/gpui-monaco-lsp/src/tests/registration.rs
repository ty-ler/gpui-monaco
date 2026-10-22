use gpui::TestAppContext;
use gpui_monaco::providers::*;
use serde_json::json;

use super::{start, start_with};

fn target(language: &str, uri: &str) -> ProviderTarget {
    ProviderTarget {
        language_id: Some(language.to_string().into()),
        uri: Some(uri.to_string().into()),
    }
}

#[gpui::test]
async fn dynamic_registration(cx: &mut TestAppContext) {
    let (client, server) = start_with(cx, json!({ "textDocumentSync": 2 }), |_, _| None).await;
    let initialize = &server.messages("initialize")[0]["params"]["capabilities"];
    assert_eq!(
        initialize["textDocument"]["hover"]["dynamicRegistration"],
        true
    );
    assert_eq!(
        initialize["textDocument"]["completion"]["dynamicRegistration"],
        true
    );
    assert_eq!(
        initialize["workspace"]["executeCommand"]["dynamicRegistration"],
        true
    );
    assert_eq!(
        initialize["textDocument"]["synchronization"]["dynamicRegistration"],
        false
    );
    let rust = || target("rust", "file:///project/main.rs");
    assert!(cx.update(|cx| hover_providers(cx, rust())).is_empty());

    server.request(
        1,
        "client/registerCapability",
        json!({ "registrations": [
            // For the document selector's documents only.
            {
                "id": "hover",
                "method": "textDocument/hover",
                "registerOptions": { "documentSelector": [{ "language": "toml", "pattern": "**/Cargo.toml" }] },
            },
            // For the client's languages, with its options.
            {
                "id": "completion",
                "method": "textDocument/completion",
                "registerOptions": { "documentSelector": null, "triggerCharacters": [":"] },
            },
            { "id": "commands", "method": "workspace/executeCommand", "registerOptions": { "commands": ["dyn.run"] } },
            // `lsp-types` doesn't know it; the raw capabilities have it.
            { "id": "types", "method": "textDocument/prepareTypeHierarchy" },
            { "id": "inline", "method": "textDocument/inlineCompletion" },
            // Kept, though the bridge has no provider for it.
            { "id": "watch", "method": "workspace/didChangeWatchedFiles", "registerOptions": { "watchers": [] } },
        ] }),
    );
    cx.run_until_parked();
    assert_eq!(server.response(1)["result"], serde_json::Value::Null);
    assert!(server.response(1).get("error").is_none());
    let cargo = || target("toml", "file:///project/Cargo.toml");
    assert_eq!(cx.update(|cx| hover_providers(cx, cargo())).len(), 1);
    assert!(
        cx.update(|cx| hover_providers(cx, target("toml", "file:///project/a.toml")))
            .is_empty()
    );
    assert!(cx.update(|cx| hover_providers(cx, rust())).is_empty());
    let completions = cx.update(|cx| completion_providers(cx, rust()));
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].trigger_characters(), vec![':']);
    assert_eq!(
        cx.update(|cx| inline_completions_providers(cx, rust()))
            .len(),
        1
    );
    assert!(client.read_with(cx, |client, _| client.supports_inline_completions()));
    assert!(
        cx.update(|cx| completion_providers(cx, target("python", "file:///a.py")))
            .is_empty()
    );
    client.read_with(cx, |client, _| {
        let capabilities = client.capabilities();
        assert!(capabilities.hover_provider.is_some());
        assert!(capabilities.completion_provider.is_some());
        assert_eq!(
            capabilities
                .execute_command_provider
                .as_ref()
                .unwrap()
                .commands,
            ["dyn.run"]
        );
        assert_eq!(
            client.raw_capabilities()["typeHierarchyProvider"],
            json!({})
        );
    });
    // The registered command runs on the server.
    cx.update(|cx| gpui_monaco::execute_command(cx, &Command::new("dyn.run", "Run")))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        server.messages("workspace/executeCommand")[0]["params"]["command"],
        "dyn.run"
    );

    // Registering an id again replaces it.
    server.request(
        2,
        "client/registerCapability",
        json!({ "registrations": [{
            "id": "completion",
            "method": "textDocument/completion",
            "registerOptions": { "triggerCharacters": ["."] },
        }] }),
    );
    cx.run_until_parked();
    let completions = cx.update(|cx| completion_providers(cx, rust()));
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].trigger_characters(), vec!['.']);

    // Unregistering removes the providers and the capability.
    server.request(
        3,
        "client/unregisterCapability",
        json!({ "unregisterations": [
            { "id": "hover", "method": "textDocument/hover" },
            { "id": "commands", "method": "workspace/executeCommand" },
        ] }),
    );
    cx.run_until_parked();
    assert_eq!(server.response(3)["result"], serde_json::Value::Null);
    assert!(cx.update(|cx| hover_providers(cx, cargo())).is_empty());
    assert!(
        cx.update(|cx| gpui_monaco::execute_command(cx, &Command::new("dyn.run", "Run")))
            .is_err()
    );
    client.read_with(cx, |client, _| {
        assert!(client.capabilities().hover_provider.is_none());
        assert!(client.capabilities().execute_command_provider.is_none());
    });

    // Shutting down removes the rest.
    client
        .update(cx, |client, cx| client.shutdown(cx))
        .await
        .unwrap();
    assert!(cx.update(|cx| completion_providers(cx, rust())).is_empty());
}

#[gpui::test]
async fn invalid_registrations_are_rejected(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    server.request(1, "client/registerCapability", json!({ "nope": true }));
    // Options the feature can't use are skipped; the rest register.
    server.request(
        2,
        "client/registerCapability",
        json!({ "registrations": [
            { "id": "a", "method": "textDocument/onTypeFormatting", "registerOptions": { "firstTriggerCharacter": 5 } },
            { "id": "b", "method": "textDocument/documentSymbol" },
        ] }),
    );
    cx.run_until_parked();
    assert_eq!(server.response(1)["error"]["code"], -32602);
    assert_eq!(server.response(2)["result"], serde_json::Value::Null);
    let rust = target("rust", "file:///project/main.rs");
    assert_eq!(
        cx.update(|cx| document_symbol_providers(cx, rust.clone()))
            .len(),
        1
    );
    let on_type = cx.update(|cx| on_type_formatting_providers(cx, rust)).len();
    // Only the static one, from the fake server's capabilities.
    assert_eq!(on_type, 1);
    assert!(client.read_with(cx, |client, _| client.is_running()));
}
