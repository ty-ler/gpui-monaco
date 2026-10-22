//! gpui-monaco providers that forward to a language server.

use anyhow::Result;
use gpui::{App, Task, WeakEntity};
use std::ops::Range;

use gpui_monaco::providers::{
    CodeAction, CodeActionContext, CodeActionKind, CodeActionProvider, CodeActionTrigger,
    register_code_action_provider,
};
use gpui_monaco::providers::{
    CodeLens, CodeLensProvider, ColorInformation, ColorPresentation, DocumentColorProvider,
    DocumentLink, LinkProvider, LinkedEditingRangeProvider, LinkedEditingRanges,
    register_code_lens_provider, register_color_provider, register_link_provider,
    register_linked_editing_range_provider,
};
use gpui_monaco::providers::{
    CompletionItem, CompletionList, CompletionProvider, CompletionTrigger, DefinitionProvider,
    DocumentFormattingProvider, DocumentHighlight, DocumentHighlightProvider,
    DocumentRangeFormattingProvider, FormattingEdits, FormattingOptions, Hover, HoverProvider,
    LanguageSelector, Location, OnTypeFormattingProvider, ProviderContext, ProviderRegistration,
    SignatureHelp, SignatureHelpContext, SignatureHelpProvider, SignatureHelpTrigger, TextEdit,
    register_completion_provider, register_definition_provider,
    register_document_formatting_provider, register_document_highlight_provider,
    register_document_range_formatting_provider, register_hover_provider,
    register_on_type_formatting_provider, register_signature_help_provider,
};
use gpui_monaco::providers::{
    DeclarationProvider, ImplementationProvider, ReferenceContext, ReferenceProvider,
    TypeDefinitionProvider, register_declaration_provider, register_implementation_provider,
    register_reference_provider, register_type_definition_provider,
};
use gpui_monaco::providers::{
    DocumentRangeSemanticTokensProvider, DocumentSemanticTokensProvider, SemanticTokens,
    SemanticTokensEdit, SemanticTokensEdits, SemanticTokensLegend, SemanticTokensResult,
    register_document_range_semantic_tokens_provider, register_document_semantic_tokens_provider,
};
use gpui_monaco::providers::{
    DocumentSymbol, DocumentSymbolProvider, register_document_symbol_provider,
};
use gpui_monaco::providers::{
    FoldingContext, FoldingRange, FoldingRangeKind, FoldingRangeProvider,
    register_folding_range_provider,
};
use gpui_monaco::providers::{InlayHint, InlayHintsProvider, register_inlay_hints_provider};
use gpui_monaco::providers::{
    RenameLocation, RenameProvider, WorkspaceEdit, register_rename_provider,
};
use gpui_monaco::providers::{SelectionRangeProvider, register_selection_range_provider};
use gpui_monaco::text::PointUtf16;
use lsp_types as lsp;

use crate::client::LspClient;
use crate::convert;
use crate::rpc::Connection;

/// Every provider the bridge implements; registered once per feature the
/// server supports.
#[derive(Clone)]
pub(crate) struct LspFeatures {
    client: WeakEntity<LspClient>,
    completion_triggers: Vec<char>,
    signature_triggers: Vec<char>,
    signature_retriggers: Vec<char>,
    on_type_triggers: Vec<char>,
    /// The server answers `textDocument/prepareRename`.
    prepare_rename: bool,
    /// The server answers `completionItem/resolve`.
    resolve_completions: bool,
    /// The server answers `codeAction/resolve`.
    resolve_code_actions: bool,
    /// The server answers `codeLens/resolve`.
    resolve_code_lenses: bool,
    /// The server answers `documentLink/resolve`.
    resolve_links: bool,
    /// The code action kinds the server returns, if it says.
    code_action_kinds: Option<Vec<CodeActionKind>>,
    /// The server's semantic token types and modifiers.
    semantic_legend: SemanticTokensLegend,
    /// The server answers `textDocument/semanticTokens/full/delta`.
    semantic_delta: bool,
}

/// The semantic tokens options a server advertises.
fn semantic_options(capabilities: &lsp::ServerCapabilities) -> Option<&lsp::SemanticTokensOptions> {
    match capabilities.semantic_tokens_provider.as_ref()? {
        lsp::SemanticTokensServerCapabilities::SemanticTokensOptions(options) => Some(options),
        lsp::SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(options) => {
            Some(&options.semantic_tokens_options)
        }
    }
}

fn chars(strings: Option<&Vec<String>>) -> Vec<char> {
    strings
        .into_iter()
        .flatten()
        .filter_map(|s| s.chars().next())
        .collect()
}

fn supported<T>(capability: Option<&lsp::OneOf<bool, T>>) -> bool {
    matches!(
        capability,
        Some(lsp::OneOf::Left(true) | lsp::OneOf::Right(_))
    )
}

impl LspFeatures {
    pub(crate) fn new(
        client: WeakEntity<LspClient>,
        capabilities: &lsp::ServerCapabilities,
    ) -> Self {
        let completion = capabilities.completion_provider.as_ref();
        let signature = capabilities.signature_help_provider.as_ref();
        Self {
            client,
            completion_triggers: chars(completion.and_then(|c| c.trigger_characters.as_ref())),
            signature_triggers: chars(signature.and_then(|s| s.trigger_characters.as_ref())),
            signature_retriggers: chars(signature.and_then(|s| s.retrigger_characters.as_ref())),
            on_type_triggers: capabilities
                .document_on_type_formatting_provider
                .iter()
                .flat_map(|options| {
                    std::iter::once(&options.first_trigger_character)
                        .chain(options.more_trigger_character.iter().flatten())
                })
                .filter_map(|s| s.chars().next())
                .collect(),
            resolve_completions: capabilities
                .completion_provider
                .as_ref()
                .and_then(|c| c.resolve_provider)
                .unwrap_or(false),
            resolve_code_actions: matches!(
                &capabilities.code_action_provider,
                Some(lsp::CodeActionProviderCapability::Options(
                    lsp::CodeActionOptions {
                        resolve_provider: Some(true),
                        ..
                    }
                ))
            ),
            resolve_code_lenses: capabilities
                .code_lens_provider
                .as_ref()
                .and_then(|options| options.resolve_provider)
                .unwrap_or(false),
            resolve_links: capabilities
                .document_link_provider
                .as_ref()
                .and_then(|options| options.resolve_provider)
                .unwrap_or(false),
            code_action_kinds: match &capabilities.code_action_provider {
                Some(lsp::CodeActionProviderCapability::Options(options)) => {
                    options.code_action_kinds.as_ref().map(|kinds| {
                        kinds
                            .iter()
                            .map(|kind| CodeActionKind::new(kind.as_str().to_string()))
                            .collect()
                    })
                }
                _ => None,
            },
            semantic_legend: semantic_options(capabilities)
                .map(|options| SemanticTokensLegend {
                    token_types: options
                        .legend
                        .token_types
                        .iter()
                        .map(|t| t.as_str().to_string().into())
                        .collect(),
                    token_modifiers: options
                        .legend
                        .token_modifiers
                        .iter()
                        .map(|m| m.as_str().to_string().into())
                        .collect(),
                })
                .unwrap_or_default(),
            semantic_delta: matches!(
                semantic_options(capabilities).and_then(|o| o.full.as_ref()),
                Some(lsp::SemanticTokensFullOptions::Delta { delta: Some(true) })
            ),
            prepare_rename: matches!(
                &capabilities.rename_provider,
                Some(lsp::OneOf::Right(lsp::RenameOptions {
                    prepare_provider: Some(true),
                    ..
                }))
            ),
        }
    }

    pub(crate) fn register(
        &self,
        selector: LanguageSelector,
        capabilities: &lsp::ServerCapabilities,
        cx: &mut App,
    ) -> Vec<ProviderRegistration> {
        let mut registrations = Vec::new();
        if capabilities.completion_provider.is_some() {
            registrations.push(register_completion_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        let hover = matches!(
            capabilities.hover_provider,
            Some(
                lsp::HoverProviderCapability::Simple(true)
                    | lsp::HoverProviderCapability::Options(_)
            )
        );
        if hover {
            registrations.push(register_hover_provider(cx, selector.clone(), self.clone()));
        }
        if supported(capabilities.definition_provider.as_ref()) {
            registrations.push(register_definition_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities
            .declaration_provider
            .as_ref()
            .is_some_and(|c| !matches!(c, lsp::DeclarationCapability::Simple(false)))
        {
            registrations.push(register_declaration_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities
            .type_definition_provider
            .as_ref()
            .is_some_and(|c| !matches!(c, lsp::TypeDefinitionProviderCapability::Simple(false)))
        {
            registrations.push(register_type_definition_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities
            .implementation_provider
            .as_ref()
            .is_some_and(|c| !matches!(c, lsp::ImplementationProviderCapability::Simple(false)))
        {
            registrations.push(register_implementation_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        let code_actions = match &capabilities.code_action_provider {
            Some(lsp::CodeActionProviderCapability::Simple(enabled)) => *enabled,
            Some(lsp::CodeActionProviderCapability::Options(_)) => true,
            None => false,
        };
        if code_actions {
            registrations.push(register_code_action_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities
            .folding_range_provider
            .as_ref()
            .is_some_and(|c| !matches!(c, lsp::FoldingRangeProviderCapability::Simple(false)))
        {
            registrations.push(register_folding_range_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities
            .selection_range_provider
            .as_ref()
            .is_some_and(|c| !matches!(c, lsp::SelectionRangeProviderCapability::Simple(false)))
        {
            registrations.push(register_selection_range_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if let Some(options) = semantic_options(capabilities) {
            if matches!(
                options.full,
                Some(
                    lsp::SemanticTokensFullOptions::Bool(true)
                        | lsp::SemanticTokensFullOptions::Delta { .. }
                )
            ) {
                registrations.push(register_document_semantic_tokens_provider(
                    cx,
                    selector.clone(),
                    self.clone(),
                ));
            }
            if options.range == Some(true) {
                registrations.push(register_document_range_semantic_tokens_provider(
                    cx,
                    selector.clone(),
                    self.clone(),
                ));
            }
        }
        if supported(capabilities.document_symbol_provider.as_ref()) {
            registrations.push(register_document_symbol_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if supported(capabilities.inlay_hint_provider.as_ref()) {
            registrations.push(register_inlay_hints_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if supported(capabilities.rename_provider.as_ref()) {
            registrations.push(register_rename_provider(cx, selector.clone(), self.clone()));
        }
        if supported(capabilities.references_provider.as_ref()) {
            registrations.push(register_reference_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if supported(capabilities.document_highlight_provider.as_ref()) {
            registrations.push(register_document_highlight_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if supported(capabilities.document_formatting_provider.as_ref()) {
            registrations.push(register_document_formatting_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if supported(capabilities.document_range_formatting_provider.as_ref()) {
            registrations.push(register_document_range_formatting_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities.document_on_type_formatting_provider.is_some() {
            registrations.push(register_on_type_formatting_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities.code_lens_provider.is_some() {
            registrations.push(register_code_lens_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities.document_link_provider.is_some() {
            registrations.push(register_link_provider(cx, selector.clone(), self.clone()));
        }
        if capabilities
            .color_provider
            .as_ref()
            .is_some_and(|c| !matches!(c, lsp::ColorProviderCapability::Simple(false)))
        {
            registrations.push(register_color_provider(cx, selector.clone(), self.clone()));
        }
        if capabilities
            .linked_editing_range_provider
            .as_ref()
            .is_some_and(|c| !matches!(c, lsp::LinkedEditingRangeServerCapabilities::Simple(false)))
        {
            registrations.push(register_linked_editing_range_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if matches!(
            capabilities.call_hierarchy_provider,
            Some(
                lsp::CallHierarchyServerCapability::Simple(true)
                    | lsp::CallHierarchyServerCapability::Options(_)
            )
        ) {
            registrations.push(gpui_monaco::providers::register_call_hierarchy_provider(
                cx,
                selector.clone(),
                self.clone(),
            ));
        }
        if capabilities.signature_help_provider.is_some() {
            registrations.push(register_signature_help_provider(cx, selector, self.clone()));
        }
        // Not tied to a language, as in `vscode-languageclient`.
        if matches!(
            capabilities.workspace_symbol_provider,
            Some(lsp::OneOf::Left(true) | lsp::OneOf::Right(_))
        ) {
            registrations.push(gpui_monaco::providers::register_workspace_symbol_provider(
                cx,
                self.clone(),
            ));
        }
        registrations
    }

    /// The client the providers forward to.
    pub(crate) fn client(&self) -> &WeakEntity<LspClient> {
        &self.client
    }

    /// Syncs the document and returns what a request needs, or `None` when
    /// the model isn't attached to this server.
    fn prepare(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Option<(Connection, lsp::TextDocumentPositionParams)> {
        let uri = &context.resource;
        let client = self.client.upgrade()?;
        let (connection, text_document) =
            client.update(cx, |client, cx| client.prepare_request(uri, cx))?;
        Some((
            connection,
            lsp::TextDocumentPositionParams {
                text_document,
                position: convert::position(position),
            },
        ))
    }
}

impl CompletionProvider for LspFeatures {
    fn trigger_characters(&self) -> Vec<char> {
        self.completion_triggers.clone()
    }

    fn provide_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        trigger: CompletionTrigger,
        cx: &mut App,
    ) -> Task<Result<CompletionList>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(CompletionList::default()));
        };
        let context = match trigger {
            CompletionTrigger::Invoke => lsp::CompletionContext {
                trigger_kind: lsp::CompletionTriggerKind::INVOKED,
                trigger_character: None,
            },
            CompletionTrigger::TriggerCharacter(c) => lsp::CompletionContext {
                trigger_kind: lsp::CompletionTriggerKind::TRIGGER_CHARACTER,
                trigger_character: Some(c.to_string()),
            },
            CompletionTrigger::Incomplete => lsp::CompletionContext {
                trigger_kind: lsp::CompletionTriggerKind::TRIGGER_FOR_INCOMPLETE_COMPLETIONS,
                trigger_character: None,
            },
        };
        let request = connection.request::<lsp::request::Completion>(lsp::CompletionParams {
            text_document_position: position,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: Some(context),
        });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .map(convert::completion_list)
                .unwrap_or_default())
        })
    }

    fn resolve_completion_item(
        &self,
        item: &CompletionItem,
        cx: &mut App,
    ) -> Option<Task<Result<CompletionItem>>> {
        if !self.resolve_completions {
            return None;
        }
        let original: lsp::CompletionItem = serde_json::from_value(item.data.clone()?).ok()?;
        let client = self.client.upgrade()?;
        let connection = client.read(cx).connection().clone();
        let request = connection.request::<lsp::request::ResolveCompletionItem>(original);
        Some(
            cx.background_executor()
                .spawn(async move { Ok(convert::completion_item(request.await?)) }),
        )
    }
}

impl HoverProvider for LspFeatures {
    fn provide_hover(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(None));
        };
        let request = connection.request::<lsp::request::HoverRequest>(lsp::HoverParams {
            text_document_position_params: position,
            work_done_progress_params: Default::default(),
        });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .map(convert::hover)
                .filter(|hover| !hover.contents.is_empty()))
        })
    }
}

impl DefinitionProvider for LspFeatures {
    fn provide_definition(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<Result<Vec<Location>>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let queried = position.text_document.uri.as_str().to_string();
        let request =
            connection.request::<lsp::request::GotoDefinition>(lsp::GotoDefinitionParams {
                text_document_position_params: position,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .map(|response| convert::locations(response, &queried))
                .unwrap_or_default())
        })
    }
}

impl DocumentHighlightProvider for LspFeatures {
    fn provide_document_highlights(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<Result<Option<Vec<DocumentHighlight>>>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(None));
        };
        let request = connection.request::<lsp::request::DocumentHighlightRequest>(
            lsp::DocumentHighlightParams {
                text_document_position_params: position,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        );
        cx.background_executor().spawn(async move {
            Ok(request.await?.map(|highlights| {
                highlights
                    .into_iter()
                    .map(convert::document_highlight)
                    .collect()
            }))
        })
    }
}

impl SignatureHelpProvider for LspFeatures {
    fn trigger_characters(&self) -> Vec<char> {
        self.signature_triggers.clone()
    }

    fn retrigger_characters(&self) -> Vec<char> {
        self.signature_retriggers.clone()
    }

    fn provide_signature_help(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        trigger: SignatureHelpTrigger,
        cx: &mut App,
    ) -> Task<Result<Option<SignatureHelp>>> {
        // Without the editor's state, a retrigger character or a content
        // change means hints are showing.
        let is_retrigger = match &trigger {
            SignatureHelpTrigger::Invoke => false,
            SignatureHelpTrigger::TriggerCharacter(c) => self.signature_retriggers.contains(c),
            SignatureHelpTrigger::ContentChange => true,
        };
        let help_context = SignatureHelpContext {
            trigger,
            is_retrigger,
            active_signature_help: None,
        };
        self.provide_signature_help_with_context(context, position, &help_context, cx)
    }

    /// `monaco-languageclient`'s `asSignatureHelpContext`: the editor's
    /// `isRetrigger` and `activeSignatureHelp` (with the overload the user
    /// chose) go to the server as they are.
    fn provide_signature_help_with_context(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        help_context: &SignatureHelpContext,
        cx: &mut App,
    ) -> Task<Result<Option<SignatureHelp>>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(None));
        };
        let (trigger_kind, trigger_character) = match &help_context.trigger {
            SignatureHelpTrigger::Invoke => (lsp::SignatureHelpTriggerKind::INVOKED, None),
            SignatureHelpTrigger::TriggerCharacter(c) => (
                lsp::SignatureHelpTriggerKind::TRIGGER_CHARACTER,
                Some(c.to_string()),
            ),
            SignatureHelpTrigger::ContentChange => {
                (lsp::SignatureHelpTriggerKind::CONTENT_CHANGE, None)
            }
        };
        let request =
            connection.request::<lsp::request::SignatureHelpRequest>(lsp::SignatureHelpParams {
                context: Some(lsp::SignatureHelpContext {
                    trigger_kind,
                    trigger_character,
                    is_retrigger: help_context.is_retrigger,
                    active_signature_help: help_context
                        .active_signature_help
                        .as_ref()
                        .map(convert::lsp_signature_help),
                }),
                text_document_position_params: position,
                work_done_progress_params: Default::default(),
            });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .map(convert::signature_help)
                .filter(|help| !help.signatures.is_empty()))
        })
    }
}

fn formatting_options(options: FormattingOptions) -> lsp::FormattingOptions {
    lsp::FormattingOptions {
        tab_size: options.tab_size,
        insert_spaces: options.insert_spaces,
        ..Default::default()
    }
}

fn text_edits(edits: Option<Vec<lsp::TextEdit>>) -> Vec<TextEdit> {
    edits
        .unwrap_or_default()
        .into_iter()
        .map(|edit| (convert::range(edit.range), edit.new_text))
        .collect()
}

impl DocumentFormattingProvider for LspFeatures {
    fn provide_document_formatting_edits(
        &self,
        context: &ProviderContext,
        options: FormattingOptions,
        cx: &mut App,
    ) -> Task<Result<FormattingEdits>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::zero(), cx) else {
            return Task::ready(Ok(FormattingEdits::default()));
        };
        let request =
            connection.request::<lsp::request::Formatting>(lsp::DocumentFormattingParams {
                text_document: position.text_document,
                options: formatting_options(options),
                work_done_progress_params: Default::default(),
            });
        cx.background_executor()
            .spawn(async move { Ok(text_edits(request.await?).into()) })
    }
}

impl DocumentRangeFormattingProvider for LspFeatures {
    fn provide_document_range_formatting_edits(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        options: FormattingOptions,
        cx: &mut App,
    ) -> Task<Result<FormattingEdits>> {
        let Some((connection, position)) = self.prepare(context, range.start, cx) else {
            return Task::ready(Ok(FormattingEdits::default()));
        };
        let request = connection.request::<lsp::request::RangeFormatting>(
            lsp::DocumentRangeFormattingParams {
                text_document: position.text_document,
                range: convert::lsp_range(&range),
                options: formatting_options(options),
                work_done_progress_params: Default::default(),
            },
        );
        cx.background_executor()
            .spawn(async move { Ok(text_edits(request.await?).into()) })
    }
}

impl OnTypeFormattingProvider for LspFeatures {
    fn auto_format_trigger_characters(&self) -> Vec<char> {
        self.on_type_triggers.clone()
    }

    fn provide_on_type_formatting_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        ch: char,
        options: FormattingOptions,
        cx: &mut App,
    ) -> Task<Result<FormattingEdits>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(FormattingEdits::default()));
        };
        let request = connection.request::<lsp::request::OnTypeFormatting>(
            lsp::DocumentOnTypeFormattingParams {
                text_document_position: position,
                ch: ch.to_string(),
                options: formatting_options(options),
            },
        );
        cx.background_executor()
            .spawn(async move { Ok(text_edits(request.await?).into()) })
    }
}

/// Sends a `GotoDefinition`-shaped request (`R`) and converts its locations.
fn locations_request<R>(
    features: &LspFeatures,
    context: &ProviderContext,
    position: PointUtf16,
    cx: &mut App,
) -> Task<Result<Vec<Location>>>
where
    R: lsp::request::Request<
            Params = lsp::GotoDefinitionParams,
            Result = Option<lsp::GotoDefinitionResponse>,
        >,
{
    let Some((connection, position)) = features.prepare(context, position, cx) else {
        return Task::ready(Ok(Vec::new()));
    };
    let queried = position.text_document.uri.as_str().to_string();
    let request = connection.request::<R>(lsp::GotoDefinitionParams {
        text_document_position_params: position,
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    });
    cx.background_executor().spawn(async move {
        Ok(request
            .await?
            .map(|response| convert::locations(response, &queried))
            .unwrap_or_default())
    })
}

impl DeclarationProvider for LspFeatures {
    fn provide_declaration(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<Result<Vec<Location>>> {
        locations_request::<lsp::request::GotoDeclaration>(self, context, position, cx)
    }
}

impl TypeDefinitionProvider for LspFeatures {
    fn provide_type_definition(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<Result<Vec<Location>>> {
        locations_request::<lsp::request::GotoTypeDefinition>(self, context, position, cx)
    }
}

impl ImplementationProvider for LspFeatures {
    fn provide_implementation(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<Result<Vec<Location>>> {
        locations_request::<lsp::request::GotoImplementation>(self, context, position, cx)
    }
}

impl ReferenceProvider for LspFeatures {
    fn provide_references(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        reference_context: ReferenceContext,
        cx: &mut App,
    ) -> Task<Result<Vec<Location>>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let queried = position.text_document.uri.as_str().to_string();
        let request = connection.request::<lsp::request::References>(lsp::ReferenceParams {
            text_document_position: position,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp::ReferenceContext {
                include_declaration: reference_context.include_declaration,
            },
        });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .map(|locations| {
                    convert::locations(lsp::GotoDefinitionResponse::Array(locations), &queried)
                })
                .unwrap_or_default())
        })
    }
}

impl RenameProvider for LspFeatures {
    fn resolve_rename_location(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Option<Task<Result<Option<RenameLocation>>>> {
        if !self.prepare_rename {
            return None;
        }
        let Some((connection, params)) = self.prepare(context, position, cx) else {
            return Some(Task::ready(Ok(None)));
        };
        let snapshot = context.snapshot.clone();
        let request = connection.request::<lsp::request::PrepareRenameRequest>(params);
        Some(cx.background_executor().spawn(async move {
            let range_text = |range: lsp::Range| {
                let range = convert::range(range);
                let rope = snapshot.rope();
                let text = rope.text_for_range(
                    rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end),
                );
                (range, text)
            };
            match request.await? {
                // monaco-languageclient's message for a null result.
                None => Err(anyhow::anyhow!("The element can't be renamed.")),
                Some(lsp::PrepareRenameResponse::Range(range)) => {
                    let (range, text) = range_text(range);
                    Ok(Some(RenameLocation {
                        range,
                        text: text.into(),
                    }))
                }
                Some(lsp::PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) => {
                    Ok(Some(RenameLocation {
                        range: convert::range(range),
                        text: placeholder.into(),
                    }))
                }
                // The word at the cursor.
                Some(lsp::PrepareRenameResponse::DefaultBehavior { .. }) => Ok(None),
            }
        }))
    }

    fn provide_rename_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        new_name: &str,
        cx: &mut App,
    ) -> Task<Result<Option<WorkspaceEdit>>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(None));
        };
        let request = connection.request::<lsp::request::Rename>(lsp::RenameParams {
            text_document_position: position,
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        });
        cx.background_executor().spawn(async move {
            match request.await? {
                Some(edit) => convert::workspace_edit(edit).map(Some),
                None => Ok(None),
            }
        })
    }
}

impl CodeActionProvider for LspFeatures {
    fn provided_kinds(&self) -> Option<Vec<CodeActionKind>> {
        self.code_action_kinds.clone()
    }

    fn provide_code_actions(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        action_context: &CodeActionContext,
        cx: &mut App,
    ) -> Task<Result<Vec<CodeAction>>> {
        let Some((connection, position)) = self.prepare(context, range.start, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let diagnostics = self.client.upgrade().map_or_else(Vec::new, |client| {
            client
                .read(cx)
                .lsp_diagnostics(&context.resource, &action_context.markers)
        });
        let request =
            connection.request::<lsp::request::CodeActionRequest>(lsp::CodeActionParams {
                text_document: position.text_document,
                range: convert::lsp_range(&range),
                context: lsp::CodeActionContext {
                    diagnostics,
                    only: action_context
                        .only
                        .as_ref()
                        .map(|kind| vec![lsp::CodeActionKind::from(kind.0.to_string())]),
                    trigger_kind: Some(match action_context.trigger {
                        CodeActionTrigger::Invoke => lsp::CodeActionTriggerKind::INVOKED,
                        CodeActionTrigger::Auto => lsp::CodeActionTriggerKind::AUTOMATIC,
                    }),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            });
        cx.background_executor().spawn(async move {
            request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(convert::code_action)
                .collect()
        })
    }

    fn resolve_code_action(
        &self,
        action: &CodeAction,
        cx: &mut App,
    ) -> Option<Task<Result<CodeAction>>> {
        if !self.resolve_code_actions || action.edit.is_some() {
            return None;
        }
        let original: lsp::CodeAction = serde_json::from_value(action.data.clone()?).ok()?;
        let client = self.client.upgrade()?;
        let connection = client.read(cx).connection().clone();
        let request = connection.request::<lsp::request::CodeActionResolveRequest>(original);
        Some(cx.background_executor().spawn(async move {
            convert::code_action(lsp::CodeActionOrCommand::CodeAction(request.await?))
        }))
    }
}

impl InlayHintsProvider for LspFeatures {
    fn provide_inlay_hints(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        cx: &mut App,
    ) -> Task<Result<Vec<InlayHint>>> {
        let Some((connection, position)) = self.prepare(context, range.start, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let queried = position.text_document.uri.as_str().to_string();
        let request = connection.request::<lsp::request::InlayHintRequest>(lsp::InlayHintParams {
            text_document: position.text_document,
            range: convert::lsp_range(&range),
            work_done_progress_params: Default::default(),
        });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(|hint| convert::inlay_hint(hint, &queried))
                .collect())
        })
    }
}

impl FoldingRangeProvider for LspFeatures {
    fn provide_folding_ranges(
        &self,
        context: &ProviderContext,
        _: &FoldingContext,
        cx: &mut App,
    ) -> Task<Result<Option<Vec<FoldingRange>>>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::zero(), cx) else {
            return Task::ready(Ok(None));
        };
        let request =
            connection.request::<lsp::request::FoldingRangeRequest>(lsp::FoldingRangeParams {
                text_document: position.text_document,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            });
        cx.background_executor().spawn(async move {
            Ok(request.await?.map(|ranges| {
                ranges
                    .into_iter()
                    .map(|range| FoldingRange {
                        start_row: range.start_line,
                        end_row: range.end_line,
                        kind: range.kind.map(|kind| match kind {
                            lsp::FoldingRangeKind::Comment => FoldingRangeKind::Comment,
                            lsp::FoldingRangeKind::Imports => FoldingRangeKind::Imports,
                            lsp::FoldingRangeKind::Region => FoldingRangeKind::Region,
                        }),
                    })
                    .collect()
            }))
        })
    }
}

impl SelectionRangeProvider for LspFeatures {
    fn provide_selection_ranges(
        &self,
        context: &ProviderContext,
        positions: &[PointUtf16],
        cx: &mut App,
    ) -> Task<Result<Vec<Vec<Range<PointUtf16>>>>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::zero(), cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request =
            connection.request::<lsp::request::SelectionRangeRequest>(lsp::SelectionRangeParams {
                text_document: position.text_document,
                positions: positions.iter().map(|p| convert::position(*p)).collect(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(|selection| {
                    // Each range with its parents, outward.
                    let mut ranges = Vec::new();
                    let mut current = Some(Box::new(selection));
                    while let Some(range) = current {
                        ranges.push(convert::range(range.range));
                        current = range.parent;
                    }
                    ranges
                })
                .collect())
        })
    }
}

/// LSP tokens as Monaco's flat 5-number encoding.
fn flatten_tokens(tokens: &[lsp::SemanticToken]) -> Vec<u32> {
    tokens
        .iter()
        .flat_map(|t| {
            [
                t.delta_line,
                t.delta_start,
                t.length,
                t.token_type,
                t.token_modifiers_bitset,
            ]
        })
        .collect()
}

fn semantic_tokens(tokens: lsp::SemanticTokens) -> SemanticTokens {
    SemanticTokens {
        result_id: tokens.result_id.map(Into::into),
        data: flatten_tokens(&tokens.data),
    }
}

impl DocumentSemanticTokensProvider for LspFeatures {
    fn legend(&self) -> SemanticTokensLegend {
        self.semantic_legend.clone()
    }

    fn provide_document_semantic_tokens(
        &self,
        context: &ProviderContext,
        last_result_id: Option<gpui::SharedString>,
        cx: &mut App,
    ) -> Task<Result<Option<SemanticTokensResult>>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::default(), cx) else {
            return Task::ready(Ok(None));
        };
        let text_document = position.text_document;
        match last_result_id.filter(|_| self.semantic_delta) {
            Some(previous_result_id) => {
                let request = connection.request::<lsp::request::SemanticTokensFullDeltaRequest>(
                    lsp::SemanticTokensDeltaParams {
                        text_document,
                        previous_result_id: previous_result_id.to_string(),
                        work_done_progress_params: Default::default(),
                        partial_result_params: Default::default(),
                    },
                );
                cx.background_executor().spawn(async move {
                    Ok(request.await?.map(|result| match result {
                        lsp::SemanticTokensFullDeltaResult::Tokens(tokens) => {
                            SemanticTokensResult::Tokens(semantic_tokens(tokens))
                        }
                        lsp::SemanticTokensFullDeltaResult::TokensDelta(delta) => {
                            SemanticTokensResult::Edits(SemanticTokensEdits {
                                result_id: delta.result_id.map(Into::into),
                                // LSP counts edits in numbers, like Monaco.
                                edits: delta
                                    .edits
                                    .into_iter()
                                    .map(|edit| SemanticTokensEdit {
                                        start: edit.start as usize,
                                        delete_count: edit.delete_count as usize,
                                        data: flatten_tokens(&edit.data.unwrap_or_default()),
                                    })
                                    .collect(),
                            })
                        }
                        lsp::SemanticTokensFullDeltaResult::PartialTokensDelta { edits } => {
                            SemanticTokensResult::Edits(SemanticTokensEdits {
                                result_id: None,
                                edits: edits
                                    .into_iter()
                                    .map(|edit| SemanticTokensEdit {
                                        start: edit.start as usize,
                                        delete_count: edit.delete_count as usize,
                                        data: flatten_tokens(&edit.data.unwrap_or_default()),
                                    })
                                    .collect(),
                            })
                        }
                    }))
                })
            }
            None => {
                let request = connection.request::<lsp::request::SemanticTokensFullRequest>(
                    lsp::SemanticTokensParams {
                        text_document,
                        work_done_progress_params: Default::default(),
                        partial_result_params: Default::default(),
                    },
                );
                cx.background_executor().spawn(async move {
                    Ok(request.await?.map(|result| match result {
                        lsp::SemanticTokensResult::Tokens(tokens) => {
                            SemanticTokensResult::Tokens(semantic_tokens(tokens))
                        }
                        lsp::SemanticTokensResult::Partial(partial) => {
                            SemanticTokensResult::Tokens(SemanticTokens {
                                result_id: None,
                                data: flatten_tokens(&partial.data),
                            })
                        }
                    }))
                })
            }
        }
    }
}

impl DocumentRangeSemanticTokensProvider for LspFeatures {
    fn legend(&self) -> SemanticTokensLegend {
        self.semantic_legend.clone()
    }

    fn provide_document_range_semantic_tokens(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        cx: &mut App,
    ) -> Task<Result<Option<SemanticTokens>>> {
        let Some((connection, position)) = self.prepare(context, range.start, cx) else {
            return Task::ready(Ok(None));
        };
        let request = connection.request::<lsp::request::SemanticTokensRangeRequest>(
            lsp::SemanticTokensRangeParams {
                text_document: position.text_document,
                range: convert::lsp_range(&range),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        );
        cx.background_executor().spawn(async move {
            Ok(request.await?.map(|result| match result {
                lsp::SemanticTokensRangeResult::Tokens(tokens) => semantic_tokens(tokens),
                lsp::SemanticTokensRangeResult::Partial(partial) => SemanticTokens {
                    result_id: None,
                    data: flatten_tokens(&partial.data),
                },
            }))
        })
    }
}

impl DocumentSymbolProvider for LspFeatures {
    fn provide_document_symbols(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<Result<Vec<DocumentSymbol>>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::default(), cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request =
            connection.request::<lsp::request::DocumentSymbolRequest>(lsp::DocumentSymbolParams {
                text_document: position.text_document,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .map(convert::document_symbols)
                .unwrap_or_default())
        })
    }
}

/// `vscode-languageclient`'s `CodeLensFeature`: `textDocument/codeLens`,
/// and `codeLens/resolve` for lenses without a command when the server
/// resolves them.
impl CodeLensProvider for LspFeatures {
    fn provide_code_lenses(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<Result<Vec<CodeLens>>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::zero(), cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request = connection.request::<lsp::request::CodeLensRequest>(lsp::CodeLensParams {
            text_document: position.text_document,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        });
        cx.background_executor().spawn(async move {
            request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(convert::code_lens)
                .collect()
        })
    }

    fn resolve_code_lens(
        &self,
        _context: &ProviderContext,
        lens: &CodeLens,
        cx: &mut App,
    ) -> Option<Task<Result<CodeLens>>> {
        if !self.resolve_code_lenses {
            return None;
        }
        let original: lsp::CodeLens = serde_json::from_value(lens.data.clone()?).ok()?;
        let client = self.client.upgrade()?;
        let connection = client.read(cx).connection().clone();
        let request = connection.request::<lsp::request::CodeLensResolve>(original);
        Some(
            cx.background_executor()
                .spawn(async move { convert::code_lens(request.await?) }),
        )
    }
}

/// `vscode-languageclient`'s `DocumentLinkFeature`:
/// `textDocument/documentLink`, and `documentLink/resolve` for links
/// without a target when the server resolves them.
impl LinkProvider for LspFeatures {
    fn provide_links(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<Result<Vec<DocumentLink>>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::zero(), cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request =
            connection.request::<lsp::request::DocumentLinkRequest>(lsp::DocumentLinkParams {
                text_document: position.text_document,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            });
        cx.background_executor().spawn(async move {
            request
                .await?
                .unwrap_or_default()
                .into_iter()
                .map(convert::document_link)
                .collect()
        })
    }

    fn resolve_link(
        &self,
        link: &DocumentLink,
        cx: &mut App,
    ) -> Option<Task<Result<DocumentLink>>> {
        if !self.resolve_links {
            return None;
        }
        let original: lsp::DocumentLink = serde_json::from_value(link.data.clone()?).ok()?;
        let client = self.client.upgrade()?;
        let connection = client.read(cx).connection().clone();
        let request = connection.request::<lsp::request::DocumentLinkResolve>(original);
        Some(
            cx.background_executor()
                .spawn(async move { convert::document_link(request.await?) }),
        )
    }
}

/// `vscode-languageclient`'s `ColorProviderFeature`:
/// `textDocument/documentColor` and `textDocument/colorPresentation`.
impl DocumentColorProvider for LspFeatures {
    fn provide_document_colors(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<Result<Vec<ColorInformation>>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::zero(), cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request = connection.request::<lsp::request::DocumentColor>(lsp::DocumentColorParams {
            text_document: position.text_document,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        });
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .into_iter()
                .map(convert::color_information)
                .collect())
        })
    }

    fn provide_color_presentations(
        &self,
        context: &ProviderContext,
        color: &ColorInformation,
        cx: &mut App,
    ) -> Task<Result<Vec<ColorPresentation>>> {
        let Some((connection, position)) = self.prepare(context, color.range.start, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request = connection.request::<lsp::request::ColorPresentationRequest>(
            lsp::ColorPresentationParams {
                text_document: position.text_document,
                color: convert::lsp_color(&color.color),
                range: convert::lsp_range(&color.range),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        );
        cx.background_executor().spawn(async move {
            Ok(request
                .await?
                .into_iter()
                .map(convert::color_presentation)
                .collect())
        })
    }
}

/// `vscode-languageclient`'s `LinkedEditingFeature`:
/// `textDocument/linkedEditingRange`.
impl LinkedEditingRangeProvider for LspFeatures {
    fn provide_linked_editing_ranges(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<Result<Option<LinkedEditingRanges>>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(None));
        };
        let request =
            connection.request::<lsp::request::LinkedEditingRange>(lsp::LinkedEditingRangeParams {
                text_document_position_params: position,
                work_done_progress_params: Default::default(),
            });
        cx.background_executor()
            .spawn(async move { Ok(request.await?.map(convert::linked_editing_ranges)) })
    }
}
