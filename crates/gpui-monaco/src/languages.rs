//! Monaco's `monaco.languages` extras: `setLanguageConfiguration`,
//! `setMonarchTokensProvider`, `setTokensProvider` and
//! `registerTokensProviderFactory` at runtime, `onLanguage` and
//! `onLanguageEncountered`, and guessing a model's language from its uri
//! and first line.
//!
//! Monaco's `setColorMap`, `getEncodedLanguageId` and
//! `EncodedTokensProvider` have no equivalent: they serve encoded token
//! metadata (language and color indices packed into numbers), while
//! highlighters here give named tokens that the theme resolves by name.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Global, SharedString, Task};
use syntax::{
    HighlighterFactory, LanguageConfiguration, MonarchHighlighter, MonarchTokenizer,
    TokensProvider, TokensProviderHighlighter,
};

use crate::model::language_registry;
use crate::registry::Disposable;

/// An `on_language` or `on_language_encountered` callback.
type LanguageCallback = Box<dyn FnOnce(&mut App)>;

/// Callbacks waiting for a language, by id, with their registration ids.
type Waiting = HashMap<SharedString, Vec<(u64, LanguageCallback)>>;

/// Languages already requested, and the callbacks waiting for theirs:
/// Monaco's `requestRichLanguageFeatures` (a model uses the language) and
/// `requestBasicLanguageFeatures` (a model uses it, or it's met while
/// tokenizing another).
#[derive(Default)]
struct RequestedLanguages {
    rich: HashSet<SharedString>,
    basic: HashSet<SharedString>,
    next_id: u64,
    waiting_rich: Waiting,
    waiting_basic: Waiting,
}

impl Global for RequestedLanguages {}

/// Each language's tokens provider factory (Monaco
/// `TokenizationRegistry._factories`): one per language id, with its
/// registration id and what disposes it.
#[derive(Default)]
struct TokenizerFactories {
    next_id: u64,
    factories: HashMap<SharedString, (u64, Disposable)>,
}

impl Global for TokenizerFactories {}

/// Layers `config` over the configuration of the registered language
/// `language_id` at runtime (Monaco `languages.setLanguageConfiguration`).
/// Each field `config` sets replaces the language's, and fields it leaves
/// `None` keep theirs, so `{ comments }` alone keeps the brackets and
/// auto-closing pairs; with several, the latest wins per field. Every model
/// of the language picks it up and emits
/// [`TextModelEvent::LanguageConfigurationChanged`](crate::TextModelEvent::LanguageConfigurationChanged).
///
/// Fails, as Monaco throws, for an unknown language. Disposing the result
/// removes this configuration, restoring what it replaced.
pub fn set_language_configuration(
    cx: &mut App,
    language_id: &str,
    config: LanguageConfiguration,
) -> anyhow::Result<Disposable> {
    let Some(registration) = language_registry(cx).register_configuration(language_id, config)
    else {
        anyhow::bail!("Cannot set configuration for unknown language {language_id}");
    };
    refresh_configuration(cx, language_id);
    let language_id = language_id.to_string();
    Ok(Disposable::new(move |cx| {
        if language_registry(cx).unregister_configuration(&language_id, registration) {
            refresh_configuration(cx, &language_id);
        }
    }))
}

/// Re-resolves the language of models in `language_id` after its
/// configuration changed, including plain text models for `plaintext`.
fn refresh_configuration(cx: &mut App, language_id: &str) {
    for model in crate::registry::models(cx) {
        model.update(cx, |model, cx| {
            if model.language_id().is_some_and(|id| id == language_id)
                || model
                    .language()
                    .is_some_and(|language| &*language.id == language_id)
            {
                model.refresh_language(cx);
            }
        });
    }
}

/// Registers `language` in the [language registry](crate::language_registry)
/// (Monaco `languages.register`), as
/// [`LanguageRegistry::register`](syntax::LanguageRegistry::register) does,
/// merging into an existing registration of its id, then updates models
/// as Monaco's language service does: a model whose id is the new
/// language's (or its name or MIME type), or whose uri now guesses it,
/// takes it, and so do models already in it. Sets an empty registry first
/// if there's none.
pub fn register_language(cx: &mut App, language: syntax::Language) -> Arc<syntax::Language> {
    let language = crate::model::language_registry_mut(cx).register(language);
    languages_changed(cx);
    language
}

/// Re-resolves the language of every model after registrations changed.
pub(crate) fn languages_changed(cx: &mut App) {
    for model in crate::registry::models(cx) {
        model.update(cx, |model, cx| model.languages_changed(cx));
    }
}

/// Highlights the language `language_id` with a Monarch grammar (Monaco
/// `languages.setMonarchTokensProvider`): the JSON form of Monaco's
/// `IMonarchLanguage`, regexes as strings (see [`syntax::monarch`] for the
/// regex dialect). It replaces the language's tree-sitter or custom
/// highlighting; models of the language, and of Monarch languages
/// embedding others, re-tokenize. [`colorize`] and [`tokenize`] use it
/// too.
///
/// As in Monaco, the language needn't be registered yet: the grammar
/// applies once it is. Fails, as Monaco throws, for an invalid grammar.
/// Disposing the result restores the previous highlighting, unless another
/// tokenizer replaced this one since.
///
/// [`colorize`]: crate::colorize::colorize
/// [`tokenize`]: crate::colorize::tokenize
pub fn set_monarch_tokens_provider(
    cx: &mut App,
    language_id: &str,
    grammar: &serde_json::Value,
) -> anyhow::Result<Disposable> {
    let tokenizer = Arc::new(MonarchTokenizer::new(language_id, grammar)?);
    Ok(install_highlighter(
        cx,
        language_id,
        MonarchHighlighter::factory(tokenizer),
    ))
}

/// Highlights the registered language `language_id` with a line-state
/// tokenizer (Monaco `languages.setTokensProvider`), run by a
/// [`TokensProviderHighlighter`]: each line is tokenized from the state the
/// previous one ended in, and edits re-tokenize until a line ends as it did
/// before. It replaces the language's other highlighting, like
/// [`set_monarch_tokens_provider`].
///
/// Fails, as Monaco throws, for an unknown language. For a provider
/// that's only available later (Monaco's `Thenable` providers), use
/// [`register_tokens_provider_factory`]. Disposing the result restores the
/// previous highlighting, unless another tokenizer replaced this one
/// since.
pub fn set_tokens_provider<P: TokensProvider>(
    cx: &mut App,
    language_id: &str,
    provider: P,
) -> anyhow::Result<Disposable> {
    if language_registry(cx).language_for_id(language_id).is_none() {
        anyhow::bail!("Cannot set tokens provider for unknown language {language_id}");
    }
    Ok(install_highlighter(
        cx,
        language_id,
        TokensProviderHighlighter::factory(Arc::new(provider)),
    ))
}

/// What a [`register_tokens_provider_factory`] factory creates (Monaco's
/// `TokensProvider | EncodedTokensProvider | IMonarchLanguage`).
pub enum LanguageTokenizer {
    /// A highlighter, such as a [`TokensProviderHighlighter`] (see
    /// [`LanguageTokenizer::tokens_provider`]).
    Highlighter(HighlighterFactory),
    /// A Monarch grammar, as for [`set_monarch_tokens_provider`].
    Monarch(serde_json::Value),
}

impl LanguageTokenizer {
    /// A [`TokensProvider`], run by a [`TokensProviderHighlighter`].
    pub fn tokens_provider(provider: impl TokensProvider) -> Self {
        Self::Highlighter(TokensProviderHighlighter::factory(Arc::new(provider)))
    }

    fn into_factory(self, language_id: &str) -> anyhow::Result<HighlighterFactory> {
        Ok(match self {
            Self::Highlighter(factory) => factory,
            Self::Monarch(grammar) => {
                MonarchHighlighter::factory(Arc::new(MonarchTokenizer::new(language_id, &grammar)?))
            }
        })
    }
}

/// Registers a factory for the tokenizer of `language_id` (Monaco
/// `languages.registerTokensProviderFactory`), called once, when the
/// language is first needed: when a model uses it or it's embedded in
/// another (see [`on_language_encountered`]), or right away if that
/// already happened. Its task's tokenizer then replaces the language's
/// highlighting, like [`set_tokens_provider`]; `None` leaves it as is.
/// As in Monaco, a tokenizer already set for the language then (with
/// [`set_monarch_tokens_provider`], [`set_tokens_provider`] or
/// [`Language::with_highlighter`](syntax::Language::with_highlighter))
/// wins, and the factory isn't called; a tree-sitter grammar doesn't
/// count, so a factory can replace it.
///
/// A language has one factory, as in Monaco's `registerFactory`: a new
/// one disposes the previous, which then isn't called (or, if it was,
/// its task's result is dropped and the tokenizer it installed removed).
///
/// Disposing the result drops the factory if it wasn't called yet, and
/// otherwise restores the previous highlighting, unless another tokenizer
/// replaced this one since.
pub fn register_tokens_provider_factory(
    cx: &mut App,
    language_id: impl Into<SharedString>,
    factory: impl FnOnce(&mut App) -> Task<anyhow::Result<Option<LanguageTokenizer>>> + 'static,
) -> Disposable {
    let language_id = language_id.into();
    // Monaco `registerFactory`: the language's previous factory is
    // disposed first.
    let previous = cx
        .default_global::<TokenizerFactories>()
        .factories
        .remove(&language_id);
    if let Some((_, previous)) = previous {
        previous.dispose(cx);
    }
    let installed: Rc<RefCell<Option<Disposable>>> = Rc::default();
    let disposed = Rc::new(Cell::new(false));
    let create = {
        let language_id = language_id.clone();
        let installed = installed.clone();
        let disposed = disposed.clone();
        move |cx: &mut App| {
            // Monaco `TokenizationRegistry.getOrCreate`: a tokenizer
            // already set wins, and the factory is never called.
            if current_highlighter(&language_registry(cx), &language_id).is_some() {
                return;
            }
            let task = factory(cx);
            cx.spawn(async move |cx| {
                let result = task.await.and_then(|tokenizer| {
                    tokenizer
                        .map(|tokenizer| tokenizer.into_factory(&language_id))
                        .transpose()
                });
                cx.update(|cx| match result {
                    _ if disposed.get() => {}
                    Ok(Some(factory)) => {
                        *installed.borrow_mut() =
                            Some(install_highlighter(cx, &language_id, factory));
                    }
                    Ok(None) => {}
                    Err(err) => log::warn!("tokens provider for {language_id} failed: {err:#}"),
                });
            })
            .detach();
        }
    };
    let registration = when_requested(cx, language_id.clone(), false, true, Box::new(create));
    let disposable = Disposable::new(move |cx| {
        disposed.set(true);
        registration.dispose(cx);
        if let Some(installed) = installed.borrow_mut().take() {
            installed.dispose(cx);
        }
    });
    let factories = cx.default_global::<TokenizerFactories>();
    factories.next_id += 1;
    let id = factories.next_id;
    factories
        .factories
        .insert(language_id.clone(), (id, disposable));
    Disposable::new(move |cx| {
        let factories = cx.default_global::<TokenizerFactories>();
        // Only while it's still the language's factory.
        if factories
            .factories
            .get(&language_id)
            .is_some_and(|(current, _)| *current == id)
            && let Some((_, disposable)) = factories.factories.remove(&language_id)
        {
            disposable.dispose(cx);
        }
    })
}

/// The tokenizer set for `language_id`, registered or not yet: its
/// [`highlighter`](syntax::Language::highlighter), not its tree-sitter
/// grammar.
fn current_highlighter(
    registry: &syntax::LanguageRegistry,
    language_id: &str,
) -> Option<HighlighterFactory> {
    registry
        .language_for_id(language_id)
        .map(|language| language.highlighter.clone())
        .unwrap_or_else(|| registry.pending_highlighter(language_id))
}

/// Sets the highlighter of `language_id` (registered or not yet), re-tokenizing
/// affected models; disposing restores the previous one if this one is still
/// set (Monaco's tokenization registry).
fn install_highlighter(cx: &mut App, language_id: &str, factory: HighlighterFactory) -> Disposable {
    let registry = language_registry(cx);
    let previous = current_highlighter(&registry, language_id);
    registry.set_highlighter(language_id, Some(factory.clone()));
    refresh_highlighting(cx, language_id);
    let language_id = language_id.to_string();
    Disposable::new(move |cx| {
        let registry = language_registry(cx);
        let ours = current_highlighter(&registry, &language_id)
            .is_some_and(|current| Arc::ptr_eq(&current, &factory));
        if ours {
            registry.set_highlighter(&language_id, previous);
            refresh_highlighting(cx, &language_id);
        }
    })
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
/// was already used doesn't call back. Disposing the result before then
/// cancels it.
pub fn on_language(
    cx: &mut App,
    language_id: impl Into<SharedString>,
    callback: impl FnOnce(&mut App) + 'static,
) -> Disposable {
    when_requested(cx, language_id.into(), true, false, Box::new(callback))
}

/// Runs `callback` once, the first time `language_id` is needed for basic
/// features such as highlighting (Monaco `languages.onLanguageEncountered`):
/// when a model uses it, or when it's met while tokenizing another
/// language, like a Monarch grammar's `nextEmbedded`. That's where to set
/// an embedded language's tokenizer lazily. As in Monaco, a language that
/// was already encountered doesn't call back. Disposing the result before
/// then cancels it.
pub fn on_language_encountered(
    cx: &mut App,
    language_id: impl Into<SharedString>,
    callback: impl FnOnce(&mut App) + 'static,
) -> Disposable {
    when_requested(cx, language_id.into(), false, false, Box::new(callback))
}

/// Queues `callback` for the first rich (`rich`) or basic request of
/// `language_id`; with `if_seen`, runs it now if that already happened.
fn when_requested(
    cx: &mut App,
    language_id: SharedString,
    rich: bool,
    if_seen: bool,
    callback: LanguageCallback,
) -> Disposable {
    let requested = cx.default_global::<RequestedLanguages>();
    let seen = if rich {
        requested.rich.contains(&language_id)
    } else {
        requested.basic.contains(&language_id)
    };
    if seen {
        if if_seen {
            callback(cx);
        }
        return Disposable::none();
    }
    requested.next_id += 1;
    let id = requested.next_id;
    let waiting = if rich {
        &mut requested.waiting_rich
    } else {
        &mut requested.waiting_basic
    };
    waiting
        .entry(language_id.clone())
        .or_default()
        .push((id, callback));
    Disposable::new(move |cx| {
        let requested = cx.default_global::<RequestedLanguages>();
        let waiting = if rich {
            &mut requested.waiting_rich
        } else {
            &mut requested.waiting_basic
        };
        if let Some(callbacks) = waiting.get_mut(&language_id) {
            callbacks.retain(|(other, _)| *other != id);
        }
    })
}

/// A model now uses `language_id` (Monaco `requestRichLanguageFeatures`,
/// which also requests basic features): the first time, runs its
/// `on_language_encountered` and `on_language` callbacks after the current
/// update.
pub(crate) fn language_requested(language_id: &SharedString, cx: &mut App) {
    language_encountered(language_id, cx);
    let requested = cx.default_global::<RequestedLanguages>();
    if !requested.rich.insert(language_id.clone()) {
        return;
    }
    let Some(callbacks) = requested.waiting_rich.remove(language_id) else {
        return;
    };
    cx.defer(move |cx| {
        for (_, callback) in callbacks {
            callback(cx);
        }
    });
}

/// `language_id` is needed for basic features (Monaco
/// `requestBasicLanguageFeatures`): the first time, runs its
/// `on_language_encountered` callbacks after the current update.
pub(crate) fn language_encountered(language_id: &SharedString, cx: &mut App) {
    let requested = cx.default_global::<RequestedLanguages>();
    if !requested.basic.insert(language_id.clone()) {
        return;
    }
    let Some(callbacks) = requested.waiting_basic.remove(language_id) else {
        return;
    };
    cx.defer(move |cx| {
        for (_, callback) in callbacks {
            callback(cx);
        }
    });
}

/// Requests basic features for the languages tokenizers met since the last
/// call (Monarch `nextEmbedded`), after a model's highlighting updated.
pub(crate) fn embedded_languages_encountered(cx: &mut App) {
    for language_id in language_registry(cx).take_encountered_languages() {
        language_encountered(&SharedString::from(language_id.to_string()), cx);
    }
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
