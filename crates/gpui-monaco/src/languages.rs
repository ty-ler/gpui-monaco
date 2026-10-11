//! Monaco's `monaco.languages` extras: `setLanguageConfiguration` at
//! runtime, `onLanguage`, and guessing a model's language from its uri and
//! first line.

use std::collections::{HashMap, HashSet};

use gpui::{App, Global, SharedString};
use syntax::LanguageConfiguration;

use crate::model::language_registry;

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
