//! `textDocument/inlineValue`, as `vscode-languageclient`'s
//! `InlineValueFeature` sends it for VS Code's `InlineValuesProvider`:
//! debuggers show the values while execution is paused.
//!
//! gpui-monaco has no inline values provider or rendering (Monaco declares
//! the provider, but VS Code's debugger draws the values), so this is an API
//! on [`LspClient`] for hosts that embed a debugger.
//! `workspace/inlineValue/refresh` is
//! [`LspClientEvent::InlineValuesRefresh`](crate::LspClientEvent::InlineValuesRefresh).

use std::ops::Range;

use anyhow::Result;
use gpui::{Context, Entity, SharedString, Task};
use gpui_monaco::TextModel;
use gpui_monaco::text::PointUtf16;
use lsp_types as lsp;

use crate::LspClient;
use crate::convert;

/// Where the debugger stopped (VS Code `InlineValueContext`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineValueContext {
    /// The stack frame (the debug adapter protocol's `frameId`) the values
    /// are for.
    pub frame_id: i32,
    /// The range where execution stopped, usually the current line.
    pub stopped_location: Range<PointUtf16>,
}

/// A value to show, or how to find it (VS Code `InlineValue`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineValue {
    /// Text to show as is (`InlineValueText`).
    Text {
        /// The range it's for.
        range: Range<PointUtf16>,
        /// The text.
        text: SharedString,
    },
    /// A variable whose value the debugger looks up
    /// (`InlineValueVariableLookup`).
    VariableLookup {
        /// The range it's for.
        range: Range<PointUtf16>,
        /// The variable's name; the text in `range` if `None`.
        variable_name: Option<SharedString>,
        /// Whether the name is looked up case sensitively.
        case_sensitive_lookup: bool,
    },
    /// An expression the debugger evaluates
    /// (`InlineValueEvaluatableExpression`).
    EvaluatableExpression {
        /// The range it's for.
        range: Range<PointUtf16>,
        /// The expression; the text in `range` if `None`.
        expression: Option<SharedString>,
    },
}

impl InlineValue {
    /// The range it's for.
    pub fn range(&self) -> &Range<PointUtf16> {
        match self {
            Self::Text { range, .. }
            | Self::VariableLookup { range, .. }
            | Self::EvaluatableExpression { range, .. } => range,
        }
    }
}

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
        if !self.supports_inline_values() {
            return Task::ready(Ok(Vec::new()));
        }
        let uri = model.read(cx).resource().to_string();
        let Some((connection, text_document)) = self.prepare_request(&uri, cx) else {
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
