//! The client side of one language server: lifecycle, document sync,
//! diagnostics and the requests a server sends the client.

mod pull_diagnostics;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use futures_lite::StreamExt;
use futures_lite::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, SharedString, Subscription, Task,
    WeakEntity,
};
use gpui_monaco::events::content_changes;
use gpui_monaco::providers::{LanguageSelector, ProviderRegistration, unregister_provider};
use gpui_monaco::text::BufferSnapshot;
use gpui_monaco::{Marker, TextModel, TextModelEvent};
use lsp_types as lsp;
use serde_json::{Value, json};

use crate::convert;
use crate::features::LspFeatures;
use crate::registration::{self, DynamicRegistration};
use crate::rpc::{self, Connection, Incoming, METHOD_NOT_FOUND, ResponseError};
use pull_diagnostics::PullDiagnostics;

/// How long [`LspClient::will_save_wait_until`] waits for the server's
/// edits: VS Code's budget for a save participant.
pub const WILL_SAVE_WAIT_UNTIL_TIMEOUT: Duration = Duration::from_millis(1500);

/// JSON-RPC's invalid params error code.
const INVALID_PARAMS: i64 = -32602;

/// How to run a language server.
#[derive(Clone, Debug, Default)]
pub struct LanguageServerBinary {
    /// The executable.
    pub path: PathBuf,
    /// Its arguments (e.g. `["--stdio"]`).
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
    /// The working directory; the current one if `None`.
    pub cwd: Option<PathBuf>,
}

/// How the client talks to its server.
#[derive(Clone, Debug, Default)]
pub struct LspClientOptions {
    /// Names the server in logs and owns its diagnostics markers
    /// (`lsp:<name>`).
    pub name: SharedString,
    /// The language ids the server's features are registered for; empty
    /// registers them for every language (they only answer for attached
    /// models either way).
    pub languages: Vec<SharedString>,
    /// The workspace root, as a uri (e.g. `file:///home/me/project`).
    pub root_uri: Option<String>,
    /// `initializationOptions` for the server.
    pub initialization_options: Option<Value>,
    /// Settings the server can read with `workspace/configuration`, as
    /// nested objects (`{ "rust-analyzer": { "checkOnSave": true } }`).
    pub settings: Value,
}

/// Events from an [`LspClient`].
#[derive(Clone, Debug)]
pub enum LspClientEvent {
    /// `window/showMessage`: something the host may show the user.
    ShowMessage {
        /// Error, warning, info or log.
        kind: lsp::MessageType,
        /// The message.
        message: String,
    },
    /// `window/showMessageRequest`: a message with actions the user can
    /// pick from. The server waits for [`MessageResponder::respond`]; if
    /// every clone of `respond` is dropped first, it's told the prompt was
    /// dismissed (`null`).
    ShowMessageRequest {
        /// Error, warning, info or log.
        kind: lsp::MessageType,
        /// The message.
        message: String,
        /// The actions to offer, with any extra properties the server
        /// gave them.
        actions: Vec<lsp::MessageActionItem>,
        /// Answers the server.
        respond: MessageResponder,
    },
    /// `workspace/inlineValue/refresh`: the server's inline values
    /// changed. Editors showing inline values ask again on their own; a
    /// host showing them itself asks again ([`LspClient::inline_values`]).
    InlineValuesRefresh,
    /// The server registered or unregistered file watchers: watch what
    /// [`LspClient::file_watchers`] lists now.
    FileWatchersChanged,
    /// The server stopped; its features are unregistered.
    Exited,
}

/// Answers a `window/showMessageRequest` with the action the user picked.
/// Cheap to clone; the first answer wins, and dropping every clone
/// unanswered answers `null`, as dismissing the prompt does.
#[derive(Clone)]
pub struct MessageResponder(Arc<Responder>);

struct Responder {
    connection: Connection,
    id: Value,
    answered: AtomicBool,
}

impl MessageResponder {
    /// Answers with `action`, one of the request's actions, or `None` if
    /// the user dismissed the prompt. Later answers are ignored.
    pub fn respond(&self, action: Option<lsp::MessageActionItem>) {
        self.0.respond(action);
    }
}

impl Responder {
    fn respond(&self, action: Option<lsp::MessageActionItem>) {
        if self.answered.swap(true, Ordering::AcqRel) {
            return;
        }
        let result = serde_json::to_value(action).unwrap_or(Value::Null);
        self.connection.respond(self.id.clone(), Ok(result)).ok();
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.respond(None);
    }
}

impl std::fmt::Debug for MessageResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageResponder")
            .field("id", &self.0.id)
            .finish_non_exhaustive()
    }
}

struct Document {
    model: WeakEntity<TextModel>,
    /// The text the server has, at `version`.
    snapshot: BufferSnapshot,
    version: i32,
    _subscriptions: Vec<Subscription>,
}

/// A running language server bridged to gpui-monaco's providers (one for
/// each feature the server supports, such as completion, hover, definition,
/// code actions and rename) and markers (diagnostics). Attach models with [`LspClient::attach`].
pub struct LspClient {
    pub(crate) options: LspClientOptions,
    connection: Connection,
    /// The `initialize` result's capabilities with the dynamic
    /// registrations' on top.
    pub(crate) capabilities: lsp::ServerCapabilities,
    /// The `initialize` result's capabilities, as the server sent them.
    pub(crate) static_capabilities: Value,
    /// The server's `client/registerCapability` registrations, in order.
    pub(crate) dynamic: Vec<DynamicRegistration>,
    /// The capabilities as the server sent them, with any `lsp-types`
    /// doesn't know (such as `typeHierarchyProvider`).
    pub(crate) raw_capabilities: Value,
    server_info: Option<lsp::ServerInfo>,
    /// The attached documents, by [`TextModel::resource`].
    documents: HashMap<String, Document>,
    /// The diagnostics last published for each uri (normalized), with
    /// their markers: code action requests send the server's own back
    /// (`vscode-languageclient` keeps them in its diagnostic collection).
    diagnostics: HashMap<String, Vec<(Marker, lsp::Diagnostic)>>,
    /// Pull diagnostics, if the server has a `diagnosticProvider`.
    pull: Option<PullDiagnostics>,
    registrations: Vec<ProviderRegistration>,
    /// The server's commands, registered with [`gpui_monaco::register_command`].
    commands: Vec<gpui_monaco::Disposable>,
    running: bool,
    _tasks: Vec<Task<()>>,
    _child: Option<async_process::Child>,
}

impl EventEmitter<LspClientEvent> for LspClient {}

impl LspClient {
    /// Starts `binary` and initializes it. The process is killed when the
    /// client is dropped.
    pub fn start(
        binary: LanguageServerBinary,
        options: LspClientOptions,
        cx: &mut App,
    ) -> Task<Result<Entity<Self>>> {
        let mut command = async_process::Command::new(&binary.path);
        command
            .args(&binary.args)
            .envs(binary.env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = &binary.cwd {
            command.current_dir(cwd);
        }
        let mut child = match command
            .spawn()
            .with_context(|| format!("failed to start {}", binary.path.display()))
        {
            Ok(child) => child,
            Err(err) => return Task::ready(Err(err)),
        };
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Task::ready(Err(anyhow::anyhow!("no stdio for the language server")));
        };
        let name = options.name.clone();
        let log_stderr = cx.background_executor().spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Some(Ok(line)) = lines.next().await {
                log::debug!("{name}: {line}");
            }
        });
        Self::connect(stdout, stdin, options, Some(child), vec![log_stderr], cx)
    }

    /// Initializes a server reached through `reader` (its output) and
    /// `writer` (its input), e.g. a socket or an in-process server.
    pub fn from_transport<R, W>(
        reader: R,
        writer: W,
        options: LspClientOptions,
        cx: &mut App,
    ) -> Task<Result<Entity<Self>>>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::connect(reader, writer, options, None, Vec::new(), cx)
    }

    fn connect<R, W>(
        reader: R,
        writer: W,
        options: LspClientOptions,
        child: Option<async_process::Child>,
        mut tasks: Vec<Task<()>>,
        cx: &mut App,
    ) -> Task<Result<Entity<Self>>>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (connection, incoming, io) = Connection::new(reader, writer, cx.background_executor());
        tasks.extend(io);
        // Sent raw so the capabilities `lsp-types` doesn't know are kept.
        let initialize = initialize_params(&options).map(|params| {
            connection.request_raw(
                <lsp::request::Initialize as lsp::request::Request>::METHOD,
                params,
            )
        });
        cx.spawn(async move |cx| {
            let raw = initialize?.await?;
            let raw_capabilities = raw.get("capabilities").cloned().unwrap_or(Value::Null);
            let result: lsp::InitializeResult =
                serde_json::from_value(raw).context("invalid initialize result")?;
            connection.notify::<lsp::notification::Initialized>(lsp::InitializedParams {})?;
            Ok(cx.new(|cx: &mut Context<Self>| {
                tasks.push(cx.spawn(async move |this, cx| {
                    while let Ok(message) = incoming.recv().await {
                        if this
                            .update(cx, |this, cx| this.handle_incoming(message, cx))
                            .is_err()
                        {
                            return;
                        }
                    }
                    this.update(cx, |this, cx| this.exited(cx)).ok();
                }));
                cx.on_release(|this, cx| this.unregister(cx)).detach();
                let pull = PullDiagnostics::new(&result.capabilities);
                let mut client = Self {
                    options,
                    connection,
                    static_capabilities: raw_capabilities.clone(),
                    capabilities: result.capabilities,
                    raw_capabilities,
                    dynamic: Vec::new(),
                    server_info: result.server_info,
                    documents: HashMap::new(),
                    diagnostics: HashMap::new(),
                    pull,
                    registrations: Vec::new(),
                    commands: Vec::new(),
                    running: true,
                    _tasks: tasks,
                    _child: child,
                };
                client.register(cx);
                client.pull_workspace_diagnostics(None, cx);
                client
            }))
        })
    }

    /// What the server said it can do: its `initialize` result, with the
    /// capabilities it registered since (`client/registerCapability`).
    pub fn capabilities(&self) -> &lsp::ServerCapabilities {
        &self.capabilities
    }

    /// The capabilities as the server sent them, and registered since,
    /// including any that [`LspClient::capabilities`] leaves out because
    /// `lsp-types` doesn't have them.
    pub fn raw_capabilities(&self) -> &Value {
        &self.raw_capabilities
    }

    /// The server's name and version, if it gave them.
    pub fn server_info(&self) -> Option<&lsp::ServerInfo> {
        self.server_info.as_ref()
    }

    /// The connection, for requests the bridge doesn't cover.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// The settings served through `workspace/configuration`.
    pub fn settings(&self) -> &Value {
        &self.options.settings
    }

    /// Replaces the settings served through `workspace/configuration` and
    /// tells the server (`workspace/didChangeConfiguration`). As in
    /// `vscode-languageclient`, the notification's `settings` is `null`:
    /// servers that pull their settings ask for them again.
    pub fn set_settings(&mut self, settings: Value) {
        self.options.settings = settings;
        if !self.running {
            return;
        }
        self.connection
            .notify::<lsp::notification::DidChangeConfiguration>(
                lsp::DidChangeConfigurationParams {
                    settings: Value::Null,
                },
            )
            .ok();
    }

    /// Whether the server is still running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// The markers owner for this server's diagnostics.
    pub fn markers_owner(&self) -> SharedString {
        format!("lsp:{}", self.options.name).into()
    }

    /// Opens `model` on the server (`textDocument/didOpen`) and keeps it in
    /// sync until it's released or [`LspClient::detach`]ed. The document is
    /// the model's [`TextModel::resource`] (`monaco-languageclient` sends
    /// `model.uri.toString()`): its uri normalized (`file:///a b.rs` is sent
    /// as `file:///a%20b.rs`), or `inmemory://model/N` for a model without
    /// one. The model is [registered](gpui_monaco::register_model), as
    /// Monaco's model service has every model, and its language id (or
    /// `plaintext`) is sent as the document's language.
    pub fn attach(&mut self, model: &Entity<TextModel>, cx: &mut Context<Self>) -> Result<()> {
        let uri = model.read(cx).resource().to_string();
        if self.documents.contains_key(&uri) {
            return Ok(());
        }
        // Monaco's model service has every model: `workspace/applyEdit`
        // and the server's locations find this one by its uri.
        gpui_monaco::register_model(cx, model);
        let model_ref = model.read(cx);
        let language_id = model_ref
            .language_id()
            .map_or_else(|| "plaintext".to_string(), |id| id.to_string());
        let snapshot = model_ref.snapshot();
        self.open(&uri, &language_id, &snapshot, 0)?;
        let subscriptions = vec![
            cx.subscribe(model, {
                let uri = uri.clone();
                move |this, model, event: &TextModelEvent, cx| match event {
                    TextModelEvent::Edited { .. } => {
                        this.sync(&uri, cx);
                        this.pull_diagnostics_after_edit(&uri, cx);
                    }
                    TextModelEvent::LanguageChanged { .. } => this.reopen(&uri, &model, cx),
                    _ => {}
                }
            }),
            cx.observe_release(model, {
                let uri = uri.clone();
                move |this, _, _| this.close(&uri)
            }),
        ];
        self.documents.insert(
            uri.clone(),
            Document {
                model: model.downgrade(),
                snapshot,
                version: 0,
                _subscriptions: subscriptions,
            },
        );
        self.pull_diagnostics(&uri, cx);
        Ok(())
    }

    /// Closes `model` on the server (`textDocument/didClose`) and clears
    /// its diagnostics.
    pub fn detach(&mut self, model: &Entity<TextModel>, cx: &mut Context<Self>) {
        let uri = model.read(cx).resource().to_string();
        if self.documents.contains_key(&uri) {
            self.close(&uri);
            self.diagnostics.remove(&uri);
            let owner = self.markers_owner();
            model.update(cx, |model, cx| model.set_markers(owner, Vec::new(), cx));
        }
    }

    /// Whether `model` is attached.
    pub fn is_attached(&self, model: &Entity<TextModel>, cx: &App) -> bool {
        self.documents
            .contains_key(model.read(cx).resource().as_ref())
    }

    /// Tells the server `model` was saved (`textDocument/didSave`), if it
    /// wants to know.
    pub fn did_save(&mut self, model: &Entity<TextModel>, cx: &mut Context<Self>) {
        let uri = model.read(cx).resource().to_string();
        self.sync(&uri, cx);
        let save = match &self.capabilities.text_document_sync {
            Some(lsp::TextDocumentSyncCapability::Options(options)) => options.save.clone(),
            _ => None,
        };
        let include_text = match save {
            None => return,
            Some(lsp::TextDocumentSyncSaveOptions::Supported(supported)) => {
                if !supported {
                    return;
                }
                false
            }
            Some(lsp::TextDocumentSyncSaveOptions::SaveOptions(options)) => {
                options.include_text.unwrap_or(false)
            }
        };
        let Some(document) = self.documents.get(&uri) else {
            return;
        };
        let Ok(lsp_uri) = lsp::Uri::from_str(&uri) else {
            return;
        };
        self.connection
            .notify::<lsp::notification::DidSaveTextDocument>(lsp::DidSaveTextDocumentParams {
                text_document: lsp::TextDocumentIdentifier { uri: lsp_uri },
                text: include_text.then(|| document.snapshot.rope().to_string()),
            })
            .ok();
    }

    /// Tells the server `model` is about to be saved
    /// (`textDocument/willSave`), if it wants to know.
    pub fn will_save(
        &mut self,
        model: &Entity<TextModel>,
        reason: lsp::TextDocumentSaveReason,
        cx: &mut Context<Self>,
    ) {
        let uri = model.read(cx).resource().to_string();
        if !self
            .sync_options()
            .is_some_and(|o| o.will_save == Some(true))
        {
            return;
        }
        let Some((connection, text_document)) = self.prepare_request(&uri, cx) else {
            return;
        };
        connection
            .notify::<lsp::notification::WillSaveTextDocument>(lsp::WillSaveTextDocumentParams {
                text_document,
                reason,
            })
            .ok();
    }

    /// Asks the server for the edits to make before `model` is saved
    /// (`textDocument/willSaveWaitUntil`), such as organizing imports, if it
    /// supports it, and applies them to the model as one undo step. The
    /// task resolves when they're applied, or right away if the server
    /// doesn't support it. Save when it does: a server that fails or takes
    /// longer than [`WILL_SAVE_WAIT_UNTIL_TIMEOUT`] doesn't hold up the
    /// save, and its edits are dropped, as VS Code's save participants are.
    /// So are edits that come after the document changed.
    pub fn will_save_wait_until(
        &mut self,
        model: &Entity<TextModel>,
        reason: lsp::TextDocumentSaveReason,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let uri = model.read(cx).resource().to_string();
        if !self
            .sync_options()
            .is_some_and(|o| o.will_save_wait_until == Some(true))
        {
            return Task::ready(());
        }
        let Some((connection, text_document)) = self.prepare_request(&uri, cx) else {
            return Task::ready(());
        };
        let Some(version) = self.documents.get(&uri).map(|d| d.version) else {
            return Task::ready(());
        };
        let request = connection.request::<lsp::request::WillSaveWaitUntil>(
            lsp::WillSaveTextDocumentParams {
                text_document: text_document.clone(),
                reason,
            },
        );
        let timeout = cx.background_executor().timer(WILL_SAVE_WAIT_UNTIL_TIMEOUT);
        cx.spawn(async move |this, cx| {
            let response = futures_lite::future::or(async { Some(request.await) }, async {
                timeout.await;
                None
            })
            .await;
            let edits = match response {
                Some(Ok(edits)) => edits.unwrap_or_default(),
                Some(Err(err)) => {
                    log::warn!("{err:#}");
                    return;
                }
                None => {
                    log::warn!("textDocument/willSaveWaitUntil timed out");
                    return;
                }
            };
            if edits.is_empty() {
                return;
            }
            // A versioned edit isn't applied if the document changed since.
            let edit = lsp::WorkspaceEdit {
                document_changes: Some(lsp::DocumentChanges::Edits(vec![lsp::TextDocumentEdit {
                    text_document: lsp::OptionalVersionedTextDocumentIdentifier {
                        uri: text_document.uri,
                        version: Some(version),
                    },
                    edits: edits.into_iter().map(lsp::OneOf::Left).collect(),
                }])),
                ..Default::default()
            };
            let Ok(apply) = this.update(cx, |this, cx| this.apply_workspace_edit(edit, cx)) else {
                return;
            };
            let response = apply.await;
            if !response.applied {
                log::warn!(
                    "willSaveWaitUntil edits not applied: {}",
                    response.failure_reason.unwrap_or_default()
                );
            }
        })
    }

    /// Asks the server to shut down (`shutdown`, then `exit`). Its
    /// features are unregistered right away.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        self.unregister(cx);
        // Stops pulling diagnostics.
        self.pull = None;
        let connection = self.connection.clone();
        let shutdown = connection.request::<lsp::request::Shutdown>(());
        cx.background_executor().spawn(async move {
            shutdown.await?;
            connection.notify::<lsp::notification::Exit>(())?;
            Ok(())
        })
    }

    /// Brings the server's copy of `uri` (a [`TextModel::resource`]) up to
    /// date and returns what a request about it needs, or `None` if it
    /// isn't attached.
    pub(crate) fn prepare_request(
        &mut self,
        uri: &str,
        cx: &mut Context<Self>,
    ) -> Option<(Connection, lsp::TextDocumentIdentifier)> {
        if !self.running || !self.documents.contains_key(uri) {
            return None;
        }
        self.sync(uri, cx);
        let uri = lsp::Uri::from_str(uri).ok()?;
        Some((self.connection.clone(), lsp::TextDocumentIdentifier { uri }))
    }

    /// The diagnostics a code action request sends for `markers` of `uri`
    /// (a [`TextModel::resource`]): the server's own as it published them
    /// (`vscode-languageclient` sends its collection's, numeric codes and
    /// related information included), else [`convert::diagnostic`]'s.
    /// Markers move with edits, so one whose range changed still finds its
    /// diagnostic by the rest.
    pub(crate) fn lsp_diagnostics(&self, uri: &str, markers: &[Marker]) -> Vec<lsp::Diagnostic> {
        let published = self.diagnostics.get(uri).map_or(&[][..], Vec::as_slice);
        let mut used = vec![false; published.len()];
        markers
            .iter()
            .map(|marker| {
                let unused = |ix: &usize| !used[*ix];
                let found = (0..published.len())
                    .filter(unused)
                    .find(|&ix| published[ix].0 == *marker)
                    .or_else(|| {
                        (0..published.len()).filter(unused).find(|&ix| {
                            let candidate = &published[ix].0;
                            Marker {
                                range: marker.range.clone(),
                                ..candidate.clone()
                            } == *marker
                        })
                    });
                match found {
                    Some(ix) => {
                        used[ix] = true;
                        published[ix].1.clone()
                    }
                    None => convert::diagnostic(marker),
                }
            })
            .collect()
    }

    fn sync_options(&self) -> Option<&lsp::TextDocumentSyncOptions> {
        match &self.capabilities.text_document_sync {
            Some(lsp::TextDocumentSyncCapability::Options(options)) => Some(options),
            _ => None,
        }
    }

    fn sync_kind(&self) -> lsp::TextDocumentSyncKind {
        match &self.capabilities.text_document_sync {
            Some(lsp::TextDocumentSyncCapability::Kind(kind)) => *kind,
            Some(lsp::TextDocumentSyncCapability::Options(options)) => {
                options.change.unwrap_or(lsp::TextDocumentSyncKind::NONE)
            }
            None => lsp::TextDocumentSyncKind::NONE,
        }
    }

    fn open(
        &self,
        uri: &str,
        language_id: &str,
        snapshot: &BufferSnapshot,
        version: i32,
    ) -> Result<()> {
        self.connection
            .notify::<lsp::notification::DidOpenTextDocument>(lsp::DidOpenTextDocumentParams {
                text_document: lsp::TextDocumentItem {
                    uri: lsp::Uri::from_str(uri).with_context(|| format!("invalid uri {uri}"))?,
                    language_id: language_id.to_string(),
                    version,
                    text: snapshot.rope().to_string(),
                },
            })
    }

    fn close(&mut self, uri: &str) {
        if let Some(pull) = &mut self.pull {
            pull.forget(uri);
        }
        if self.documents.remove(uri).is_some()
            && let Ok(uri) = lsp::Uri::from_str(uri)
        {
            self.connection
                .notify::<lsp::notification::DidCloseTextDocument>(
                    lsp::DidCloseTextDocumentParams {
                        text_document: lsp::TextDocumentIdentifier { uri },
                    },
                )
                .ok();
        }
    }

    fn reopen(&mut self, uri: &str, model: &Entity<TextModel>, cx: &mut Context<Self>) {
        self.close(uri);
        self.attach(model, cx).log_err();
    }

    /// Sends the edits made since the server's copy (`didChange`).
    fn sync(&mut self, uri: &str, cx: &mut Context<Self>) {
        let kind = self.sync_kind();
        let Some(document) = self.documents.get_mut(uri) else {
            return;
        };
        let Some(model) = document.model.upgrade() else {
            return;
        };
        let model = model.read(cx);
        let current = model.snapshot();
        if current.version() == document.snapshot.version() {
            return;
        }
        let patch = model.buffer().edits_since(document.snapshot.version());
        // A line ending change is a new version with the same text: the
        // server's copy has `\n` line breaks either way.
        if patch.is_empty() {
            document.snapshot = current;
            return;
        }
        let changes = if kind == lsp::TextDocumentSyncKind::INCREMENTAL {
            let eol = document.snapshot.line_ending();
            content_changes(document.snapshot.rope(), current.rope(), &patch, eol)
                .into_iter()
                .map(|change| lsp::TextDocumentContentChangeEvent {
                    range: Some(convert::lsp_range(&change.range)),
                    range_length: None,
                    text: change.text,
                })
                .collect()
        } else {
            vec![lsp::TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: current.rope().to_string(),
            }]
        };
        document.snapshot = current;
        if kind == lsp::TextDocumentSyncKind::NONE {
            return;
        }
        document.version += 1;
        let Ok(uri) = lsp::Uri::from_str(uri) else {
            return;
        };
        self.connection
            .notify::<lsp::notification::DidChangeTextDocument>(lsp::DidChangeTextDocumentParams {
                text_document: lsp::VersionedTextDocumentIdentifier {
                    uri,
                    version: document.version,
                },
                content_changes: changes,
            })
            .ok();
    }

    /// The selector the server's features are registered for.
    pub(crate) fn default_selector(&self) -> LanguageSelector {
        if self.options.languages.is_empty() {
            LanguageSelector::All
        } else {
            LanguageSelector::Languages(self.options.languages.clone())
        }
    }

    fn register(&mut self, cx: &mut Context<Self>) {
        let selector = self.default_selector();
        let features = LspFeatures::new(cx.weak_entity(), &self.capabilities);
        self.registrations = features.register(selector.clone(), &self.capabilities, cx);
        // `lsp-types` has no `typeHierarchyProvider`: read from the raw
        // capabilities.
        if self.supports_type_hierarchy() {
            self.registrations
                .push(gpui_monaco::providers::register_type_hierarchy_provider(
                    cx,
                    selector.clone(),
                    features,
                ));
        }
        let inline_completions =
            crate::inline_completions::register(self, cx.weak_entity(), selector, cx);
        self.registrations.extend(inline_completions);
        let ids = self
            .capabilities
            .execute_command_provider
            .as_ref()
            .map(|options| options.commands.clone())
            .unwrap_or_default();
        self.commands = self.register_commands(ids, cx);
    }

    /// Registers the server's commands with [`gpui_monaco::register_command`];
    /// they run on the server (`workspace/executeCommand`).
    pub(crate) fn register_commands(
        &self,
        ids: Vec<String>,
        cx: &mut App,
    ) -> Vec<gpui_monaco::Disposable> {
        let mut commands = Vec::new();
        for id in ids {
            let connection = self.connection.clone();
            let command = id.clone();
            let registration = gpui_monaco::register_command(cx, id, move |arguments, cx| {
                let request =
                    connection.request::<lsp::request::ExecuteCommand>(lsp::ExecuteCommandParams {
                        command: command.clone(),
                        arguments: arguments.to_vec(),
                        work_done_progress_params: Default::default(),
                    });
                cx.background_executor()
                    .spawn(async move {
                        if let Err(err) = request.await {
                            log::warn!("{err:#}");
                        }
                    })
                    .detach();
            });
            commands.push(registration);
        }
        commands
    }

    fn unregister(&mut self, cx: &mut App) {
        for registration in self.registrations.drain(..) {
            unregister_provider(cx, registration);
        }
        // Only this server's handlers go; another's for the same id stay.
        for registration in self.commands.drain(..) {
            registration.dispose(cx);
        }
        for registration in self.dynamic.drain(..) {
            registration.dispose(cx);
        }
    }

    fn exited(&mut self, cx: &mut Context<Self>) {
        self.running = false;
        self.pull = None;
        self.unregister(cx);
        cx.emit(LspClientEvent::Exited);
    }

    fn handle_incoming(&mut self, message: Incoming, cx: &mut Context<Self>) {
        match message {
            Incoming::Notification { method, params } => {
                self.handle_notification(&method, params, cx)
            }
            Incoming::Request { id, method, params }
                if method
                    == <lsp::request::ApplyWorkspaceEdit as lsp::request::Request>::METHOD =>
            {
                // Applying may wait for the host's file operations or
                // preview, so the response comes when it's done.
                let params = match rpc::params::<lsp::ApplyWorkspaceEditParams>(params) {
                    Ok(params) => params,
                    Err(err) => {
                        let error = ResponseError {
                            code: INVALID_PARAMS,
                            message: err.to_string(),
                        };
                        self.connection.respond(id, Err(error)).ok();
                        return;
                    }
                };
                let response = self.apply_workspace_edit(params.edit, cx);
                let connection = self.connection.clone();
                cx.background_executor()
                    .spawn(async move {
                        let response = response.await;
                        let result = serde_json::to_value(response).unwrap_or(Value::Null);
                        connection.respond(id, Ok(result)).ok();
                    })
                    .detach();
            }
            Incoming::Request { id, method, params }
                if method
                    == <lsp::request::ShowMessageRequest as lsp::request::Request>::METHOD =>
            {
                let params = match rpc::params::<lsp::ShowMessageRequestParams>(params) {
                    Ok(params) => params,
                    Err(err) => {
                        let error = ResponseError {
                            code: INVALID_PARAMS,
                            message: err.to_string(),
                        };
                        self.connection.respond(id, Err(error)).ok();
                        return;
                    }
                };
                let respond = MessageResponder(Arc::new(Responder {
                    connection: self.connection.clone(),
                    id,
                    answered: AtomicBool::new(false),
                }));
                cx.emit(LspClientEvent::ShowMessageRequest {
                    kind: params.typ,
                    message: params.message,
                    actions: params.actions.unwrap_or_default(),
                    respond,
                });
            }
            Incoming::Request { id, method, params } => {
                let result = self.handle_request(&method, params, cx);
                self.connection.respond(id, result).ok();
            }
        }
    }

    fn handle_notification(&mut self, method: &str, params: Value, cx: &mut Context<Self>) {
        use lsp::notification::{LogMessage, Notification as _, PublishDiagnostics, ShowMessage};
        match method {
            PublishDiagnostics::METHOD => {
                let Ok(params) = rpc::params::<lsp::PublishDiagnosticsParams>(params) else {
                    return;
                };
                // Servers may write uris another way than the client
                // (`vscode-uri` turns `C:` into `c%3A`).
                let uri = convert::uri(&params.uri).to_string();
                self.set_diagnostics(uri, params.diagnostics, cx);
            }
            ShowMessage::METHOD => {
                if let Ok(params) = rpc::params::<lsp::ShowMessageParams>(params) {
                    cx.emit(LspClientEvent::ShowMessage {
                        kind: params.typ,
                        message: params.message,
                    });
                }
            }
            LogMessage::METHOD => {
                if let Ok(params) = rpc::params::<lsp::LogMessageParams>(params) {
                    let level = match params.typ {
                        lsp::MessageType::ERROR => log::Level::Error,
                        lsp::MessageType::WARNING => log::Level::Warn,
                        lsp::MessageType::INFO => log::Level::Info,
                        _ => log::Level::Debug,
                    };
                    log::log!(level, "{}: {}", self.options.name, params.message);
                }
            }
            _ => {}
        }
    }

    /// Makes `diagnostics` the markers of `uri` (normalized), pushed or
    /// pulled.
    fn set_diagnostics(
        &mut self,
        uri: String,
        diagnostics: Vec<lsp::Diagnostic>,
        cx: &mut Context<Self>,
    ) {
        let published: Vec<(Marker, lsp::Diagnostic)> = diagnostics
            .into_iter()
            .map(|diagnostic| (convert::marker(diagnostic.clone()), diagnostic))
            .collect();
        let markers = published.iter().map(|(marker, _)| marker.clone()).collect();
        if published.is_empty() {
            self.diagnostics.remove(&uri);
        } else {
            self.diagnostics.insert(uri.clone(), published);
        }
        let owner = self.markers_owner();
        // Documents that aren't attached get them too, as
        // `vscode-languageclient`'s diagnostic collection does.
        match self.documents.get(&uri).and_then(|d| d.model.upgrade()) {
            Some(model) => model.update(cx, |model, cx| model.set_markers(owner, markers, cx)),
            None => gpui_monaco::set_markers_for_uri(cx, owner, &uri, markers),
        }
    }

    fn handle_request(
        &mut self,
        method: &str,
        params: Value,
        cx: &mut Context<Self>,
    ) -> Result<Value, ResponseError> {
        use lsp::request::{
            InlayHintRefreshRequest, RegisterCapability, Request as _, UnregisterCapability,
            WorkDoneProgressCreate, WorkspaceConfiguration, WorkspaceFoldersRequest,
        };
        let invalid = |err: anyhow::Error| ResponseError {
            code: INVALID_PARAMS,
            message: err.to_string(),
        };
        match method {
            WorkspaceConfiguration::METHOD => {
                let params: lsp::ConfigurationParams = rpc::params(params).map_err(invalid)?;
                Ok(Value::Array(
                    params
                        .items
                        .iter()
                        .map(|item| setting(&self.options.settings, item.section.as_deref()))
                        .collect(),
                ))
            }
            InlayHintRefreshRequest::METHOD => {
                gpui_monaco::providers::refresh_inlay_hints(cx);
                Ok(Value::Null)
            }
            lsp_types::request::InlineValueRefreshRequest::METHOD => {
                gpui_monaco::providers::refresh_inline_values(cx);
                cx.emit(LspClientEvent::InlineValuesRefresh);
                Ok(Value::Null)
            }
            lsp_types::request::SemanticTokensRefresh::METHOD => {
                gpui_monaco::providers::refresh_semantic_tokens(cx);
                Ok(Value::Null)
            }
            // `vscode-languageclient` fires its code lens provider's
            // `onDidChangeCodeLenses`.
            lsp_types::request::CodeLensRefresh::METHOD => {
                gpui_monaco::providers::refresh_code_lenses(cx);
                Ok(Value::Null)
            }
            lsp_types::request::WorkspaceDiagnosticRefresh::METHOD => {
                self.refresh_pulled_diagnostics(cx);
                Ok(Value::Null)
            }
            RegisterCapability::METHOD => {
                let params: lsp::RegistrationParams = rpc::params(params).map_err(invalid)?;
                self.register_capabilities(params, cx);
                Ok(Value::Null)
            }
            UnregisterCapability::METHOD => {
                let params: lsp::UnregistrationParams = rpc::params(params).map_err(invalid)?;
                self.unregister_capabilities(params, cx);
                Ok(Value::Null)
            }
            WorkDoneProgressCreate::METHOD => Ok(Value::Null),
            WorkspaceFoldersRequest::METHOD => Ok(json!(workspace_folders(&self.options))),
            _ => Err(ResponseError {
                code: METHOD_NOT_FOUND,
                message: format!("unhandled method {method}"),
            }),
        }
    }

    /// Applies `edit` (`workspace/applyEdit`) through gpui-monaco's bulk
    /// edit service ([`gpui_monaco::apply_workspace_edit`]), as
    /// `vscode-languageclient` hands it to `workspace.applyEdit`: text edits
    /// go to any document [`gpui_monaco::model_for_uri`] finds, attached or
    /// not, each as one undo step; file operations go to the host's
    /// [file edit handler](gpui_monaco::set_workspace_file_edit_handler);
    /// and change annotations that need confirmation go through the
    /// [preview handler](gpui_monaco::set_workspace_edit_preview_handler).
    ///
    /// As in `vscode-languageclient`, an edit for a version of an attached
    /// document the server no longer has isn't applied. Otherwise the
    /// response says whether it was, with the bulk edit service's error as
    /// the `failureReason`.
    pub fn apply_workspace_edit(
        &mut self,
        edit: lsp::WorkspaceEdit,
        cx: &mut Context<Self>,
    ) -> Task<lsp::ApplyWorkspaceEditResponse> {
        if self.version_mismatch(&edit, cx) {
            return Task::ready(lsp::ApplyWorkspaceEditResponse {
                applied: false,
                failure_reason: None,
                failed_change: None,
            });
        }
        let edit = match convert::workspace_edit(edit) {
            Ok(edit) => edit,
            Err(err) => return Task::ready(failed(&err.to_string())),
        };
        let apply = gpui_monaco::apply_workspace_edit(cx, edit);
        cx.background_executor().spawn(async move {
            match apply.await {
                Ok(applied) => lsp::ApplyWorkspaceEditResponse {
                    applied,
                    failure_reason: None,
                    failed_change: None,
                },
                Err(err) => failed(&err.to_string()),
            }
        })
    }

    /// `vscode-languageclient`'s `handleApplyWorkspaceEdit` check: whether
    /// a versioned document edit is for an attached document whose version
    /// on the server is another.
    fn version_mismatch(&mut self, edit: &lsp::WorkspaceEdit, cx: &mut Context<Self>) -> bool {
        let document_edits: Vec<&lsp::TextDocumentEdit> = match &edit.document_changes {
            Some(lsp::DocumentChanges::Edits(edits)) => edits.iter().collect(),
            Some(lsp::DocumentChanges::Operations(operations)) => operations
                .iter()
                .filter_map(|operation| match operation {
                    lsp::DocumentChangeOperation::Edit(edit) => Some(edit),
                    lsp::DocumentChangeOperation::Op(_) => None,
                })
                .collect(),
            None => Vec::new(),
        };
        document_edits.into_iter().any(|edit| {
            let Some(version) = edit.text_document.version.filter(|v| *v >= 0) else {
                return false;
            };
            let uri = convert::uri(&edit.text_document.uri).to_string();
            self.sync(&uri, cx);
            self.documents
                .get(&uri)
                .is_some_and(|document| document.version != version)
        })
    }
}

fn failed(reason: &str) -> lsp::ApplyWorkspaceEditResponse {
    lsp::ApplyWorkspaceEditResponse {
        applied: false,
        failure_reason: Some(reason.to_string()),
        failed_change: None,
    }
}

/// The value at dotted `section` of `settings` (all of it for `None`).
fn setting(settings: &Value, section: Option<&str>) -> Value {
    let Some(section) = section.filter(|s| !s.is_empty()) else {
        return settings.clone();
    };
    section
        .split('.')
        .try_fold(settings, |value, key| value.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn workspace_folders(options: &LspClientOptions) -> Option<Vec<lsp::WorkspaceFolder>> {
    let root = options.root_uri.as_deref()?;
    let uri = lsp::Uri::from_str(root).ok()?;
    let name = root
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(root)
        .to_string();
    Some(vec![lsp::WorkspaceFolder { uri, name }])
}

/// The `initialize` request's params.
fn initialize_params(options: &LspClientOptions) -> Result<Value> {
    let mut capabilities = json!({
        "general": { "positionEncodings": ["utf-16"] },
        "window": {
            "workDoneProgress": true,
            "showMessage": { "messageActionItem": { "additionalPropertiesSupport": true } },
        },
        "workspace": {
            "applyEdit": true,
            "executeCommand": {},
            "symbol": {
                "symbolKind": { "valueSet": (1..=26).collect::<Vec<u32>>() },
                "tagSupport": { "valueSet": [1] },
                "resolveSupport": { "properties": ["location.range"] },
            },
            "inlayHint": { "refreshSupport": true },
            "inlineValue": { "refreshSupport": true },
            "semanticTokens": { "refreshSupport": true },
            "codeLens": { "refreshSupport": true },
            "configuration": true,
            "didChangeConfiguration": {},
            "didChangeWatchedFiles": { "dynamicRegistration": true, "relativePatternSupport": true },
            "workspaceFolders": true,
            "workspaceEdit": { "documentChanges": true },
        },
        "textDocument": {
            "synchronization": {
                "didSave": true,
                "willSave": true,
                "willSaveWaitUntil": true,
                "dynamicRegistration": false,
            },
            "completion": {
                "completionItem": {
                    "snippetSupport": true,
                    "insertTextModeSupport": { "valueSet": [1, 2] },
                    "commitCharactersSupport": true,
                    "documentationFormat": ["markdown", "plaintext"],
                    "preselectSupport": true,
                    "insertReplaceSupport": true,
                    "labelDetailsSupport": true,
                    "deprecatedSupport": true,
                    "tagSupport": { "valueSet": [1] },
                    "resolveSupport": {
                        "properties": ["documentation", "detail", "additionalTextEdits"],
                    },
                },
                "contextSupport": true,
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "signatureHelp": {
                "signatureInformation": {
                    "documentationFormat": ["markdown", "plaintext"],
                    "parameterInformation": { "labelOffsetSupport": true },
                    "activeParameterSupport": true,
                },
                "contextSupport": true,
            },
            "definition": { "linkSupport": true },
            "declaration": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": {},
            "rename": { "prepareSupport": true },
            "inlayHint": {},
            "semanticTokens": {
                "requests": { "range": true, "full": { "delta": true } },
                "tokenTypes": [
                    "namespace", "type", "class", "enum", "interface", "struct",
                    "typeParameter", "parameter", "variable", "property", "enumMember",
                    "event", "function", "method", "macro", "keyword", "modifier",
                    "comment", "string", "number", "regexp", "operator", "decorator",
                ],
                "tokenModifiers": [
                    "declaration", "definition", "readonly", "static", "deprecated",
                    "abstract", "async", "modification", "documentation", "defaultLibrary",
                ],
                "formats": ["relative"],
                "overlappingTokenSupport": false,
                "multilineTokenSupport": false,
                "augmentsSyntaxTokens": true,
            },
            "foldingRange": { "lineFoldingOnly": true },
            "callHierarchy": {},
            "typeHierarchy": {},
            "inlineValue": {},
            "documentSymbol": {
                "hierarchicalDocumentSymbolSupport": true,
                "tagSupport": { "valueSet": [1] },
            },
            "selectionRange": {},
            "codeAction": {
                "codeActionLiteralSupport": {
                    "codeActionKind": {
                        "valueSet": [
                            "", "quickfix", "refactor", "refactor.extract", "refactor.inline",
                            "refactor.rewrite", "refactor.move", "source",
                            "source.organizeImports", "source.fixAll",
                        ],
                    },
                },
                "isPreferredSupport": true,
                "disabledSupport": true,
                "dataSupport": true,
                "resolveSupport": { "properties": ["edit"] },
            },
            "documentHighlight": {},
            "codeLens": {},
            "documentLink": { "tooltipSupport": true },
            "colorProvider": {},
            "linkedEditingRange": {},
            "formatting": {},
            "rangeFormatting": {},
            "onTypeFormatting": {},
            "diagnostic": { "dynamicRegistration": false, "relatedDocumentSupport": true },
            "publishDiagnostics": {
                "relatedInformation": true,
                "versionSupport": true,
                "tagSupport": { "valueSet": [1, 2] },
            },
        },
    });
    // Every feature the bridge can register dynamically says so; document
    // sync is static.
    registration::advertise_dynamic_registration(&mut capabilities);
    let capabilities: lsp::ClientCapabilities = serde_json::from_value(capabilities)?;
    #[allow(deprecated)]
    let params = lsp::InitializeParams {
        process_id: Some(std::process::id()),
        root_uri: options
            .root_uri
            .as_deref()
            .and_then(|root| lsp::Uri::from_str(root).ok()),
        workspace_folders: workspace_folders(options),
        initialization_options: options.initialization_options.clone(),
        capabilities,
        client_info: Some(lsp::ClientInfo {
            name: "gpui-monaco".into(),
            version: Some(env!("CARGO_PKG_VERSION").into()),
        }),
        ..Default::default()
    };
    let mut params = serde_json::to_value(params)?;
    // `lsp-types` names this `workspace.diagnostic`; the protocol has
    // `workspace.diagnostics`.
    params["capabilities"]["workspace"]["diagnostics"] = json!({ "refreshSupport": true });
    // `lsp-types` has `textDocument.inlineCompletion` only behind its
    // `proposed` feature.
    params["capabilities"]["textDocument"]["inlineCompletion"] =
        json!({ "dynamicRegistration": true });
    Ok(params)
}

trait LogErr {
    fn log_err(self);
}

impl<T> LogErr for Result<T> {
    fn log_err(self) {
        if let Err(err) = self {
            log::warn!("{err:#}");
        }
    }
}

impl<T> LogErr for Result<T, gpui_monaco::text::EditError> {
    fn log_err(self) {
        if let Err(err) = self {
            log::warn!("{err:?}");
        }
    }
}
