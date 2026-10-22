//! `textDocument/inlineCompletion` (LSP 3.18), as `vscode-languageclient`'s
//! `InlineCompletionItemFeature` sends it for VS Code's inline completion
//! provider: ghost text from servers such as Copilot's language server.
//!
//! `lsp-types` has these types only behind its `proposed` feature, and
//! its item can't hold a snippet `StringValue`, so the request and the
//! result are JSON here.

use anyhow::Result;
use gpui::{App, Task, WeakEntity};
use gpui_monaco::providers::{
    InlineCompletion, InlineCompletionContext, InlineCompletionList, InlineCompletionTriggerKind,
    InlineCompletionsProvider, LanguageSelector, ProviderContext, ProviderRegistration,
    register_inline_completions_provider,
};
use gpui_monaco::text::PointUtf16;
use lsp_types as lsp;
use serde_json::{Value, json};

use crate::LspClient;
use crate::convert;

const METHOD: &str = "textDocument/inlineCompletion";

/// The inline completions provider for a server with an
/// `inlineCompletionProvider`.
struct LspInlineCompletions {
    client: WeakEntity<LspClient>,
}

impl LspClient {
    /// Whether the server answers `textDocument/inlineCompletion`.
    pub fn supports_inline_completions(&self) -> bool {
        match self.raw_capabilities().get("inlineCompletionProvider") {
            Some(Value::Bool(supported)) => *supported,
            Some(Value::Object(_)) => true,
            _ => false,
        }
    }
}

/// Registers the provider for `selector` if `client`'s server has
/// inline completions.
pub(crate) fn register(
    client: &LspClient,
    weak: WeakEntity<LspClient>,
    selector: LanguageSelector,
    cx: &mut App,
) -> Option<ProviderRegistration> {
    client.supports_inline_completions().then(|| {
        register_inline_completions_provider(cx, selector, LspInlineCompletions { client: weak })
    })
}

impl InlineCompletionsProvider for LspInlineCompletions {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        completion_context: InlineCompletionContext,
        cx: &mut App,
    ) -> Task<Result<InlineCompletionList>> {
        let Some(client) = self.client.upgrade() else {
            return Task::ready(Ok(InlineCompletionList::default()));
        };
        let uri = &context.resource;
        let Some((connection, text_document)) =
            client.update(cx, |client, cx| client.prepare_request(uri, cx))
        else {
            return Task::ready(Ok(InlineCompletionList::default()));
        };
        let request = connection.request_raw(
            METHOD,
            json!({
                "textDocument": text_document,
                "position": convert::position(position),
                "context": lsp_context(&completion_context),
            }),
        );
        cx.background_executor()
            .spawn(async move { completion_list(request.await?) })
    }
}

/// The request's `InlineCompletionContext`.
fn lsp_context(context: &InlineCompletionContext) -> Value {
    let trigger_kind = match context.trigger_kind {
        InlineCompletionTriggerKind::Explicit => 1,
        InlineCompletionTriggerKind::Automatic => 2,
    };
    let mut lsp_context = json!({ "triggerKind": trigger_kind });
    if let Some(selected) = &context.selected_suggestion_info {
        lsp_context["selectedCompletionInfo"] = json!({
            "range": convert::lsp_range(&selected.range),
            "text": selected.text,
        });
    }
    lsp_context
}

/// The result: `InlineCompletionItem[] | InlineCompletionList | null`.
pub(crate) fn completion_list(result: Value) -> Result<InlineCompletionList> {
    let items = match result {
        Value::Null => return Ok(InlineCompletionList::default()),
        Value::Array(items) => items,
        Value::Object(mut list) => match list.remove("items") {
            Some(Value::Array(items)) => items,
            _ => anyhow::bail!("invalid {METHOD} result"),
        },
        _ => anyhow::bail!("invalid {METHOD} result"),
    };
    Ok(InlineCompletionList {
        items: items.into_iter().filter_map(completion_item).collect(),
        ..Default::default()
    })
}

/// An `InlineCompletionItem` (`vscode-languageclient`'s
/// `asInlineCompletionItem`). `insertText` is a string or a snippet
/// `StringValue`. The editor has no `filterText` yet (#582), so it's dropped.
fn completion_item(item: Value) -> Option<InlineCompletion> {
    let (insert_text, is_snippet) = match item.get("insertText")? {
        Value::String(text) => (
            text.clone(),
            // The 3.18 drafts had `insertTextFormat`.
            item.get("insertTextFormat") == Some(&json!(2)),
        ),
        Value::Object(value) => (
            value.get("value")?.as_str()?.to_string(),
            value.get("kind").and_then(Value::as_str) == Some("snippet"),
        ),
        _ => return None,
    };
    let range = item
        .get("range")
        .and_then(|range| serde_json::from_value::<lsp::Range>(range.clone()).ok())
        .map(convert::range);
    let command = item
        .get("command")
        .and_then(|command| serde_json::from_value::<lsp::Command>(command.clone()).ok())
        .map(convert::command);
    Some(InlineCompletion {
        insert_text,
        is_snippet,
        range,
        command,
        ..Default::default()
    })
}
