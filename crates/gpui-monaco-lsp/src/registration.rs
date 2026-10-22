//! Dynamic capability registration (`client/registerCapability` and
//! `client/unregisterCapability`), as `vscode-languageclient`'s dynamic
//! features do it: a registration for a feature the bridge has registers
//! that feature's providers with the registration's document selector and
//! options, and an unregistration removes them.

use gpui::{App, Context};
use gpui_monaco::providers::{
    LanguageFilter, LanguageSelector, ProviderRegistration, unregister_provider,
};
use lsp_types as lsp;
use serde_json::{Map, Value};

use crate::client::LspClient;
use crate::features::LspFeatures;

/// The requests a server can register dynamically, with the
/// `ServerCapabilities` field a registration fills. The client advertises
/// `dynamicRegistration` for each one's client capability.
pub(crate) const DYNAMIC_FEATURES: &[(&str, &str)] = &[
    ("textDocument/completion", "completionProvider"),
    ("textDocument/hover", "hoverProvider"),
    ("textDocument/signatureHelp", "signatureHelpProvider"),
    ("textDocument/definition", "definitionProvider"),
    ("textDocument/declaration", "declarationProvider"),
    ("textDocument/typeDefinition", "typeDefinitionProvider"),
    ("textDocument/implementation", "implementationProvider"),
    ("textDocument/references", "referencesProvider"),
    (
        "textDocument/documentHighlight",
        "documentHighlightProvider",
    ),
    ("textDocument/documentSymbol", "documentSymbolProvider"),
    ("textDocument/codeAction", "codeActionProvider"),
    ("textDocument/codeLens", "codeLensProvider"),
    ("textDocument/documentLink", "documentLinkProvider"),
    ("textDocument/documentColor", "colorProvider"),
    ("textDocument/formatting", "documentFormattingProvider"),
    (
        "textDocument/rangeFormatting",
        "documentRangeFormattingProvider",
    ),
    (
        "textDocument/onTypeFormatting",
        "documentOnTypeFormattingProvider",
    ),
    ("textDocument/rename", "renameProvider"),
    ("textDocument/foldingRange", "foldingRangeProvider"),
    ("textDocument/selectionRange", "selectionRangeProvider"),
    (
        "textDocument/linkedEditingRange",
        "linkedEditingRangeProvider",
    ),
    ("textDocument/semanticTokens", "semanticTokensProvider"),
    ("textDocument/inlayHint", "inlayHintProvider"),
    ("textDocument/prepareCallHierarchy", "callHierarchyProvider"),
    ("textDocument/prepareTypeHierarchy", "typeHierarchyProvider"),
    ("textDocument/inlineValue", "inlineValueProvider"),
    ("textDocument/inlineCompletion", "inlineCompletionProvider"),
    ("workspace/symbol", "workspaceSymbolProvider"),
    ("workspace/executeCommand", "executeCommandProvider"),
];

/// The client capability that advertises dynamic registration for
/// `method`'s feature: its path under `ClientCapabilities`.
pub(crate) fn client_capability(method: &str) -> Option<(&'static str, &'static str)> {
    Some(match method {
        "textDocument/completion" => ("textDocument", "completion"),
        "textDocument/hover" => ("textDocument", "hover"),
        "textDocument/signatureHelp" => ("textDocument", "signatureHelp"),
        "textDocument/definition" => ("textDocument", "definition"),
        "textDocument/declaration" => ("textDocument", "declaration"),
        "textDocument/typeDefinition" => ("textDocument", "typeDefinition"),
        "textDocument/implementation" => ("textDocument", "implementation"),
        "textDocument/references" => ("textDocument", "references"),
        "textDocument/documentHighlight" => ("textDocument", "documentHighlight"),
        "textDocument/documentSymbol" => ("textDocument", "documentSymbol"),
        "textDocument/codeAction" => ("textDocument", "codeAction"),
        "textDocument/codeLens" => ("textDocument", "codeLens"),
        "textDocument/documentLink" => ("textDocument", "documentLink"),
        "textDocument/documentColor" => ("textDocument", "colorProvider"),
        "textDocument/formatting" => ("textDocument", "formatting"),
        "textDocument/rangeFormatting" => ("textDocument", "rangeFormatting"),
        "textDocument/onTypeFormatting" => ("textDocument", "onTypeFormatting"),
        "textDocument/rename" => ("textDocument", "rename"),
        "textDocument/foldingRange" => ("textDocument", "foldingRange"),
        "textDocument/selectionRange" => ("textDocument", "selectionRange"),
        "textDocument/linkedEditingRange" => ("textDocument", "linkedEditingRange"),
        "textDocument/semanticTokens" => ("textDocument", "semanticTokens"),
        "textDocument/inlayHint" => ("textDocument", "inlayHint"),
        "textDocument/prepareCallHierarchy" => ("textDocument", "callHierarchy"),
        "textDocument/prepareTypeHierarchy" => ("textDocument", "typeHierarchy"),
        "textDocument/inlineValue" => ("textDocument", "inlineValue"),
        // Advertised by `initialize_params` itself.
        "textDocument/inlineCompletion" => return None,
        "workspace/symbol" => ("workspace", "symbol"),
        "workspace/executeCommand" => ("workspace", "executeCommand"),
        _ => return None,
    })
}

/// Sets `dynamicRegistration: true` on the client capabilities of every
/// feature in [`DYNAMIC_FEATURES`] that `capabilities` has.
pub(crate) fn advertise_dynamic_registration(capabilities: &mut Value) {
    for (method, _) in DYNAMIC_FEATURES {
        let Some((group, name)) = client_capability(method) else {
            continue;
        };
        if let Some(Value::Object(feature)) = capabilities
            .get_mut(group)
            .and_then(|group| group.get_mut(name))
        {
            feature.insert("dynamicRegistration".into(), Value::Bool(true));
        }
    }
}

/// One `client/registerCapability` registration.
pub(crate) struct DynamicRegistration {
    pub(crate) id: String,
    pub(crate) method: String,
    /// Its `registerOptions` (`null` without).
    pub(crate) options: Value,
    providers: Vec<ProviderRegistration>,
    commands: Vec<gpui_monaco::Disposable>,
}

impl DynamicRegistration {
    pub(crate) fn dispose(self, cx: &mut App) {
        for registration in self.providers {
            unregister_provider(cx, registration);
        }
        for command in self.commands {
            command.dispose(cx);
        }
    }

    /// The `ServerCapabilities` field it fills, if the bridge has the
    /// feature.
    fn capability(&self) -> Option<&'static str> {
        capability_key(&self.method)
    }
}

fn capability_key(method: &str) -> Option<&'static str> {
    DYNAMIC_FEATURES
        .iter()
        .find(|(m, _)| *m == method)
        .map(|(_, key)| *key)
}

/// The registration's options as the capability's value: a registration
/// without options turns the feature on with its defaults.
fn capability_value(options: &Value) -> Value {
    match options {
        Value::Null => Value::Object(Map::new()),
        options => options.clone(),
    }
}

/// `ServerCapabilities` with only `key` set, from a registration's options.
fn partial_capabilities(key: &str, options: &Value) -> serde_json::Result<lsp::ServerCapabilities> {
    let mut capabilities = Map::new();
    capabilities.insert(key.to_string(), capability_value(options));
    serde_json::from_value(Value::Object(capabilities))
}

/// The selector for a registration's `documentSelector`, or `None` for the
/// client's own (a `null` or missing selector, as in
/// `vscode-languageclient`).
fn document_selector(options: &Value) -> Option<LanguageSelector> {
    let selector = options.get("documentSelector")?;
    let filters: Vec<lsp::DocumentFilter> = serde_json::from_value(selector.clone()).ok()?;
    Some(LanguageSelector::Filters(
        filters
            .into_iter()
            .map(|filter| LanguageFilter {
                language: filter.language.map(Into::into),
                scheme: filter.scheme.map(Into::into),
                pattern: filter.pattern.map(Into::into),
                exclusive: false,
            })
            .collect(),
    ))
}

impl LspClient {
    /// `client/registerCapability`: registers each registration's providers
    /// (for a feature the bridge has) and adds its options to
    /// [`LspClient::capabilities`] and [`LspClient::raw_capabilities`]. A registration for an id that's
    /// registered already replaces it. Registrations for other methods are
    /// kept with their options, for the bridge's notifications.
    pub(crate) fn register_capabilities(
        &mut self,
        params: lsp::RegistrationParams,
        cx: &mut Context<Self>,
    ) {
        for registration in params.registrations {
            self.remove_registration(&registration.id, cx);
            let options = registration.register_options.unwrap_or(Value::Null);
            let mut providers = Vec::new();
            let mut commands = Vec::new();
            if let Some(key) = capability_key(&registration.method) {
                let capabilities = match partial_capabilities(key, &options) {
                    Ok(capabilities) => capabilities,
                    Err(err) => {
                        log::warn!(
                            "{}: invalid {} registration options: {err}",
                            self.options.name,
                            registration.method
                        );
                        continue;
                    }
                };
                let selector =
                    document_selector(&options).unwrap_or_else(|| self.default_selector());
                let features = LspFeatures::new(cx.weak_entity(), &capabilities);
                if registration.method == "textDocument/inlineCompletion" {
                    providers.push(crate::inline_completions::register_provider(
                        cx.weak_entity(),
                        selector.clone(),
                        cx,
                    ));
                }
                providers.extend(features.register(selector, &capabilities, cx));
                if let Some(execute) = &capabilities.execute_command_provider {
                    commands = self.register_commands(execute.commands.clone(), cx);
                }
            }
            self.dynamic.push(DynamicRegistration {
                id: registration.id,
                method: registration.method,
                options,
                providers,
                commands,
            });
        }
        self.update_capabilities();
    }

    /// `client/unregisterCapability`: removes each registration's providers
    /// and capability.
    pub(crate) fn unregister_capabilities(
        &mut self,
        params: lsp::UnregistrationParams,
        cx: &mut Context<Self>,
    ) {
        for unregistration in params.unregisterations {
            self.remove_registration(&unregistration.id, cx);
        }
        self.update_capabilities();
    }

    fn remove_registration(&mut self, id: &str, cx: &mut App) {
        if let Some(ix) = self.dynamic.iter().position(|r| r.id == id) {
            self.dynamic.remove(ix).dispose(cx);
        }
    }

    /// The `initialize` result's capabilities with each registration's
    /// options on top, raw and parsed.
    fn update_capabilities(&mut self) {
        let mut capabilities = match &self.static_capabilities {
            Value::Object(capabilities) => capabilities.clone(),
            _ => Map::new(),
        };
        for registration in &self.dynamic {
            if let Some(key) = registration.capability() {
                capabilities.insert(key.to_string(), capability_value(&registration.options));
            }
        }
        let capabilities = Value::Object(capabilities);
        match serde_json::from_value(capabilities.clone()) {
            Ok(parsed) => {
                self.capabilities = parsed;
                self.raw_capabilities = capabilities;
            }
            Err(err) => log::warn!("{}: invalid capabilities: {err}", self.options.name),
        }
    }
}
