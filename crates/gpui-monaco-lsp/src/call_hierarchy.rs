//! `textDocument/prepareCallHierarchy`, `callHierarchy/incomingCalls` and
//! `callHierarchy/outgoingCalls`, as `vscode-languageclient`'s
//! `CallHierarchyFeature` sends them for VS Code's call hierarchy view.
//!
//! The bridge registers a gpui-monaco
//! [`CallHierarchyProvider`](gpui_monaco::providers::CallHierarchyProvider)
//! on top of it, for Peek Call Hierarchy; hosts can also call these for a
//! view of their own.

use std::ops::Range;

use anyhow::Result;
use gpui::{App, Context, Entity, SharedString, Task};
use gpui_monaco::TextModel;
use gpui_monaco::providers::SymbolKind;
use gpui_monaco::text::PointUtf16;
use lsp_types as lsp;

use crate::LspClient;
use crate::convert;
use crate::features::LspFeatures;

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
        let uri = model.read(cx).resource().to_string();
        self.prepare_call_hierarchy_at(&uri, position, cx)
    }

    /// [`Self::prepare_call_hierarchy`] for the attached document `uri`.
    pub(crate) fn prepare_call_hierarchy_at(
        &mut self,
        uri: &str,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<CallHierarchyItem>>> {
        if !self.supports_call_hierarchy() {
            return Task::ready(Ok(Vec::new()));
        }
        let Some((connection, text_document)) = self.prepare_request(uri, cx) else {
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

/// The editor's form of `item`, with the server's item as its `data`.
fn provider_item(item: CallHierarchyItem) -> gpui_monaco::providers::HierarchyItem {
    gpui_monaco::providers::HierarchyItem {
        data: serde_json::to_value(&item.server_item).ok(),
        name: item.name,
        kind: item.kind,
        deprecated: item.deprecated,
        detail: item.detail,
        uri: Some(item.uri),
        range: item.range,
        selection_range: item.selection_range,
    }
}

/// The server's item back from the editor's.
fn server_item(item: &gpui_monaco::providers::HierarchyItem) -> Result<CallHierarchyItem> {
    let data = item
        .data
        .clone()
        .ok_or_else(|| anyhow::anyhow!("a call hierarchy item without the server's item"))?;
    Ok(call_item(serde_json::from_value(data)?))
}

/// `vscode-languageclient`'s `CallHierarchyFeature`: Peek Call Hierarchy
/// asks the server.
impl gpui_monaco::providers::CallHierarchyProvider for LspFeatures {
    fn prepare_call_hierarchy(
        &self,
        context: &gpui_monaco::providers::ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<Result<Vec<gpui_monaco::providers::HierarchyItem>>> {
        let Some(client) = self.client().upgrade() else {
            return Task::ready(Ok(Vec::new()));
        };
        let uri = context.resource.to_string();
        let request = client.update(cx, |client, cx| {
            client.prepare_call_hierarchy_at(&uri, position, cx)
        });
        cx.background_executor()
            .spawn(async move { Ok(request.await?.into_iter().map(provider_item).collect()) })
    }

    fn provide_incoming_calls(
        &self,
        item: &gpui_monaco::providers::HierarchyItem,
        cx: &mut App,
    ) -> Task<Result<Vec<gpui_monaco::providers::IncomingCall>>> {
        let (Some(client), item) = (self.client().upgrade(), server_item(item)) else {
            return Task::ready(Ok(Vec::new()));
        };
        let item = match item {
            Ok(item) => item,
            Err(err) => return Task::ready(Err(err)),
        };
        let request = client.read(cx).incoming_calls(&item, cx);
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .into_iter()
                .map(|call| gpui_monaco::providers::IncomingCall {
                    from: provider_item(call.from),
                    from_ranges: call.from_ranges,
                })
                .collect())
        })
    }

    fn provide_outgoing_calls(
        &self,
        item: &gpui_monaco::providers::HierarchyItem,
        cx: &mut App,
    ) -> Task<Result<Vec<gpui_monaco::providers::OutgoingCall>>> {
        let (Some(client), item) = (self.client().upgrade(), server_item(item)) else {
            return Task::ready(Ok(Vec::new()));
        };
        let item = match item {
            Ok(item) => item,
            Err(err) => return Task::ready(Err(err)),
        };
        let request = client.read(cx).outgoing_calls(&item, cx);
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .into_iter()
                .map(|call| gpui_monaco::providers::OutgoingCall {
                    to: provider_item(call.to),
                    from_ranges: call.from_ranges,
                })
                .collect())
        })
    }
}
