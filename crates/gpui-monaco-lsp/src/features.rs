//! gpui-monaco providers that forward to a language server.

use anyhow::Result;
use gpui::{App, Task, WeakEntity};
use std::ops::Range;

use gpui_monaco::providers::{
    CompletionList, CompletionProvider, CompletionTrigger, DefinitionProvider,
    DocumentFormattingProvider, DocumentHighlight, DocumentHighlightProvider,
    DocumentRangeFormattingProvider, FormattingOptions, Hover, HoverProvider, LanguageSelector,
    Location, OnTypeFormattingProvider, ProviderContext, ProviderRegistration, SignatureHelp,
    SignatureHelpProvider, SignatureHelpTrigger, TextEdit, register_completion_provider,
    register_definition_provider, register_document_formatting_provider,
    register_document_highlight_provider, register_document_range_formatting_provider,
    register_hover_provider, register_on_type_formatting_provider,
    register_signature_help_provider,
};
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
        if capabilities.signature_help_provider.is_some() {
            registrations.push(register_signature_help_provider(cx, selector, self.clone()));
        }
        registrations
    }

    /// Syncs the document and returns what a request needs, or `None` when
    /// the model isn't attached to this server.
    fn prepare(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Option<(Connection, lsp::TextDocumentPositionParams)> {
        let uri = context.uri.as_ref()?;
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
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(None));
        };
        let (trigger_kind, trigger_character, is_retrigger) = match trigger {
            SignatureHelpTrigger::Invoke => (lsp::SignatureHelpTriggerKind::INVOKED, None, false),
            SignatureHelpTrigger::TriggerCharacter(c) => (
                lsp::SignatureHelpTriggerKind::TRIGGER_CHARACTER,
                Some(c.to_string()),
                self.signature_retriggers.contains(&c),
            ),
            SignatureHelpTrigger::ContentChange => {
                (lsp::SignatureHelpTriggerKind::CONTENT_CHANGE, None, true)
            }
        };
        let request =
            connection.request::<lsp::request::SignatureHelpRequest>(lsp::SignatureHelpParams {
                context: Some(lsp::SignatureHelpContext {
                    trigger_kind,
                    trigger_character,
                    is_retrigger,
                    active_signature_help: None,
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
    ) -> Task<Result<Vec<TextEdit>>> {
        let Some((connection, position)) = self.prepare(context, PointUtf16::zero(), cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request =
            connection.request::<lsp::request::Formatting>(lsp::DocumentFormattingParams {
                text_document: position.text_document,
                options: formatting_options(options),
                work_done_progress_params: Default::default(),
            });
        cx.background_executor()
            .spawn(async move { Ok(text_edits(request.await?)) })
    }
}

impl DocumentRangeFormattingProvider for LspFeatures {
    fn provide_document_range_formatting_edits(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        options: FormattingOptions,
        cx: &mut App,
    ) -> Task<Result<Vec<TextEdit>>> {
        let Some((connection, position)) = self.prepare(context, range.start, cx) else {
            return Task::ready(Ok(Vec::new()));
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
            .spawn(async move { Ok(text_edits(request.await?)) })
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
    ) -> Task<Result<Vec<TextEdit>>> {
        let Some((connection, position)) = self.prepare(context, position, cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let request = connection.request::<lsp::request::OnTypeFormatting>(
            lsp::DocumentOnTypeFormattingParams {
                text_document_position: position,
                ch: ch.to_string(),
                options: formatting_options(options),
            },
        );
        cx.background_executor()
            .spawn(async move { Ok(text_edits(request.await?)) })
    }
}
