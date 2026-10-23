use std::sync::{Arc, Mutex};

use gpui::TestAppContext;
use serde_json::json;

use super::{model, p, range, start};
use crate::{InlineValue, InlineValueContext, LspClientEvent};

#[gpui::test]
async fn inline_values(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, _| match method {
        "textDocument/inlineValue" => Some(json!([
            { "range": range((1, 4), (1, 5)), "text": "x = 1" },
            { "range": range((2, 4), (2, 5)), "variableName": "y", "caseSensitiveLookup": true },
            { "range": range((3, 4), (3, 9)), "caseSensitiveLookup": false },
            { "range": range((4, 4), (4, 9)), "expression": "a.b" },
            { "range": range((5, 4), (5, 9)) },
        ])),
        _ => None,
    })
    .await;
    let init = &server.messages("initialize")[0]["params"]["capabilities"];
    assert_eq!(
        init["textDocument"]["inlineValue"],
        json!({ "dynamicRegistration": true })
    );
    assert_eq!(
        init["workspace"]["inlineValue"],
        json!({ "refreshSupport": true })
    );

    let model = model(cx, "fn main() {\n    x\n    y\n    z\n}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let values = client.update(cx, |client, cx| {
        client.inline_values(
            &model,
            p(0, 0)..p(6, 0),
            InlineValueContext {
                frame_id: 3,
                stopped_location: p(2, 0)..p(2, 5),
            },
            cx,
        )
    });
    cx.run_until_parked();
    let values = values.await.unwrap();
    let sent = &server.messages("textDocument/inlineValue")[0]["params"];
    assert_eq!(sent["textDocument"]["uri"], "file:///project/main.rs");
    assert_eq!(sent["range"], range((0, 0), (6, 0)));
    assert_eq!(
        sent["context"],
        json!({ "frameId": 3, "stoppedLocation": range((2, 0), (2, 5)) })
    );
    assert_eq!(
        values,
        vec![
            InlineValue::Text {
                range: p(1, 4)..p(1, 5),
                text: "x = 1".into(),
            },
            InlineValue::VariableLookup {
                range: p(2, 4)..p(2, 5),
                variable_name: Some("y".into()),
                case_sensitive_lookup: true,
            },
            InlineValue::VariableLookup {
                range: p(3, 4)..p(3, 9),
                variable_name: None,
                case_sensitive_lookup: false,
            },
            InlineValue::EvaluatableExpression {
                range: p(4, 4)..p(4, 9),
                expression: Some("a.b".into()),
            },
            InlineValue::EvaluatableExpression {
                range: p(5, 4)..p(5, 9),
                expression: None,
            },
        ]
    );
    assert_eq!(values[4].range(), &(p(5, 4)..p(5, 9)));

    // `workspace/inlineValue/refresh` tells the host to ask again.
    let refreshed = Arc::new(Mutex::new(0));
    let _subscription = cx.update({
        let refreshed = refreshed.clone();
        |cx| {
            cx.subscribe(&client, move |_, event: &LspClientEvent, _| {
                if matches!(event, LspClientEvent::InlineValuesRefresh) {
                    *refreshed.lock().unwrap() += 1;
                }
            })
        }
    });
    server.request(41, "workspace/inlineValue/refresh", json!(null));
    cx.run_until_parked();
    assert_eq!(*refreshed.lock().unwrap(), 1);
    assert_eq!(server.response(41)["result"], json!(null));
}

/// The bridge registers an inline values provider for editors showing
/// inline values.
#[gpui::test]
async fn inline_values_provider(cx: &mut TestAppContext) {
    let (client, server) = start(cx, |method, _| match method {
        "textDocument/inlineValue" => Some(json!([
            { "range": range((1, 4), (1, 5)), "variableName": "x", "caseSensitiveLookup": true },
        ])),
        _ => None,
    })
    .await;
    let model = model(cx, "fn main() {\n    x\n}\n");
    client
        .update(cx, |client, cx| client.attach(&model, cx))
        .unwrap();
    let provider = cx.update(|cx| {
        let providers = gpui_monaco::providers::inline_values_providers(cx, model.read(cx));
        assert_eq!(providers.len(), 1);
        providers[0].clone()
    });
    let context = model.read_with(cx, |m, _| m.provider_context());
    let values = cx.update(|cx| {
        provider.provide_inline_values(
            &context,
            p(0, 0)..p(2, 1),
            &InlineValueContext {
                frame_id: 5,
                stopped_location: p(1, 4)..p(1, 5),
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(
        values.await.unwrap(),
        vec![InlineValue::VariableLookup {
            range: p(1, 4)..p(1, 5),
            variable_name: Some("x".into()),
            case_sensitive_lookup: true,
        }]
    );
    let sent = &server.messages("textDocument/inlineValue")[0]["params"];
    assert_eq!(sent["context"]["frameId"], 5);
    assert_eq!(sent["range"], range((0, 0), (2, 1)));
}
