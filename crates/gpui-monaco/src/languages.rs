//! Monaco's `monaco.languages` extras: `setLanguageConfiguration` and
//! `setMonarchTokensProvider` at runtime, `onLanguage`, and guessing a
//! model's language from its uri and first line.
//!
//! Monaco's `setColorMap` and `getEncodedLanguageId` have no equivalent:
//! they serve encoded token metadata (language and color indices packed
//! into numbers, from `EncodedTokensProvider`s), while highlighters here
//! give named tokens that the theme resolves by name.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, Global, SharedString};
use syntax::{LanguageConfiguration, MonarchHighlighter, MonarchTokenizer};

use crate::model::language_registry;
use crate::registry::Disposable;

/// An `on_language` callback.
type LanguageCallback = Box<dyn FnOnce(&mut App)>;

/// Languages already requested by a model, and `on_language` callbacks
/// waiting for theirs.
#[derive(Default)]
struct RequestedLanguages {
    seen: HashSet<SharedString>,
    waiting: HashMap<SharedString, Vec<LanguageCallback>>,
}

impl Global for RequestedLanguages {}

/// Replaces the editing configuration (brackets, comments, auto-closing
/// pairs, indentation rules, …) of the registered language `language_id`
/// at runtime (Monaco `languages.setLanguageConfiguration`). Every model of
/// that language picks it up and emits
/// [`TextModelEvent::LanguageConfigurationChanged`](crate::TextModelEvent::LanguageConfigurationChanged).
/// Returns whether the language is registered.
pub fn set_language_configuration(
    cx: &mut App,
    language_id: &str,
    config: LanguageConfiguration,
) -> bool {
    if !language_registry(cx).set_configuration(language_id, config) {
        return false;
    }
    for model in crate::registry::models(cx) {
        model.update(cx, |model, cx| {
            if model.language_id().is_some_and(|id| id == language_id) {
                model.refresh_language(cx);
            }
        });
    }
    true
}

/// Highlights the registered language `language_id` with a Monarch grammar
/// (Monaco `languages.setMonarchTokensProvider`): the JSON form of
/// Monaco's `IMonarchLanguage`, regexes as strings (see
/// [`syntax::monarch`] for the regex dialect). It replaces the language's
/// tree-sitter or custom highlighting; models of the language, and of
/// Monarch languages embedding others, re-tokenize. [`colorize`] and
/// [`tokenize`] use it too.
///
/// Fails, as Monaco throws, for an unknown language or an invalid grammar.
/// Disposing the result restores the previous highlighting, unless another
/// provider replaced this one since.
///
/// [`colorize`]: crate::colorize::colorize
/// [`tokenize`]: crate::colorize::tokenize
pub fn set_monarch_tokens_provider(
    cx: &mut App,
    language_id: &str,
    grammar: &serde_json::Value,
) -> anyhow::Result<Disposable> {
    let registry = language_registry(cx);
    let Some(language) = registry.language_for_id(language_id) else {
        anyhow::bail!("Cannot set tokens provider for unknown language {language_id}");
    };
    let tokenizer = Arc::new(MonarchTokenizer::new(language_id, grammar)?);
    let factory = MonarchHighlighter::factory(tokenizer);
    let previous = language.highlighter.clone();
    registry.set_highlighter(language_id, Some(factory.clone()));
    refresh_highlighting(cx, language_id);
    let language_id = language_id.to_string();
    Ok(Disposable::new(move |cx| {
        let registry = language_registry(cx);
        let ours = registry
            .language_for_id(&language_id)
            .and_then(|language| language.highlighter.clone())
            .is_some_and(|current| Arc::ptr_eq(&current, &factory));
        if ours {
            registry.set_highlighter(&language_id, previous);
            refresh_highlighting(cx, &language_id);
        }
    }))
}

/// Re-resolves the highlighter of models in `language_id`, and of models in
/// Monarch languages that embed languages (which may be this one).
fn refresh_highlighting(cx: &mut App, language_id: &str) {
    let registry = language_registry(cx);
    for model in crate::registry::models(cx) {
        model.update(cx, |model, cx| {
            let affected = model.language_id().is_some_and(|id| id == language_id)
                || model.language().is_some_and(|language| {
                    &*language.id == language_id
                        || MonarchHighlighter::tokenizer_of(language, &registry)
                            .is_some_and(|tokenizer| tokenizer.uses_embedded_languages())
                });
            if affected {
                model.refresh_highlighter(cx);
            }
        });
    }
}

/// Runs `callback` once, the first time a model uses `language_id` (Monaco
/// `languages.onLanguage`), e.g. to register its providers lazily. Models
/// resolve their language when first shown. As in Monaco, a language that
/// was already used doesn't call back.
pub fn on_language(
    cx: &mut App,
    language_id: impl Into<SharedString>,
    callback: impl FnOnce(&mut App) + 'static,
) {
    let language_id = language_id.into();
    let requested = cx.default_global::<RequestedLanguages>();
    if requested.seen.contains(&language_id) {
        return;
    }
    requested
        .waiting
        .entry(language_id)
        .or_default()
        .push(Box::new(callback));
}

/// A model now uses `language_id` (Monaco
/// `requestRichLanguageFeatures`): the first time, runs its `on_language`
/// callbacks after the current update.
pub(crate) fn language_requested(language_id: &SharedString, cx: &mut App) {
    let requested = cx.default_global::<RequestedLanguages>();
    if !requested.seen.insert(language_id.clone()) {
        return;
    }
    let Some(callbacks) = requested.waiting.remove(language_id) else {
        return;
    };
    cx.defer(move |cx| {
        for callback in callbacks {
            callback(cx);
        }
    });
}

/// The path of `uri` for language guessing (Monaco uses `fsPath` for
/// `file:` uris and `path` otherwise), percent-decoded.
pub(crate) fn uri_path(uri: &str) -> Option<String> {
    let rest = match uri.split_once("://") {
        Some((_, rest)) => rest.find('/').map(|ix| &rest[ix..])?,
        None => uri.split_once(':').map_or(uri, |(_, path)| path),
    };
    let path = rest.split(['?', '#']).next()?;
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = path
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            decoded.push(byte);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8_lossy(&decoded).into_owned();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::uri_path;

    #[test]
    fn paths_of_uris() {
        assert_eq!(uri_path("file:///a/b%20c.rs").as_deref(), Some("/a/b c.rs"));
        assert_eq!(
            uri_path("inmemory://model/1/main.ts?x=1").as_deref(),
            Some("/1/main.ts")
        );
        assert_eq!(
            uri_path("untitled:Untitled-1").as_deref(),
            Some("Untitled-1")
        );
        assert_eq!(uri_path("http://host").as_deref(), None);
    }
}
