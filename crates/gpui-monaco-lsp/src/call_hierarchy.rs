//! `textDocument/prepareCallHierarchy`, `callHierarchy/incomingCalls` and
//! `callHierarchy/outgoingCalls`, as `vscode-languageclient`'s
//! `CallHierarchyFeature` sends them for VS Code's call hierarchy view.
//!
//! gpui-monaco has no call hierarchy provider or peek view (Monaco has
//! neither), so this is an API on [`LspClient`] for hosts to build that UI
//! on.

use std::ops::Range;

use anyhow::Result;
use gpui::{App, Context, Entity, SharedString, Task};
use gpui_monaco::TextModel;
use gpui_monaco::providers::SymbolKind;
use gpui_monaco::text::PointUtf16;
use lsp_types as lsp;

use crate::LspClient;
use crate::convert;

/// A function, method or other callable in a call hierarchy (VS Code
/// `CallHierarchyItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct CallHierarchyItem {
    /// Its name.
    pub name: SharedString,
    /// Its kind.
    pub kind: SymbolKind,
    /// Tagged deprecated.
    pub deprecated: bool,
    /// More detail, such as its signature.
    pub detail: Option<SharedString>,
    /// Its document, in [`TextModel::resource`]'s form ([`convert::uri`]).
    pub uri: SharedString,
    /// The whole definition, in `uri`.
    pub range: Range<PointUtf16>,
    /// The part to select and reveal, usually its name.
    pub selection_range: Range<PointUtf16>,
    /// The server's item, sent back as is for its calls.
    server_item: lsp::CallHierarchyItem,
}

impl CallHierarchyItem {
    /// The item as the server sent it.
    pub fn server_item(&self) -> &lsp::CallHierarchyItem {
        &self.server_item
    }
}

/// A call to an item (VS Code `CallHierarchyIncomingCall`).
#[derive(Clone, Debug, PartialEq)]
pub struct IncomingCall {
    /// The caller.
    pub from: CallHierarchyItem,
    /// Where it calls the item, in `from.uri`.
    pub from_ranges: Vec<Range<PointUtf16>>,
}

/// A call an item makes (VS Code `CallHierarchyOutgoingCall`).
#[derive(Clone, Debug, PartialEq)]
pub struct OutgoingCall {
    /// What it calls.
    pub to: CallHierarchyItem,
    /// Where the item calls it, in the item's own document (not `to.uri`).
    pub from_ranges: Vec<Range<PointUtf16>>,
}

fn call_item(item: lsp::CallHierarchyItem) -> CallHierarchyItem {
    CallHierarchyItem {
        name: item.name.clone().into(),
        kind: convert::symbol_kind(item.kind),
        deprecated: item
            .tags
            .as_ref()
            .is_some_and(|tags| tags.contains(&lsp::SymbolTag::DEPRECATED)),
        detail: item.detail.clone().map(Into::into),
        uri: convert::uri(&item.uri),
        range: convert::range(item.range),
        selection_range: convert::range(item.selection_range),
        server_item: item,
    }
}

fn ranges(ranges: Vec<lsp::Range>) -> Vec<Range<PointUtf16>> {
    ranges.into_iter().map(convert::range).collect()
}

impl LspClient {
    /// Whether the server answers `textDocument/prepareCallHierarchy`.
    pub fn supports_call_hierarchy(&self) -> bool {
        matches!(
            self.capabilities().call_hierarchy_provider,
            Some(
                lsp::CallHierarchyServerCapability::Simple(true)
                    | lsp::CallHierarchyServerCapability::Options(_)
            )
        )
    }

    /// The items at `position` in `model` that a call hierarchy starts
    /// from (`textDocument/prepareCallHierarchy`), usually the function
    /// there. Empty if `model` isn't attached, or the server doesn't
    /// support it or has stopped.
    pub fn prepare_call_hierarchy(
        &mut self,
        model: &Entity<TextModel>,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<CallHierarchyItem>>> {
        if !self.supports_call_hierarchy() {
            return Task::ready(Ok(Vec::new()));
        }
        let uri = model.read(cx).resource().to_string();
        let Some((connection, text_document)) = self.prepare_request(&uri, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request = connection.request::<lsp::request::CallHierarchyPrepare>(
            lsp::CallHierarchyPrepareParams {
                text_document_position_params: lsp::TextDocumentPositionParams {
                    text_document,
                    position: convert::position(position),
                },
                work_done_progress_params: Default::default(),
            },
        );
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(call_item)
                .collect())
        })
    }

    /// The calls to `item` (`callHierarchy/incomingCalls`).
    pub fn incoming_calls(
        &self,
        item: &CallHierarchyItem,
        cx: &App,
    ) -> Task<Result<Vec<IncomingCall>>> {
        if !self.is_running() {
            return Task::ready(Ok(Vec::new()));
        }
        let request = self
            .connection()
            .request::<lsp::request::CallHierarchyIncomingCalls>(
                lsp::CallHierarchyIncomingCallsParams {
                    item: item.server_item.clone(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            );
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(|call| IncomingCall {
                    from: call_item(call.from),
                    from_ranges: ranges(call.from_ranges),
                })
                .collect())
        })
    }

    /// The calls `item` makes (`callHierarchy/outgoingCalls`).
    pub fn outgoing_calls(
        &self,
        item: &CallHierarchyItem,
        cx: &App,
    ) -> Task<Result<Vec<OutgoingCall>>> {
        if !self.is_running() {
            return Task::ready(Ok(Vec::new()));
        }
        let request = self
            .connection()
            .request::<lsp::request::CallHierarchyOutgoingCalls>(
                lsp::CallHierarchyOutgoingCallsParams {
                    item: item.server_item.clone(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            );
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(|call| OutgoingCall {
                    to: call_item(call.to),
                    from_ranges: ranges(call.from_ranges),
                })
                .collect())
        })
    }
}
