//! Pull diagnostics (`textDocument/diagnostic`, `workspace/diagnostic`).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext};
use serde_json::{Value, json};

use super::{capabilities, model, p, range, start};

/// [`super::start_with`] with `extra` added to the usual capabilities.
async fn start_with(
    cx: &mut TestAppContext,
    extra: Value,
    handler: impl Fn(&str, &Value) -> Option<Value> + Send + 'static,
) -> (gpui::Entity<crate::LspClient>, super::Server) {
    let mut all = capabilities();
    if let (Value::Object(all), Value::Object(extra)) = (&mut all, extra) {
        all.extend(extra);
    }
    super::start_with(cx, all, handler).await
}

/// Answers each `method` request with the next of `reports`.
fn reports(
    method: &'static str,
    reports: Vec<Value>,
) -> impl Fn(&str, &Value) -> Option<Value> + Send + 'static {
    let reports = Mutex::new(VecDeque::from(reports));
    move |m, _| {
        if m == method {
            reports.lock().unwrap().pop_front()
        } else {
            None
        }
    }
}

#[gpui::test]
async fn documents_are_pulled_after_open_and_edits(cx: &mut TestAppContext) {
    let (client, server) = start_with(
        cx,
        json!({ "diagnosticProvider": {
            "identifier": "fake",
            "interFileDependencies": false,
            "workspaceDiagnostics": false,
        } }),
        reports(
            "textDocument/diagnostic",
            vec![
                json!({
                    "kind": "full",
                    "resultId": "1",
                    "items": [{ "range": range((0, 4), (0, 5)), "message": "unused x" }],
                    "relatedDocuments": {
                        "file:///project/other.rs": {
                            "kind": "full",
                            "items": [{ "range": range((0, 0), (0, 1)), "message": "elsewhere" }],
                        },
                    },
                }),
                json!({ "kind": "unchanged", "resultId": "2" }),
                json!({ "kind": "full", "resultId": "3", "items": [] }),
            ],
        ),
    )
    .await;
    cx.update(gpui_monaco::init);
    let model = model(cx, "let x = 1;\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    cx.run_until_parked();

    // Opening pulls, with the server's identifier and no previous result.
    let pulls = server.messages("textDocument/diagnostic");
    assert_eq!(pulls.len(), 1);
    assert_eq!(
        pulls[0]["params"],
        json!({ "textDocument": { "uri": "file:///project/main.rs" }, "identifier": "fake" })
    );
    let markers = model.read_with(cx, |m, _| m.markers());
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].message, "unused x");
    assert_eq!(markers[0].range, p(0, 4)..p(0, 5));
    // Related documents get their diagnostics too.
    let listed = cx.update(|cx| {
        gpui_monaco::model_markers(
            cx,
            &gpui_monaco::MarkerFilter {
                resource: Some("file:///project/other.rs".into()),
                ..Default::default()
            },
        )
    });
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].marker.message, "elsewhere");

    // Edits are pulled once typing pauses, after they're synced.
    for column in [10, 11, 12] {
        model.update(cx, |model, cx| {
            model.edit([(column..column, " ")], cx).unwrap();
        });
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    }
    assert_eq!(server.messages("textDocument/diagnostic").len(), 1);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    let pulls = server.messages("textDocument/diagnostic");
    assert_eq!(pulls.len(), 2);
    assert_eq!(pulls[1]["params"]["previousResultId"], "1");
    let methods = server.methods();
    let last_change = methods
        .iter()
        .rposition(|m| m == "textDocument/didChange")
        .unwrap();
    let last_pull = methods
        .iter()
        .rposition(|m| m == "textDocument/diagnostic")
        .unwrap();
    assert!(last_change < last_pull);
    // An unchanged report keeps the markers.
    assert_eq!(model.read_with(cx, |m, _| m.markers()).len(), 1);

    // `workspace/diagnostic/refresh` pulls again at once.
    server.request(7, "workspace/diagnostic/refresh", Value::Null);
    cx.run_until_parked();
    assert_eq!(server.response(7)["result"], Value::Null);
    let pulls = server.messages("textDocument/diagnostic");
    assert_eq!(pulls.len(), 3);
    assert_eq!(pulls[2]["params"]["previousResultId"], "2");
    assert!(model.read_with(cx, |m, _| m.markers()).is_empty());

    // Detaching stops pulling.
    client.update(cx, |client, cx| client.detach(&model, cx));
    model.update(cx, |model, cx| {
        model.edit([(0..0, " ")], cx).unwrap();
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(server.messages("textDocument/diagnostic").len(), 3);
}

#[gpui::test]
async fn a_new_edit_cancels_the_pending_pull(cx: &mut TestAppContext) {
    // The server never answers.
    let (client, server) = start_with(
        cx,
        json!({ "diagnosticProvider": {
            "interFileDependencies": false,
            "workspaceDiagnostics": false,
        } }),
        |_, _| None,
    )
    .await;
    let model = model(cx, "x");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    cx.run_until_parked();
    let first = server.messages("textDocument/diagnostic");
    assert_eq!(first.len(), 1);
    assert!(first[0]["params"].get("identifier").is_none());
    model.update(cx, |model, cx| {
        model.edit([(1..1, "y")], cx).unwrap();
    });
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(server.messages("textDocument/diagnostic").len(), 2);
    let cancels = server.messages("$/cancelRequest");
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0]["params"]["id"], first[0]["id"]);
}

#[gpui::test]
async fn workspace_diagnostics_are_pulled(cx: &mut TestAppContext) {
    let (client, server) = start_with(
        cx,
        json!({ "diagnosticProvider": {
            "interFileDependencies": true,
            "workspaceDiagnostics": true,
        } }),
        reports(
            "workspace/diagnostic",
            vec![
                json!({ "items": [{
                    "kind": "full",
                    "uri": "file:///project/lib.rs",
                    "version": null,
                    "resultId": "w1",
                    "items": [{ "range": range((2, 0), (2, 3)), "message": "in lib" }],
                }] }),
                json!({ "items": [{
                    "kind": "full",
                    "uri": "file:///project/lib.rs",
                    "version": null,
                    "items": [],
                }] }),
            ],
        ),
    )
    .await;
    cx.update(gpui_monaco::init);
    let lib_markers = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            gpui_monaco::model_markers(
                cx,
                &gpui_monaco::MarkerFilter {
                    resource: Some("file:///project/lib.rs".into()),
                    ..Default::default()
                },
            )
        })
    };
    let pulls = server.messages("workspace/diagnostic");
    assert_eq!(pulls.len(), 1);
    assert_eq!(pulls[0]["params"], json!({ "previousResultIds": [] }));
    assert_eq!(lib_markers(cx).len(), 1);

    // It's pulled again a while after each response.
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(server.messages("workspace/diagnostic").len(), 2);
    assert!(lib_markers(cx).is_empty());

    // Diagnostics that depend on other files pull every attached document
    // after an edit to one.
    let main = model(cx, "a");
    let other = cx.new(|_| {
        gpui_monaco::TextModel::new("b")
            .with_uri("file:///project/other.rs")
            .with_language("rust")
    });
    for model in [&main, &other] {
        client
            .update(cx, |client, cx| client.attach(model, cx))
            .unwrap();
    }
    cx.run_until_parked();
    assert_eq!(server.messages("textDocument/diagnostic").len(), 2);
    main.update(cx, |model, cx| {
        model.edit([(1..1, "b")], cx).unwrap();
    });
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    let uris: Vec<Value> = server
        .messages("textDocument/diagnostic")
        .iter()
        .skip(2)
        .map(|m| m["params"]["textDocument"]["uri"].clone())
        .collect();
    assert_eq!(uris.len(), 2);
    assert!(uris.contains(&json!("file:///project/other.rs")));

    // Shutting down stops pulling.
    let pulled = server.messages("workspace/diagnostic").len();
    client.update(cx, |client, cx| client.shutdown(cx)).detach();
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(server.messages("workspace/diagnostic").len(), pulled);
}

#[gpui::test]
async fn servers_without_pull_diagnostics_are_not_pulled(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |_, _| None).await;
    let model = model(cx, "x");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    model.update(cx, |model, cx| {
        model.edit([(1..1, "y")], cx).unwrap();
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(server.messages("textDocument/diagnostic").is_empty());
    assert!(server.messages("workspace/diagnostic").is_empty());
    let initialize = &server.messages("initialize")[0]["params"]["capabilities"];
    assert_eq!(
        initialize["textDocument"]["diagnostic"]["relatedDocumentSupport"],
        true
    );
    assert_eq!(
        initialize["workspace"]["diagnostics"]["refreshSupport"],
        true
    );
}
