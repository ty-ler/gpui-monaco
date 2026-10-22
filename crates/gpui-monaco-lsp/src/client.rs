//! The client side of one language server: lifecycle, document sync,
//! diagnostics and the requests a server sends the client.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
use crate::rpc::{self, Connection, Incoming, METHOD_NOT_FOUND, ResponseError};

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
    options: LspClientOptions,
    connection: Connection,
    capabilities: lsp::ServerCapabilities,
    server_info: Option<lsp::ServerInfo>,
    /// The attached documents, by [`TextModel::resource`].
    documents: HashMap<String, Document>,
    /// The diagnostics last published for each uri (normalized), with
    /// their markers: code action requests send the server's own back
    /// (`vscode-languageclient` keeps them in its diagnostic collection).
    diagnostics: HashMap<String, Vec<(Marker, lsp::Diagnostic)>>,
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
        let initialize = initialize_params(&options)
            .map(|params| connection.request::<lsp::request::Initialize>(params));
        cx.spawn(async move |cx| {
            let result = initialize?.await?;
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
                let mut client = Self {
                    options,
                    connection,
                    capabilities: result.capabilities,
                    server_info: result.server_info,
                    documents: HashMap::new(),
                    diagnostics: HashMap::new(),
                    registrations: Vec::new(),
                    commands: Vec::new(),
                    running: true,
                    _tasks: tasks,
                    _child: child,
                };
                client.register(cx);
                client
            }))
        })
    }

    /// What the server said it can do.
    pub fn capabilities(&self) -> &lsp::ServerCapabilities {
        &self.capabilities
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
                    TextModelEvent::Edited { .. } => this.sync(&uri, cx),
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
            uri,
            Document {
                model: model.downgrade(),
                snapshot,
                version: 0,
                _subscriptions: subscriptions,
            },
        );
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

    /// Asks the server to shut down (`shutdown`, then `exit`). Its
    /// features are unregistered right away.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        self.unregister(cx);
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

    fn register(&mut self, cx: &mut Context<Self>) {
        let selector = if self.options.languages.is_empty() {
            LanguageSelector::All
        } else {
            LanguageSelector::Languages(self.options.languages.clone())
        };
        let features = LspFeatures::new(cx.weak_entity(), &self.capabilities);
        self.registrations = features.register(selector, &self.capabilities, cx);
        // The server's commands run on the server (`workspace/executeCommand`).
        let ids = self
            .capabilities
            .execute_command_provider
            .as_ref()
            .map(|options| options.commands.clone())
            .unwrap_or_default();
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
            self.commands.push(registration);
        }
    }

    fn unregister(&mut self, cx: &mut App) {
        for registration in self.registrations.drain(..) {
            unregister_provider(cx, registration);
        }
        // Only this server's handlers go; another's for the same id stay.
        for registration in self.commands.drain(..) {
            registration.dispose(cx);
        }
    }

    fn exited(&mut self, cx: &mut Context<Self>) {
        self.running = false;
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
                let published: Vec<(Marker, lsp::Diagnostic)> = params
                    .diagnostics
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
                    Some(model) => {
                        model.update(cx, |model, cx| model.set_markers(owner, markers, cx))
                    }
                    None => gpui_monaco::set_markers_for_uri(cx, owner, &uri, markers),
                }
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
            RegisterCapability::METHOD
            | UnregisterCapability::METHOD
            | WorkDoneProgressCreate::METHOD => Ok(Value::Null),
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

fn initialize_params(options: &LspClientOptions) -> Result<lsp::InitializeParams> {
    let capabilities: lsp::ClientCapabilities = serde_json::from_value(json!({
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
            "semanticTokens": { "refreshSupport": true },
            "codeLens": { "refreshSupport": true },
            "configuration": true,
            "didChangeConfiguration": {},
            "workspaceFolders": true,
            "workspaceEdit": { "documentChanges": true },
        },
        "textDocument": {
            "synchronization": { "didSave": true, "dynamicRegistration": false },
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
            "publishDiagnostics": {
                "relatedInformation": true,
                "versionSupport": true,
                "tagSupport": { "valueSet": [1, 2] },
            },
        },
    }))?;
    #[allow(deprecated)]
    Ok(lsp::InitializeParams {
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
    })
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
