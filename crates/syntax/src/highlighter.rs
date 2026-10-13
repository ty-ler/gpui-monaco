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

    /// Whether `other` is this same state (a clone of it), so whatever was
    /// read from one holds for the other.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
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

    /// Foldable regions as (first row, last row folded), 0-based, or `None`
    /// if this highlighter doesn't know (folding then uses indentation).
    fn fold_ranges(&self, _state: &HighlightState, _text: &Rope) -> Option<Vec<(u32, u32)>> {
        None
    }

    /// Byte ranges of the syntax nodes around `offset`, for smart select,
    /// or `None` if this highlighter doesn't know.
    fn selection_ranges(
        &self,
        _state: &HighlightState,
        _offset: usize,
    ) -> Option<Vec<Range<usize>>> {
        None
    }

    /// Definitions from `tags.scm` queries, for document symbols, or `None`.
    fn tags(&self, _state: &HighlightState, _text: &Rope) -> Option<Vec<crate::syntax_map::Tag>> {
        None
    }

    /// Whether the editor's `max_tokenization_line_length` applies: lines
    /// that long are then shown and queried as untokenized. `true` by
    /// default, as for Monaco's Monarch tokenizer; a
    /// [`TokensProviderHighlighter`](crate::TokensProviderHighlighter)
    /// returns `false`, as Monaco's `setTokensProvider` adapter has no
    /// limit.
    fn limits_line_length(&self) -> bool {
        true
    }

    /// The highlighter as [`Any`], for downcasting to its concrete type
    /// (such as [`MonarchHighlighter`](crate::MonarchHighlighter)), or
    /// `None`.
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }

    /// The tag name at `offset` and its matching start or end tag's name,
    /// for linked editing, or `None`.
    fn linked_tag_ranges(
        &self,
        _state: &HighlightState,
        _offset: usize,
    ) -> Option<Vec<Range<usize>>> {
        None
    }

    /// The rows of `text` (end exclusive, sorted) whose highlights may
    /// differ between `previous` (what [`Highlighter::update`] got) and
    /// `state` (what it returned), as Monaco's `onDidChangeTokens` reports
    /// the lines a retokenization changed; `None` if this highlighter can't
    /// tell.
    fn changed_rows(
        &self,
        _previous: (&HighlightState, &Rope, &Patch),
        _state: &HighlightState,
        _text: &Rope,
    ) -> Option<Vec<Range<u32>>> {
        None
    }
}

/// Rows `ranges` (byte ranges of `text`) touch, merged and sorted, end
/// exclusive.
pub(crate) fn rows_of(
    text: &Rope,
    ranges: impl IntoIterator<Item = Range<usize>>,
) -> Vec<Range<u32>> {
    let mut rows: Vec<Range<u32>> = ranges
        .into_iter()
        .map(|range| {
            let start = text.offset_to_point(range.start.min(text.len())).row;
            let end = text.offset_to_point(range.end.min(text.len())).row;
            start..end.max(start) + 1
        })
        .collect();
    rows.sort_by_key(|rows| rows.start);
    let mut merged: Vec<Range<u32>> = Vec::with_capacity(rows.len());
    for range in rows {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
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

    fn fold_ranges(&self, state: &HighlightState, text: &Rope) -> Option<Vec<(u32, u32)>> {
        state.syntax()?.fold_ranges(text)
    }

    fn changed_rows(
        &self,
        (previous, old_text, patch): (&HighlightState, &Rope, &Patch),
        state: &HighlightState,
        text: &Rope,
    ) -> Option<Vec<Range<u32>>> {
        let ranges = state
            .syntax()?
            .changed_ranges(previous.syntax()?, old_text, text, patch);
        Some(rows_of(text, ranges))
    }

    fn selection_ranges(&self, state: &HighlightState, offset: usize) -> Option<Vec<Range<usize>>> {
        Some(state.syntax()?.selection_ranges(offset))
    }

    fn linked_tag_ranges(
        &self,
        state: &HighlightState,
        offset: usize,
    ) -> Option<Vec<Range<usize>>> {
        state.syntax()?.linked_tag_ranges(offset)
    }

    fn tags(&self, state: &HighlightState, text: &Rope) -> Option<Vec<crate::syntax_map::Tag>> {
        state.syntax()?.tags(text)
    }
}
