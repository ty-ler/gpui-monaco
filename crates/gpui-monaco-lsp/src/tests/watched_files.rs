use std::sync::{Arc, Mutex};

use gpui::TestAppContext;
use lsp_types::{FileChangeType, FileEvent, GlobPattern, Uri};
use serde_json::json;

use super::start;
use crate::LspClientEvent;

#[gpui::test]
async fn watched_files(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    let init = &server.messages("initialize")[0]["params"]["capabilities"];
    assert_eq!(
        init["workspace"]["didChangeWatchedFiles"],
        json!({ "dynamicRegistration": true, "relativePatternSupport": true })
    );
    let changed = Arc::new(Mutex::new(0));
    let count = changed.clone();
    cx.update(|cx| {
        cx.subscribe(&client, move |_, event: &LspClientEvent, _| {
            if matches!(event, LspClientEvent::FileWatchersChanged) {
                *count.lock().unwrap() += 1;
            }
        })
        .detach();
    });
    assert!(
        client
            .read_with(cx, |client, _| client.file_watchers())
            .is_empty()
    );

    // The host learns what to watch.
    server.request(
        1,
        "client/registerCapability",
        json!({ "registrations": [{
            "id": "watch",
            "method": "workspace/didChangeWatchedFiles",
            "registerOptions": { "watchers": [
                { "globPattern": "**/Cargo.toml" },
                { "globPattern": { "baseUri": "file:///project", "pattern": "src/**/*.rs" }, "kind": 4 },
            ] },
        }] }),
    );
    cx.run_until_parked();
    let watchers = client.read_with(cx, |client, _| client.file_watchers());
    assert_eq!(watchers.len(), 2);
    assert_eq!(
        watchers[0].glob_pattern,
        GlobPattern::String("**/Cargo.toml".into())
    );
    assert!(matches!(watchers[1].glob_pattern, GlobPattern::Relative(_)));
    assert_eq!(watchers[1].kind, Some(lsp_types::WatchKind::Delete));
    assert_eq!(*changed.lock().unwrap(), 1);

    // Another registration that isn't a watcher doesn't say they changed.
    server.request(
        2,
        "client/registerCapability",
        json!({ "registrations": [{ "id": "hover", "method": "textDocument/hover" }] }),
    );
    cx.run_until_parked();
    assert_eq!(*changed.lock().unwrap(), 1);

    // The host's changes go to the server.
    let uri = |s: &str| s.parse::<Uri>().unwrap();
    client.read_with(cx, |client, _| {
        client.did_change_watched_files(Vec::new());
        client.did_change_watched_files(vec![
            FileEvent::new(uri("file:///project/Cargo.toml"), FileChangeType::CHANGED),
            FileEvent::new(uri("file:///project/src/gen.rs"), FileChangeType::DELETED),
        ]);
    });
    cx.run_until_parked();
    let notifications = server.messages("workspace/didChangeWatchedFiles");
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0]["params"],
        json!({ "changes": [
            { "uri": "file:///project/Cargo.toml", "type": 2 },
            { "uri": "file:///project/src/gen.rs", "type": 3 },
        ] })
    );

    // Unregistering stops the watching.
    server.request(
        3,
        "client/unregisterCapability",
        json!({ "unregisterations": [{ "id": "watch", "method": "workspace/didChangeWatchedFiles" }] }),
    );
    cx.run_until_parked();
    assert!(
        client
            .read_with(cx, |client, _| client.file_watchers())
            .is_empty()
    );
    assert_eq!(*changed.lock().unwrap(), 2);
}
