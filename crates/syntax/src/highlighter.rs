//! The pluggable [`Highlighter`] interface.

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

use text::{BufferSnapshot, Patch, Rope};

use crate::language::{Language, LanguageRegistry};
use crate::syntax_map::{HighlightSpan, SyntaxSnapshot};

/// Opaque parse state produced by a [`Highlighter`] for one buffer version.
#[derive(Clone)]
pub struct HighlightState(Arc<dyn Any + Send + Sync>);

impl HighlightState {
    /// Wraps a highlighter's state.
    pub fn new<T: Any + Send + Sync>(state: T) -> Self {
        Self(Arc::new(state))
    }

    /// The state as a `T`, if that is its type.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }

    /// The tree-sitter syntax, when produced by [`TreeSitterHighlighter`].
    pub fn syntax(&self) -> Option<&SyntaxSnapshot> {
        self.downcast_ref()
    }
}

/// Produces highlight spans for a buffer.
///
/// [`Highlighter::update`] may be slow (it parses) and is meant to run on a
/// background thread; [`Highlighter::highlights`] runs per frame for the
/// visible range and must be fast.
pub trait Highlighter: Send + Sync {
    /// Computes the state for `text`. `previous` is the state for an older
    /// version along with that version's text and the edits since, for
    /// incremental work.
    fn update(
        &self,
        previous: Option<(&HighlightState, &Rope, &Patch)>,
        text: &BufferSnapshot,
    ) -> HighlightState;

    /// Sorted, non-overlapping spans intersecting `range`.
    fn highlights(
        &self,
        state: &HighlightState,
        text: &Rope,
        range: Range<usize>,
    ) -> Vec<HighlightSpan>;
}

/// The tree-sitter [`Highlighter`]; its state is a [`SyntaxSnapshot`].
pub struct TreeSitterHighlighter {
    language: Arc<Language>,
    registry: Arc<LanguageRegistry>,
}

impl TreeSitterHighlighter {
    /// A highlighter for buffers in `language`, resolving injected languages
    /// through `registry`.
    pub fn new(language: Arc<Language>, registry: Arc<LanguageRegistry>) -> Self {
        Self { language, registry }
    }
}

impl Highlighter for TreeSitterHighlighter {
    fn update(
        &self,
        previous: Option<(&HighlightState, &Rope, &Patch)>,
        text: &BufferSnapshot,
    ) -> HighlightState {
        let syntax =
            match previous.and_then(|(state, old, patch)| Some((state.syntax()?, old, patch))) {
                Some((syntax, old_text, patch)) => {
                    syntax.reparse(old_text, text, patch, &self.registry)
                }
                None => SyntaxSnapshot::parse(self.language.clone(), text, &self.registry),
            };
        HighlightState::new(syntax)
    }

    fn highlights(
        &self,
        state: &HighlightState,
        text: &Rope,
        range: Range<usize>,
    ) -> Vec<HighlightSpan> {
        state
            .syntax()
            .map(|syntax| syntax.highlights(text, range))
            .unwrap_or_default()
    }
}
