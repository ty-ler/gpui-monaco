//! `textDocument/inlineValue`, as `vscode-languageclient`'s
//! `InlineValueFeature` sends it for VS Code's `InlineValuesProvider`:
//! debuggers show the values while execution is paused.
//!
//! The bridge registers an inline values provider, so an editor showing
//! inline values (`CodeEditor::show_inline_values`) asks the server; hosts
//! can also ask with [`LspClient::inline_values`].
//! `workspace/inlineValue/refresh` asks the editors again
//! (`refresh_inline_values`) and emits
//! [`LspClientEvent::InlineValuesRefresh`](crate::LspClientEvent::InlineValuesRefresh).

use std::ops::Range;

use anyhow::Result;
use gpui::{App, Context, Entity, Task};
use gpui_monaco::TextModel;
use gpui_monaco::text::PointUtf16;
use lsp_types as lsp;

use crate::LspClient;
use crate::convert;
use crate::features::LspFeatures;

pub use gpui_monaco::providers::{InlineValue, InlineValueContext};

fn inline_value(value: lsp::InlineValue) -> InlineValue {
    match value {
        lsp::InlineValue::Text(value) => InlineValue::Text {
            range: convert::range(value.range),
            text: value.text.into(),
        },
        lsp::InlineValue::VariableLookup(value) => InlineValue::VariableLookup {
            range: convert::range(value.range),
            variable_name: value.variable_name.map(Into::into),
            case_sensitive_lookup: value.case_sensitive_lookup,
        },
        lsp::InlineValue::EvaluatableExpression(value) => InlineValue::EvaluatableExpression {
            range: convert::range(value.range),
            expression: value.expression.map(Into::into),
        },
    }
}

impl LspClient {
    /// Whether the server answers `textDocument/inlineValue`.
    pub fn supports_inline_values(&self) -> bool {
        matches!(
            self.capabilities().inline_value_provider,
            Some(lsp::OneOf::Left(true) | lsp::OneOf::Right(_))
        )
    }

    /// The values to show in `range` of `model`, usually the rows in view,
    /// while a debugger is paused at `context` (`textDocument/inlineValue`).
    /// Empty if `model` isn't attached, or the server doesn't support it or
    /// has stopped.
    pub fn inline_values(
        &mut self,
        model: &Entity<TextModel>,
        range: Range<PointUtf16>,
        context: InlineValueContext,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<InlineValue>>> {
        let uri = model.read(cx).resource().to_string();
        self.inline_values_at(&uri, range, context, cx)
    }

    /// [`LspClient::inline_values`] for the attached document `uri`.
    pub(crate) fn inline_values_at(
        &mut self,
        uri: &str,
        range: Range<PointUtf16>,
        context: InlineValueContext,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<InlineValue>>> {
        if !self.supports_inline_values() {
            return Task::ready(Ok(Vec::new()));
        }
        let Some((connection, text_document)) = self.prepare_request(uri, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request =
            connection.request::<lsp::request::InlineValueRequest>(lsp::InlineValueParams {
                work_done_progress_params: Default::default(),
                text_document,
                range: convert::lsp_range(&range),
                context: lsp::InlineValueContext {
                    frame_id: context.frame_id,
                    stopped_location: convert::lsp_range(&context.stopped_location),
                },
            });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(inline_value)
                .collect())
        })
    }
}

impl gpui_monaco::providers::InlineValuesProvider for LspFeatures {
    fn provide_inline_values(
        &self,
        context: &gpui_monaco::providers::ProviderContext,
        view_port: Range<PointUtf16>,
        inline_context: &InlineValueContext,
        cx: &mut App,
    ) -> Task<Result<Vec<InlineValue>>> {
        let Some(client) = self.client().upgrade() else {
            return Task::ready(Ok(Vec::new()));
        };
        let uri = context.resource.to_string();
        let inline_context = inline_context.clone();
        client.update(cx, |client, cx| {
            client.inline_values_at(&uri, view_port, inline_context, cx)
        })
    }
}
