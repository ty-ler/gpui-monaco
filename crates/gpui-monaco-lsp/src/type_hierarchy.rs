//! `textDocument/prepareTypeHierarchy`, `typeHierarchy/supertypes` and
//! `typeHierarchy/subtypes`, as `vscode-languageclient`'s
//! `TypeHierarchyFeature` sends them for VS Code's type hierarchy view.
//!
//! gpui-monaco has no type hierarchy provider or view (Monaco has neither),
//! so this is an API on [`LspClient`] for hosts to build that UI on.
//!
//! `lsp-types` 0.97 has no `typeHierarchyProvider` server capability and
//! types an item's `tags` as one tag instead of a list, so the requests are
//! sent raw and the server's items are kept as JSON.

use std::ops::Range;

use anyhow::{Context as _, Result};
use gpui::{App, Context, Entity, SharedString, Task};
use gpui_monaco::TextModel;
use gpui_monaco::providers::SymbolKind;
use gpui_monaco::text::PointUtf16;
use lsp_types as lsp;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::LspClient;
use crate::convert;

const PREPARE: &str = "textDocument/prepareTypeHierarchy";
const SUPERTYPES: &str = "typeHierarchy/supertypes";
const SUBTYPES: &str = "typeHierarchy/subtypes";

/// A class, interface or other type in a type hierarchy (VS Code
/// `TypeHierarchyItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct TypeHierarchyItem {
    /// Its name.
    pub name: SharedString,
    /// Its kind.
    pub kind: SymbolKind,
    /// Tagged deprecated.
    pub deprecated: bool,
    /// More detail, such as its module.
    pub detail: Option<SharedString>,
    /// Its document, in [`TextModel::resource`]'s form ([`convert::uri`]).
    pub uri: SharedString,
    /// The whole definition, in `uri`.
    pub range: Range<PointUtf16>,
    /// The part to select and reveal, usually its name.
    pub selection_range: Range<PointUtf16>,
    /// The server's item, sent back as is for its supertypes and subtypes.
    server_item: Value,
}

impl TypeHierarchyItem {
    /// The item as the server sent it, as JSON (`lsp_types`'s
    /// `TypeHierarchyItem` can't read a list of `tags`).
    pub fn server_item(&self) -> &Value {
        &self.server_item
    }
}

/// The fields of a server's item the editor uses.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerItem {
    name: String,
    kind: lsp::SymbolKind,
    #[serde(default)]
    tags: Option<Vec<lsp::SymbolTag>>,
    #[serde(default)]
    detail: Option<String>,
    uri: lsp::Uri,
    range: lsp::Range,
    selection_range: lsp::Range,
}

fn type_item(server_item: Value) -> Result<TypeHierarchyItem> {
    let item: ServerItem =
        serde_json::from_value(server_item.clone()).context("invalid type hierarchy item")?;
    Ok(TypeHierarchyItem {
        name: item.name.into(),
        kind: convert::symbol_kind(item.kind),
        deprecated: item
            .tags
            .is_some_and(|tags| tags.contains(&lsp::SymbolTag::DEPRECATED)),
        detail: item.detail.map(Into::into),
        uri: convert::uri(&item.uri),
        range: convert::range(item.range),
        selection_range: convert::range(item.selection_range),
        server_item,
    })
}

/// A `TypeHierarchyItem[] | null` result.
fn type_items(method: &str, result: Value) -> Result<Vec<TypeHierarchyItem>> {
    match result {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items.into_iter().map(type_item).collect(),
        _ => Err(anyhow::anyhow!("invalid {method} result")),
    }
}

impl LspClient {
    /// Whether the server answers `textDocument/prepareTypeHierarchy`.
    pub fn supports_type_hierarchy(&self) -> bool {
        match self.raw_capabilities().get("typeHierarchyProvider") {
            Some(Value::Bool(supported)) => *supported,
            Some(Value::Object(_)) => true,
            _ => false,
        }
    }

    /// The items at `position` in `model` that a type hierarchy starts
    /// from (`textDocument/prepareTypeHierarchy`), usually the type there.
    /// Empty if `model` isn't attached, or the server doesn't support it or
    /// has stopped.
    pub fn prepare_type_hierarchy(
        &mut self,
        model: &Entity<TextModel>,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<TypeHierarchyItem>>> {
        if !self.supports_type_hierarchy() {
            return Task::ready(Ok(Vec::new()));
        }
        let uri = model.read(cx).resource().to_string();
        let Some((connection, text_document)) = self.prepare_request(&uri, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request = connection.request_raw(
            PREPARE,
            json!({
                "textDocument": text_document,
                "position": convert::position(position),
            }),
        );
        cx.background_executor()
            .spawn(async move { type_items(PREPARE, request.await?) })
    }

    /// The types `item` extends or implements (`typeHierarchy/supertypes`).
    pub fn supertypes(
        &self,
        item: &TypeHierarchyItem,
        cx: &App,
    ) -> Task<Result<Vec<TypeHierarchyItem>>> {
        self.type_hierarchy_request(SUPERTYPES, item, cx)
    }

    /// The types that extend or implement `item` (`typeHierarchy/subtypes`).
    pub fn subtypes(
        &self,
        item: &TypeHierarchyItem,
        cx: &App,
    ) -> Task<Result<Vec<TypeHierarchyItem>>> {
        self.type_hierarchy_request(SUBTYPES, item, cx)
    }

    fn type_hierarchy_request(
        &self,
        method: &'static str,
        item: &TypeHierarchyItem,
        cx: &App,
    ) -> Task<Result<Vec<TypeHierarchyItem>>> {
        if !self.is_running() {
            return Task::ready(Ok(Vec::new()));
        }
        let request = self
            .connection()
            .request_raw(method, json!({ "item": item.server_item }));
        cx.background_executor()
            .spawn(async move { type_items(method, request.await?) })
    }
}
