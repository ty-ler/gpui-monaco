//! Connects [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
//! servers to gpui-monaco, the way `monaco-languageclient` does for Monaco.
//!
//! An [`LspClient`] starts a server (or connects to one), registers
//! gpui-monaco providers for the features it supports (completion, hover,
//! go to definition, code actions, code lenses, links, colors, linked
//! editing and more; the book's LSP chapter has the full list) and turns its
//! diagnostics into markers. Attach each model the server should know
//! about; edits are synced as they happen.
//!
//! ```ignore
//! let client = LspClient::start(
//!     LanguageServerBinary { path: "rust-analyzer".into(), ..Default::default() },
//!     LspClientOptions {
//!         name: "rust-analyzer".into(),
//!         languages: vec!["rust".into()],
//!         root_uri: Some("file:///path/to/project".into()),
//!         ..Default::default()
//!     },
//!     cx,
//! );
//! cx.spawn(async move |cx| {
//!     let client = client.await?;
//!     client.update(cx, |client, cx| client.attach(&model, cx))??;
//!     anyhow::Ok(())
//! })
//! .detach();
//! ```

mod client;
pub mod convert;
mod features;
pub mod rpc;
mod workspace_symbols;

#[cfg(test)]
mod tests;

pub use client::{
    LanguageServerBinary, LspClient, LspClientEvent, LspClientOptions, MessageResponder,
};
pub use lsp_types;
pub use rpc::Connection;
pub use workspace_symbols::WorkspaceSymbol;
