//! Syntax support for gpui-monaco: languages and their editing configuration,
//! the pluggable [`Highlighter`] interface and its tree-sitter implementation
//! with incremental parsing and language injections.
#![warn(missing_docs)]

mod config;
mod highlighter;
mod language;
mod syntax_map;

pub use config::{
    AutoClosingPair, Comments, EnterAction, Folding, FoldingMarkers, IndentAction,
    IndentationRules, LanguageConfiguration, NotIn, OnEnterRule, Pattern, SurroundingPair,
    strip_jsonc,
};
pub use fancy_regex;
pub use highlighter::{HighlightState, Highlighter, TreeSitterHighlighter};
pub use language::{
    Grammar, HighlightId, HighlighterFactory, Language, LanguageRegistry, glob_matches,
};
pub use syntax_map::{HighlightSpan, RopeTextProvider, SyntaxLayer, SyntaxSnapshot, Tag};
pub use tree_sitter;
