//! `workspace/didChangeWatchedFiles`. gpui-monaco doesn't watch files, so
//! the host watches what the server asks for ([`LspClient::file_watchers`],
//! usually registered with `client/registerCapability`) and reports the
//! changes ([`LspClient::did_change_watched_files`]), as
//! `vscode-languageclient`'s `FileSystemWatcherFeature` does with VS Code's
//! file watchers.

use gpui::Context;
use lsp_types as lsp;

use crate::LspClient;
use crate::client::LspClientEvent;

pub(crate) const METHOD: &str = "workspace/didChangeWatchedFiles";

impl LspClient {
    /// The file watchers the server registered
    /// (`workspace/didChangeWatchedFiles` registrations), in order. Watch
    /// what they match and report changes with
    /// [`LspClient::did_change_watched_files`];
    /// [`LspClientEvent::FileWatchersChanged`] says when they change.
    pub fn file_watchers(&self) -> Vec<lsp::FileSystemWatcher> {
        self.dynamic
            .iter()
            .filter(|registration| registration.method == METHOD)
            .filter_map(|registration| {
                serde_json::from_value::<lsp::DidChangeWatchedFilesRegistrationOptions>(
                    registration.options.clone(),
                )
                .ok()
            })
            .flat_map(|options| options.watchers)
            .collect()
    }

    /// Tells the server files changed on disk
    /// (`workspace/didChangeWatchedFiles`), such as a `Cargo.toml` or
    /// generated code edited outside the editor. Nothing is sent for no
    /// changes, or once the server stopped.
    pub fn did_change_watched_files(&self, changes: Vec<lsp::FileEvent>) {
        if changes.is_empty() || !self.is_running() {
            return;
        }
        self.connection()
            .notify::<lsp::notification::DidChangeWatchedFiles>(lsp::DidChangeWatchedFilesParams {
                changes,
            })
            .ok();
    }

    /// Emits [`LspClientEvent::FileWatchersChanged`] if the watchers aren't
    /// `before`.
    pub(crate) fn file_watchers_changed(
        &self,
        before: &[lsp::FileSystemWatcher],
        cx: &mut Context<Self>,
    ) {
        if self.file_watchers() != before {
            cx.emit(LspClientEvent::FileWatchersChanged);
        }
    }
}
