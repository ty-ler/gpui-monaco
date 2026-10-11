//! Syntax support for gpui-monaco: languages and their editing configuration,
//! the pluggable [`Highlighter`] interface and its tree-sitter implementation
//! with incremental parsing and language injections, and Monaco's Monarch
//! tokenizer ([`MonarchHighlighter`]).
#![warn(missing_docs)]

mod config;
mod highlighter;
mod language;
pub mod monarch;
mod syntax_map;
mod tokens_provider;

pub use config::{
    AutoClosingPair, Comments, DocComment, ElectricCharacterSupport, EnterAction, Folding,
    FoldingMarkers, IndentAction, IndentationRules, LanguageConfiguration, NotIn, OnEnterRule,
    Pattern, SurroundingPair, strip_jsonc,
};
pub use fancy_regex;
pub use highlighter::{HighlightState, Highlighter, TreeSitterHighlighter};
pub use language::{
    ConfigurationRegistration, Grammar, HighlightId, HighlighterFactory, Language,
    LanguageRegistry, glob_matches,
};
pub use monarch::{
    EmbeddedLanguages, MonarchError, MonarchHighlightState, MonarchHighlighter, MonarchLineTokens,
    MonarchState, MonarchToken, MonarchTokenizer,
};
pub use syntax_map::{HighlightSpan, RopeTextProvider, SyntaxLayer, SyntaxSnapshot, Tag};
pub use tokens_provider::{LineTokens, Token, TokensProvider, TokensProviderHighlighter};
pub use tree_sitter;
