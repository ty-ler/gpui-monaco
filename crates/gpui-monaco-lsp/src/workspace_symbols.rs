//! `workspace/symbol` and `workspaceSymbol/resolve`, as
//! `vscode-languageclient`'s `WorkspaceSymbolFeature` sends them for VS
//! Code's "Go to Symbol in Workspace".
//!
//! The bridge registers a gpui-monaco
//! [`WorkspaceSymbolProvider`](gpui_monaco::providers::WorkspaceSymbolProvider)
//! on top of it, for quick access's `#` mode; hosts can also call
//! [`LspClient::workspace_symbols`] for a picker of their own.

use std::ops::Range;

use anyhow::Result;
use gpui::{App, SharedString, Task};
use gpui_monaco::providers::SymbolKind;
use gpui_monaco::text::PointUtf16;
use lsp_types as lsp;

use crate::LspClient;
use crate::convert;
use crate::features::LspFeatures;

/// A symbol from `workspace/symbol`.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSymbol {
    /// Its name.
    pub name: SharedString,
    /// Its kind.
    pub kind: SymbolKind,
    /// Tagged deprecated.
    pub deprecated: bool,
    /// The name of the symbol containing it, such as its class.
    pub container_name: Option<SharedString>,
    /// Its document, in [`TextModel::resource`](gpui_monaco::TextModel::resource)'s
    /// form ([`convert::uri`]).
    pub uri: SharedString,
    /// Where it is in `uri`, or `None` when the server only named the
    /// document: [`LspClient::resolve_workspace_symbol`] asks for it.
    pub range: Option<Range<PointUtf16>>,
    /// The server's symbol, sent back as is to resolve it.
    server_symbol: lsp::WorkspaceSymbol,
}

impl WorkspaceSymbol {
    /// The symbol as the server sent it (a flat `SymbolInformation` comes
    /// back as a `WorkspaceSymbol` with its location).
    pub fn server_symbol(&self) -> &lsp::WorkspaceSymbol {
        &self.server_symbol
    }
}

/// A `workspace/symbol` or `workspaceSymbol/resolve` result.
fn workspace_symbol(symbol: lsp::WorkspaceSymbol) -> WorkspaceSymbol {
    let (uri, range) = match &symbol.location {
        lsp::OneOf::Left(location) => (&location.uri, Some(convert::range(location.range))),
        lsp::OneOf::Right(location) => (&location.uri, None),
    };
    WorkspaceSymbol {
        name: symbol.name.clone().into(),
        kind: convert::symbol_kind(symbol.kind),
        deprecated: symbol
            .tags
            .as_ref()
            .is_some_and(|tags| tags.contains(&lsp::SymbolTag::DEPRECATED)),
        container_name: symbol.container_name.clone().map(Into::into),
        uri: convert::uri(uri),
        range,
        server_symbol: symbol,
    }
}

/// `workspace/symbol` results, flat or not.
fn workspace_symbols(response: lsp::WorkspaceSymbolResponse) -> Vec<WorkspaceSymbol> {
    match response {
        lsp::WorkspaceSymbolResponse::Flat(symbols) => symbols
            .into_iter()
            .map(|symbol| {
                #[allow(deprecated)]
                let deprecated = symbol.deprecated == Some(true);
                let mut tags = symbol.tags.unwrap_or_default();
                if deprecated && !tags.contains(&lsp::SymbolTag::DEPRECATED) {
                    tags.push(lsp::SymbolTag::DEPRECATED);
                }
                workspace_symbol(lsp::WorkspaceSymbol {
                    name: symbol.name,
                    kind: symbol.kind,
                    tags: (!tags.is_empty()).then_some(tags),
                    container_name: symbol.container_name,
                    location: lsp::OneOf::Left(symbol.location),
                    data: None,
                })
            })
            .collect(),
        lsp::WorkspaceSymbolResponse::Nested(symbols) => {
            symbols.into_iter().map(workspace_symbol).collect()
        }
    }
}

impl LspClient {
    /// Whether the server answers `workspace/symbol`.
    pub fn supports_workspace_symbols(&self) -> bool {
        matches!(
            self.capabilities().workspace_symbol_provider,
            Some(lsp::OneOf::Left(true) | lsp::OneOf::Right(_))
        )
    }

    /// The workspace's symbols matching `query` (`workspace/symbol`), for a
    /// "Go to Symbol in Workspace" picker. The server filters them; an
    /// empty query asks for all of them. Empty if the server doesn't
    /// support it or has stopped.
    pub fn workspace_symbols(&self, query: &str, cx: &App) -> Task<Result<Vec<WorkspaceSymbol>>> {
        if !self.is_running() || !self.supports_workspace_symbols() {
            return Task::ready(Ok(Vec::new()));
        }
        let request = self
            .connection()
            .request::<lsp::request::WorkspaceSymbolRequest>(lsp::WorkspaceSymbolParams {
                query: query.to_string(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            });
        cx.background_executor()
            .spawn(async move { Ok(request.await?.map(workspace_symbols).unwrap_or_default()) })
    }

    /// Fills in `symbol` with `workspaceSymbol/resolve`, as VS Code does
    /// before opening a symbol picked from the list: servers may leave out
    /// its range, or other details, until then. `symbol` as it is when the
    /// server doesn't resolve symbols.
    pub fn resolve_workspace_symbol(
        &self,
        symbol: &WorkspaceSymbol,
        cx: &App,
    ) -> Task<Result<WorkspaceSymbol>> {
        let resolves = matches!(
            &self.capabilities().workspace_symbol_provider,
            Some(lsp::OneOf::Right(lsp::WorkspaceSymbolOptions {
                resolve_provider: Some(true),
                ..
            }))
        );
        if !self.is_running() || !resolves {
            return Task::ready(Ok(symbol.clone()));
        }
        let request = self
            .connection()
            .request::<lsp::request::WorkspaceSymbolResolve>(symbol.server_symbol.clone());
        cx.background_executor()
            .spawn(async move { Ok(workspace_symbol(request.await?)) })
    }
}

/// The editor's form of `symbol`, with the server's symbol as its `data`
/// for resolving.
fn provider_symbol(symbol: WorkspaceSymbol) -> gpui_monaco::providers::WorkspaceSymbol {
    gpui_monaco::providers::WorkspaceSymbol {
        data: serde_json::to_value(&symbol.server_symbol).ok(),
        name: symbol.name,
        kind: symbol.kind,
        deprecated: symbol.deprecated,
        container_name: symbol.container_name,
        uri: symbol.uri,
        range: symbol.range,
    }
}

/// `vscode-languageclient`'s `WorkspaceSymbolFeature`: quick access's `#`
/// mode asks the server.
impl gpui_monaco::providers::WorkspaceSymbolProvider for LspFeatures {
    fn provide_workspace_symbols(
        &self,
        query: &str,
        cx: &mut App,
    ) -> Task<Result<Vec<gpui_monaco::providers::WorkspaceSymbol>>> {
        let Some(client) = self.client().upgrade() else {
            return Task::ready(Ok(Vec::new()));
        };
        let request = client.read(cx).workspace_symbols(query, cx);
        cx.background_executor()
            .spawn(async move { Ok(request.await?.into_iter().map(provider_symbol).collect()) })
    }

    fn resolve_workspace_symbol(
        &self,
        symbol: &gpui_monaco::providers::WorkspaceSymbol,
        cx: &mut App,
    ) -> Option<Task<Result<gpui_monaco::providers::WorkspaceSymbol>>> {
        let server_symbol: lsp::WorkspaceSymbol =
            serde_json::from_value(symbol.data.clone()?).ok()?;
        let client = self.client().upgrade()?;
        let request = client
            .read(cx)
            .resolve_workspace_symbol(&workspace_symbol(server_symbol), cx);
        Some(
            cx.background_executor()
                .spawn(async move { Ok(provider_symbol(request.await?)) }),
        )
    }
}
