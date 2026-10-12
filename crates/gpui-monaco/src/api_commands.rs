//! Monaco's internal API commands that ask a model's providers and return
//! what they answer (`_executeLinkProvider`,
//! `_executeSignatureHelpProvider`, `_executeSelectionRangeProvider`,
//! `_executeCompletionItemProvider`, `_executeFoldingRangeProvider`,
//! `_provideDocumentSemanticTokens` and the rest). Monaco runs them through
//! `commandService.executeCommand` with a uri; here they're functions
//! taking the model, since commands return nothing (see
//! [`crate::execute_command`]).

use std::ops::Range;

use gpui::{App, Entity, Task};
use text::PointUtf16;

use crate::TextModel;
use crate::providers::{
    self, CompletionItem, CompletionTrigger, DocumentLink, SemanticTokens, SemanticTokensLegend,
    SemanticTokensResult, SignatureHelp, SignatureHelpContext, SignatureHelpTrigger,
};
use crate::providers::{ProviderContext, ProviderTarget};

/// What the providers are looked up and asked with.
fn target(model: &Entity<TextModel>, cx: &App) -> (ProviderTarget, ProviderContext) {
    let model = model.read(cx);
    (model.into(), model.provider_context())
}

/// The model's links from every link provider, the first `resolve_count`
/// of them resolved (Monaco `_executeLinkProvider`).
pub fn execute_link_provider(
    model: &Entity<TextModel>,
    resolve_count: usize,
    cx: &mut App,
) -> Task<Vec<DocumentLink>> {
    let (model, context) = target(model, cx);
    let providers = providers::link_providers(cx, model.clone());
    let requests: Vec<_> = providers
        .iter()
        .map(|provider| provider.provide_links(&context, cx))
        .collect();
    cx.spawn(async move |cx| {
        let mut links: Vec<(usize, DocumentLink)> = Vec::new();
        for (ix, request) in requests.into_iter().enumerate() {
            if let Ok(found) = request.await {
                links.extend(found.into_iter().map(|link| (ix, link)));
            }
        }
        let mut resolved = Vec::with_capacity(links.len());
        for (n, (ix, link)) in links.into_iter().enumerate() {
            let resolve = (n < resolve_count && link.url.is_none())
                .then(|| cx.update(|cx| providers[ix].resolve_link(&link, cx)))
                .flatten();
            match resolve {
                Some(task) => resolved.push(task.await.unwrap_or(link)),
                None => resolved.push(link),
            }
        }
        resolved
    })
}

/// The first signature help a provider gives at `position` (Monaco
/// `_executeSignatureHelpProvider`), as if typed `trigger_character` or
/// invoked.
pub fn execute_signature_help_provider(
    model: &Entity<TextModel>,
    position: PointUtf16,
    trigger_character: Option<char>,
    cx: &mut App,
) -> Task<Option<SignatureHelp>> {
    let (model, context) = target(model, cx);
    let help_context = SignatureHelpContext {
        trigger: match trigger_character {
            Some(ch) => SignatureHelpTrigger::TriggerCharacter(ch),
            None => SignatureHelpTrigger::Invoke,
        },
        is_retrigger: false,
        active_signature_help: None,
    };
    let requests: Vec<_> = providers::signature_help_providers(cx, model)
        .iter()
        .map(|provider| {
            provider.provide_signature_help_with_context(&context, position, &help_context, cx)
        })
        .collect();
    cx.background_executor().spawn(async move {
        for request in requests {
            if let Ok(Some(help)) = request.await
                && !help.signatures.is_empty()
            {
                return Some(help);
            }
        }
        None
    })
}

/// The model's folding ranges as an editor with `options` computes them
/// (Monaco `_executeFoldingRangeProvider`): none with `folding` off, else
/// from the folding range providers with `folding_strategy: Auto`, then
/// the syntax tree, then indentation, keeping at most
/// `folding_maximum_regions`.
pub fn execute_folding_range_provider(
    model: &Entity<TextModel>,
    options: &crate::options::EditorOptions,
    cx: &mut App,
) -> Task<Vec<providers::FoldingRange>> {
    if !options.folding {
        return Task::ready(Vec::new());
    }
    let (snapshot, _, requests, compute) = crate::editor::fold_inputs(
        model,
        options.folding_strategy,
        options.folding_maximum_regions,
        cx,
    );
    cx.background_executor().spawn(async move {
        let line_count = snapshot.rope().line_count();
        let provided = crate::editor::collect_provided_folding_ranges(requests, line_count).await;
        compute(provided)
            .0
            .into_iter()
            .map(|range| providers::FoldingRange {
                start_row: range.start_row,
                end_row: range.end_row,
                kind: range.kind,
            })
            .collect()
    })
}

/// The selection ranges at each of `positions` from the first provider
/// that answers (Monaco `_executeSelectionRangeProvider`).
pub fn execute_selection_range_provider(
    model: &Entity<TextModel>,
    positions: Vec<PointUtf16>,
    cx: &mut App,
) -> Task<Vec<Vec<Range<PointUtf16>>>> {
    let (model, context) = target(model, cx);
    let requests: Vec<_> = providers::selection_range_providers(cx, model)
        .iter()
        .map(|provider| provider.provide_selection_ranges(&context, &positions, cx))
        .collect();
    cx.background_executor().spawn(async move {
        for request in requests {
            if let Ok(ranges) = request.await
                && !ranges.is_empty()
            {
                return ranges;
            }
        }
        Vec::new()
    })
}

/// Every completion provider's items at `position` (Monaco
/// `_executeCompletionItemProvider`), as if typed `trigger_character` or
/// invoked; the first `max_items_to_resolve` are resolved.
pub fn execute_completion_item_provider(
    model: &Entity<TextModel>,
    position: PointUtf16,
    trigger_character: Option<char>,
    max_items_to_resolve: usize,
    cx: &mut App,
) -> Task<Vec<CompletionItem>> {
    let (model, context) = target(model, cx);
    let trigger = match trigger_character {
        Some(ch) => CompletionTrigger::TriggerCharacter(ch),
        None => CompletionTrigger::Invoke,
    };
    let providers = providers::completion_providers(cx, model);
    let requests: Vec<_> = providers
        .iter()
        .map(|provider| provider.provide_completions(&context, position, trigger.clone(), cx))
        .collect();
    cx.spawn(async move |cx| {
        let mut items: Vec<(usize, CompletionItem)> = Vec::new();
        for (ix, request) in requests.into_iter().enumerate() {
            if let Ok(list) = request.await {
                items.extend(list.items.into_iter().map(|item| (ix, item)));
            }
        }
        let mut resolved = Vec::with_capacity(items.len());
        for (n, (ix, item)) in items.into_iter().enumerate() {
            let resolve = (n < max_items_to_resolve)
                .then(|| cx.update(|cx| providers[ix].resolve_completion_item(&item, cx)))
                .flatten();
            match resolve {
                Some(task) => resolved.push(task.await.unwrap_or(item)),
                None => resolved.push(item),
            }
        }
        resolved
    })
}

/// The best document semantic tokens provider's legend (Monaco
/// `_provideDocumentSemanticTokensLegend`).
pub fn provide_document_semantic_tokens_legend(
    model: &Entity<TextModel>,
    cx: &App,
) -> Option<SemanticTokensLegend> {
    providers::document_semantic_tokens_providers(cx, model.read(cx))
        .first()
        .map(|provider| provider.legend())
}

/// The best document semantic tokens provider's tokens (Monaco
/// `_provideDocumentSemanticTokens`), whole (never edits).
pub fn provide_document_semantic_tokens(
    model: &Entity<TextModel>,
    cx: &mut App,
) -> Task<Option<SemanticTokens>> {
    let (model, context) = target(model, cx);
    let Some(provider) = providers::document_semantic_tokens_providers(cx, model)
        .into_iter()
        .next()
    else {
        return Task::ready(None);
    };
    let request = provider.provide_document_semantic_tokens(&context, None, cx);
    cx.background_executor().spawn(async move {
        match request.await {
            Ok(Some(SemanticTokensResult::Tokens(tokens))) => Some(tokens),
            _ => None,
        }
    })
}

/// The best range semantic tokens provider's legend (Monaco
/// `_provideDocumentRangeSemanticTokensLegend`).
pub fn provide_document_range_semantic_tokens_legend(
    model: &Entity<TextModel>,
    cx: &App,
) -> Option<SemanticTokensLegend> {
    providers::document_range_semantic_tokens_providers(cx, model.read(cx))
        .first()
        .map(|provider| provider.legend())
}

/// The best range semantic tokens provider's tokens in `range` (Monaco
/// `_provideDocumentRangeSemanticTokens`).
pub fn provide_document_range_semantic_tokens(
    model: &Entity<TextModel>,
    range: Range<PointUtf16>,
    cx: &mut App,
) -> Task<Option<SemanticTokens>> {
    let (model, context) = target(model, cx);
    let Some(provider) = providers::document_range_semantic_tokens_providers(cx, model)
        .into_iter()
        .next()
    else {
        return Task::ready(None);
    };
    let request = provider.provide_document_range_semantic_tokens(&context, range, cx);
    cx.background_executor()
        .spawn(async move { request.await.ok().flatten() })
}
