//! Pull diagnostics (`textDocument/diagnostic`, `workspace/diagnostic`),
//! as `vscode-languageclient`'s diagnostic feature does them: attached
//! documents are pulled when opened and after edits, `unchanged` reports
//! keep the markers they had, and `workspace/diagnostic/refresh` pulls
//! everything again. The results go to the same markers as pushed
//! diagnostics.

use std::collections::HashMap;
use std::time::Duration;

use futures_lite::future::Boxed;
use gpui::{Context, Task};
use lsp_types as lsp;
use serde_json::{Value, json};

use super::LspClient;
use crate::convert;
use crate::rpc::ResponseError;

/// How long after an edit a document is pulled, so typing sends one
/// request rather than one per keystroke.
const PULL_DEBOUNCE: Duration = Duration::from_millis(200);

/// How long after a `workspace/diagnostic` response the next is sent
/// (`vscode-languageclient` waits 2 seconds too).
const WORKSPACE_PULL_INTERVAL: Duration = Duration::from_secs(2);

/// LSP's `ServerCancelled` error: the server wants the request sent again.
const SERVER_CANCELLED: i64 = -32802;

/// The pull state of a server that has a `diagnosticProvider`.
pub(super) struct PullDiagnostics {
    identifier: Option<String>,
    inter_file_dependencies: bool,
    workspace_diagnostics: bool,
    /// The `resultId` of each document's last report, by uri.
    result_ids: HashMap<String, String>,
    /// Each attached document's pending pull; replacing one cancels it.
    pulls: HashMap<String, Task<()>>,
    workspace_pull: Option<Task<()>>,
}

impl PullDiagnostics {
    pub(super) fn new(capabilities: &lsp::ServerCapabilities) -> Option<Self> {
        let options = match capabilities.diagnostic_provider.as_ref()? {
            lsp::DiagnosticServerCapabilities::Options(options) => options,
            lsp::DiagnosticServerCapabilities::RegistrationOptions(options) => {
                &options.diagnostic_options
            }
        };
        Some(Self {
            identifier: options.identifier.clone(),
            inter_file_dependencies: options.inter_file_dependencies,
            workspace_diagnostics: options.workspace_diagnostics,
            result_ids: HashMap::new(),
            pulls: HashMap::new(),
            workspace_pull: None,
        })
    }

    /// Forgets a document that was closed, cancelling its pull.
    pub(super) fn forget(&mut self, uri: &str) {
        self.result_ids.remove(uri);
        self.pulls.remove(uri);
    }
}

impl LspClient {
    /// Pulls `uri`'s diagnostics now.
    pub(super) fn pull_diagnostics(&mut self, uri: &str, cx: &mut Context<Self>) {
        self.schedule_pull(uri, None, cx);
    }

    /// Pulls diagnostics after `uri` was edited: its own, and every attached
    /// document's when the server said diagnostics depend on other files.
    pub(super) fn pull_diagnostics_after_edit(&mut self, uri: &str, cx: &mut Context<Self>) {
        let Some(pull) = &self.pull else {
            return;
        };
        let uris: Vec<String> = if pull.inter_file_dependencies {
            self.documents.keys().cloned().collect()
        } else {
            vec![uri.to_string()]
        };
        for uri in uris {
            self.schedule_pull(&uri, Some(PULL_DEBOUNCE), cx);
        }
    }

    /// `workspace/diagnostic/refresh`: pulls every attached document again,
    /// and the workspace's diagnostics if the server has them.
    pub(super) fn refresh_pulled_diagnostics(&mut self, cx: &mut Context<Self>) {
        if self.pull.is_none() {
            return;
        }
        let uris: Vec<String> = self.documents.keys().cloned().collect();
        for uri in uris {
            self.pull_diagnostics(&uri, cx);
        }
        self.pull_workspace_diagnostics(None, cx);
    }

    fn schedule_pull(&mut self, uri: &str, delay: Option<Duration>, cx: &mut Context<Self>) {
        if self.pull.is_none() || !self.documents.contains_key(uri) {
            return;
        }
        let owned_uri = uri.to_string();
        let task = cx.spawn(async move |this, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            let uri = owned_uri;
            let Ok(Some(request)) = this.update(cx, |this, cx| this.document_pull(&uri, cx)) else {
                return;
            };
            let result = request.await;
            this.update(cx, |this, cx| match result {
                Ok(report) => this.apply_document_report(&uri, report, cx),
                Err(err) if retrigger(&err) => {
                    this.schedule_pull(&uri, Some(PULL_DEBOUNCE), cx);
                }
                Err(err) => log::debug!("{}: {err:#}", this.options.name),
            })
            .ok();
        });
        if let Some(pull) = &mut self.pull {
            pull.pulls.insert(uri.to_string(), task);
        }
    }

    /// Syncs `uri` and sends its `textDocument/diagnostic` request.
    fn document_pull(
        &mut self,
        uri: &str,
        cx: &mut Context<Self>,
    ) -> Option<Boxed<anyhow::Result<Value>>> {
        let (connection, text_document) = self.prepare_request(uri, cx)?;
        let pull = self.pull.as_ref()?;
        // Optional fields are left out rather than sent as `null`.
        let mut params = json!({ "textDocument": text_document });
        if let Some(identifier) = &pull.identifier {
            params["identifier"] = json!(identifier);
        }
        if let Some(result_id) = pull.result_ids.get(uri) {
            params["previousResultId"] = json!(result_id);
        }
        Some(Box::pin(connection.request_raw(
            <lsp::request::DocumentDiagnosticRequest as lsp::request::Request>::METHOD,
            params,
        )))
    }

    fn apply_document_report(&mut self, uri: &str, report: Value, cx: &mut Context<Self>) {
        let report = match serde_json::from_value::<lsp::DocumentDiagnosticReportResult>(report) {
            Ok(report) => report,
            Err(err) => {
                log::warn!("{}: invalid diagnostic report: {err}", self.options.name);
                return;
            }
        };
        let related = match report {
            lsp::DocumentDiagnosticReportResult::Report(lsp::DocumentDiagnosticReport::Full(
                report,
            )) => {
                let full = report.full_document_diagnostic_report;
                self.set_result_id(uri, full.result_id);
                self.set_diagnostics(uri.to_string(), full.items, cx);
                report.related_documents
            }
            lsp::DocumentDiagnosticReportResult::Report(
                lsp::DocumentDiagnosticReport::Unchanged(report),
            ) => {
                let result_id = report.unchanged_document_diagnostic_report.result_id;
                self.set_result_id(uri, Some(result_id));
                report.related_documents
            }
            lsp::DocumentDiagnosticReportResult::Partial(report) => report.related_documents,
        };
        for (related_uri, report) in related.into_iter().flatten() {
            let related_uri = convert::uri(&related_uri).to_string();
            match report {
                lsp::DocumentDiagnosticReportKind::Full(report) => {
                    self.set_result_id(&related_uri, report.result_id);
                    self.set_diagnostics(related_uri, report.items, cx);
                }
                lsp::DocumentDiagnosticReportKind::Unchanged(report) => {
                    self.set_result_id(&related_uri, Some(report.result_id));
                }
            }
        }
    }

    /// Remembers `uri`'s last `resultId`, for attached documents only.
    fn set_result_id(&mut self, uri: &str, result_id: Option<String>) {
        if !self.documents.contains_key(uri) {
            return;
        }
        let Some(pull) = &mut self.pull else {
            return;
        };
        match result_id {
            Some(result_id) => pull.result_ids.insert(uri.to_string(), result_id),
            None => pull.result_ids.remove(uri),
        };
    }

    /// Sends `workspace/diagnostic` (after `delay`), if the server has
    /// workspace diagnostics, and again a while after each response.
    pub(super) fn pull_workspace_diagnostics(
        &mut self,
        delay: Option<Duration>,
        cx: &mut Context<Self>,
    ) {
        let Some(pull) = &self.pull else {
            return;
        };
        if !pull.workspace_diagnostics || !self.running {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            let Ok(Some(request)) = this.update(cx, |this, _| this.workspace_pull()) else {
                return;
            };
            let result = request.await;
            this.update(cx, |this, cx| match result {
                Ok(report) => {
                    this.apply_workspace_report(report, cx);
                    this.pull_workspace_diagnostics(Some(WORKSPACE_PULL_INTERVAL), cx);
                }
                Err(err) if retrigger(&err) => {
                    this.pull_workspace_diagnostics(Some(WORKSPACE_PULL_INTERVAL), cx);
                }
                Err(err) => log::debug!("{}: {err:#}", this.options.name),
            })
            .ok();
        });
        if let Some(pull) = &mut self.pull {
            pull.workspace_pull = Some(task);
        }
    }

    fn workspace_pull(&self) -> Option<Boxed<anyhow::Result<Value>>> {
        let pull = self.pull.as_ref()?;
        let previous_result_ids: Vec<Value> = pull
            .result_ids
            .iter()
            .map(|(uri, value)| json!({ "uri": uri, "value": value }))
            .collect();
        let mut params = json!({ "previousResultIds": previous_result_ids });
        if let Some(identifier) = &pull.identifier {
            params["identifier"] = json!(identifier);
        }
        Some(Box::pin(self.connection.request_raw(
            <lsp::request::WorkspaceDiagnosticRequest as lsp::request::Request>::METHOD,
            params,
        )))
    }

    /// Applies a workspace report's full reports. An attached document's
    /// is skipped when it's for another version than the server has; its
    /// own pulls keep it up to date.
    fn apply_workspace_report(&mut self, report: Value, cx: &mut Context<Self>) {
        let report = match serde_json::from_value::<lsp::WorkspaceDiagnosticReportResult>(report) {
            Ok(report) => report,
            Err(err) => {
                log::warn!(
                    "{}: invalid workspace diagnostic report: {err}",
                    self.options.name
                );
                return;
            }
        };
        let items = match report {
            lsp::WorkspaceDiagnosticReportResult::Report(report) => report.items,
            lsp::WorkspaceDiagnosticReportResult::Partial(report) => report.items,
        };
        for item in items {
            let lsp::WorkspaceDocumentDiagnosticReport::Full(report) = item else {
                continue;
            };
            let uri = convert::uri(&report.uri).to_string();
            if let Some(document) = self.documents.get(&uri)
                && report
                    .version
                    .is_some_and(|version| version != i64::from(document.version))
            {
                continue;
            }
            let full = report.full_document_diagnostic_report;
            self.set_result_id(&uri, full.result_id);
            self.set_diagnostics(uri, full.items, cx);
        }
    }
}

/// Whether a failed pull should be sent again: the server cancelled it
/// (`ServerCancelled`; the error's `retriggerRequest` defaults to true).
fn retrigger(err: &anyhow::Error) -> bool {
    err.downcast_ref::<ResponseError>()
        .is_some_and(|err| err.code == SERVER_CANCELLED)
}
