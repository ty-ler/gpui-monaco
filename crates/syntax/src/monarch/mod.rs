//! Monaco's Monarch tokenizer (`standalone/common/monarch`, MIT): a
//! declarative regex state machine, so grammars written for Monaco's
//! `languages.setMonarchTokensProvider` work unchanged.
//!
//! A grammar is the JSON form of Monaco's `IMonarchLanguage`, with regexes
//! as strings (JSON needs `\\` for each regex backslash):
//!
//! ```
//! use syntax::MonarchTokenizer;
//!
//! let grammar = serde_json::json!({
//!     "keywords": ["fn", "let"],
//!     "tokenizer": {
//!         "root": [
//!             ["[a-z_]\\w*", { "cases": { "@keywords": "keyword", "@default": "identifier" } }],
//!             ["\\d+", "number"],
//!             ["\"", "string", "@string"],
//!         ],
//!         "string": [
//!             ["[^\"]+", "string"],
//!             ["\"", "string", "@pop"],
//!         ],
//!     },
//! });
//! let tokenizer = MonarchTokenizer::new("mini", &grammar).unwrap();
//! let result = tokenizer.tokenize("let x = 42", true, &tokenizer.initial_state(), &());
//! let types: Vec<_> = result.tokens.iter().map(|t| t.token_type.as_str()).collect();
//! assert_eq!(
//!     types,
//!     ["keyword.mini", "source.mini", "identifier.mini", "source.mini", "number.mini"]
//! );
//! ```
//!
//! Everything in Monaco's grammars is supported: states and their dotted
//! parents, `include`, `cases` (with `@attr` word lists, `~regex`,
//! `==`/`!=`, `$n`/`$Sn`/`$#` scrutinees and `@eos`), groups, `next`
//! (`@push`, `@pop`, `@popall`, `$n` states), `switchTo`, `goBack`,
//! `@rematch`, `nextEmbedded` (other Monarch languages from the registry),
//! `@brackets`, `log` (through the `log` crate), `ignoreCase`,
//! `includeLF`, `tokenPostfix`, `defaultToken`, `start`, `brackets`, and
//! `@attr`, `@@`, `$Sn` and `$n` substitutions.
//!
//! # Regular expressions
//!
//! Regexes are JavaScript sources run by [`fancy_regex`], translated so
//! that `\d`, `\w` and `\s` keep JavaScript's sets, `.` doesn't match line
//! terminators, and `{`, `}` and `[` are literal where JavaScript treats
//! them so. Lookahead, lookbehind (including variable length),
//! backreferences, named groups and `\p{...}` work. Differences:
//! - `\b` and `\B` use Unicode word characters (`é` is one; JavaScript's
//!   `\w` is ASCII).
//! - Matching is by code point: `.` matches a whole astral character where
//!   JavaScript without the `u` flag matches half a surrogate pair. The
//!   `unicode` flag changes nothing.
//! - Case-insensitive matching (`ignoreCase`) uses Unicode simple case
//!   folding.
//! - Octal escapes (`\012`) and `\c` without a letter aren't supported.
//! - Offsets are bytes in [`MonarchToken`]; `goBack` and the
//!   maximum line length count UTF-16 code units, as in Monaco.
//!
//! # Embedded languages
//!
//! `nextEmbedded` resolves names through [`EmbeddedLanguages`]; with the
//! [`MonarchHighlighter`], that's the language registry, by id, name,
//! alias or MIME type. Only Monarch languages can be embedded: Monaco
//! tokenizes line by line with each tokenizer's state, which tree-sitter
//! grammars don't have, so embedded text in another language gets one
//! `""` token per line, as Monaco gives a language without a tokenizer.

mod compile;
mod highlighter;
pub(crate) mod js_regex;
mod lexer;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde_json::Value;

pub use highlighter::{MonarchHighlightState, MonarchHighlighter};

use compile::{Attributes, Lexer};
use lexer::{Collector, EmbeddedData, Run};

/// Monaco's default `editor.maxTokenizationLineLength`.
pub const DEFAULT_MAX_TOKENIZATION_LINE_LENGTH: usize = 20_000;

/// An invalid grammar, or a tokenizer error at runtime (Monaco throws
/// both). The message starts with the language id, as Monaco's do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonarchError(pub String);

impl std::fmt::Display for MonarchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MonarchError {}

/// A token of a line (Monaco `languages.Token`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonarchToken {
    /// Where the token starts, in bytes from the start of the line. It ends
    /// where the next one starts.
    pub offset: usize,
    /// The token type with the grammar's `tokenPostfix` (`keyword.rust`),
    /// or `""` for no style.
    pub token_type: String,
    /// The language id: the grammar's, or an embedded language's.
    pub language: Arc<str>,
}

/// A line's tokens and the state after it (Monaco `TokenizationResult`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonarchLineTokens {
    /// The tokens, in order. Empty lines may have none.
    pub tokens: Vec<MonarchToken>,
    /// The state for the next line.
    pub end_state: MonarchState,
}

/// The tokenizer state between lines (Monaco `MonarchLineState`): the state
/// stack and, inside an embedded language, that language's state. Equal
/// states tokenize the rest of a document the same, which is how edits
/// stop re-tokenizing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonarchState(Arc<StackState>);

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StackState {
    /// Bottom to top.
    stack: Vec<Arc<str>>,
    embedded: Option<EmbeddedData>,
}

impl StackState {
    fn top(&self) -> &str {
        self.stack.last().map_or("", |s| s)
    }
}

impl MonarchState {
    fn new(stack: Vec<Arc<str>>, embedded: Option<EmbeddedData>) -> Self {
        Self(Arc::new(StackState { stack, embedded }))
    }

    /// The state stack, bottom (the start state) first.
    pub fn stack(&self) -> impl Iterator<Item = &str> {
        self.0.stack.iter().map(|s| &**s)
    }

    /// The embedded language the line ended in, if any.
    pub fn embedded_language(&self) -> Option<&str> {
        self.0.embedded.as_ref().map(|data| &*data.language_id)
    }
}

/// Finds the languages `nextEmbedded` names (Monaco's language service and
/// `TokenizationRegistry`). `()` knows none.
pub trait EmbeddedLanguages {
    /// The id of the registered language with this id, name, alias or MIME
    /// type, or `None` if there's none.
    fn language_id(&self, name: &str) -> Option<Arc<str>>;

    /// The Monarch tokenizer of the language `language_id`, if it has one.
    fn tokenizer(&self, language_id: &str) -> Option<Arc<MonarchTokenizer>>;

    /// The line length (in UTF-16 code units) from which tokenizers skip a
    /// line, overriding their own: Monaco's tokenizers, embedded ones
    /// included, all read `editor.maxTokenizationLineLength`. `None` keeps
    /// each tokenizer's.
    fn max_tokenization_line_length(&self) -> Option<usize> {
        None
    }
}

impl EmbeddedLanguages for () {
    fn language_id(&self, _name: &str) -> Option<Arc<str>> {
        None
    }

    fn tokenizer(&self, _language_id: &str) -> Option<Arc<MonarchTokenizer>> {
        None
    }
}

/// A compiled Monarch grammar (Monaco `MonarchTokenizer`): tokenizes a line
/// at a time from the previous line's [`MonarchState`].
pub struct MonarchTokenizer {
    lexer: Lexer,
    attrs: Attributes,
    max_tokenization_line_length: usize,
}

impl std::fmt::Debug for MonarchTokenizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonarchTokenizer")
            .field("language_id", &self.lexer.language_id)
            .finish_non_exhaustive()
    }
}

impl MonarchTokenizer {
    /// Compiles `grammar` (Monaco `IMonarchLanguage` as JSON) for
    /// `language_id` (Monaco `compile`), failing as Monaco throws for
    /// invalid grammars: unknown states or `include` targets, bad actions,
    /// missing `@attr` references, invalid regexes and so on.
    pub fn new(language_id: &str, grammar: &Value) -> Result<Self, MonarchError> {
        let (lexer, attrs) = compile::compile(language_id, grammar)?;
        Ok(Self {
            lexer,
            attrs,
            max_tokenization_line_length: DEFAULT_MAX_TOKENIZATION_LINE_LENGTH,
        })
    }

    /// Lines this long (in UTF-16 code units) or longer get one `""` token
    /// and keep their state (Monaco `editor.maxTokenizationLineLength`,
    /// 20,000 by default).
    pub fn with_max_tokenization_line_length(mut self, length: usize) -> Self {
        self.max_tokenization_line_length = length;
        self
    }

    /// The language id the grammar was compiled for.
    pub fn language_id(&self) -> &str {
        &self.lexer.language_id
    }

    /// Whether the grammar has `nextEmbedded` actions.
    pub fn uses_embedded_languages(&self) -> bool {
        self.lexer.uses_embedded
    }

    /// The state of the first line (Monaco `getInitialState`): the `start`
    /// state, or the first one.
    pub fn initial_state(&self) -> MonarchState {
        MonarchState::new(vec![self.lexer.start.as_str().into()], None)
    }

    /// Tokenizes `line` (without its line break; `has_eol` says whether one
    /// follows, for `includeLF`) from `state` (Monaco `tokenize`). Like
    /// Monaco's tokenization of models, a grammar error gives the whole
    /// line one `""` token and keeps the state; it's logged.
    pub fn tokenize(
        &self,
        line: &str,
        has_eol: bool,
        state: &MonarchState,
        embedded: &dyn EmbeddedLanguages,
    ) -> MonarchLineTokens {
        self.try_tokenize(line, has_eol, state, embedded)
            .unwrap_or_else(|error| {
                log::warn!("{error}");
                self.null_tokens(state)
            })
    }

    /// Like [`MonarchTokenizer::tokenize`], but returns grammar errors
    /// (Monaco throws them).
    pub fn try_tokenize(
        &self,
        line: &str,
        has_eol: bool,
        state: &MonarchState,
        embedded: &dyn EmbeddedLanguages,
    ) -> Result<MonarchLineTokens, MonarchError> {
        let max_length = embedded
            .max_tokenization_line_length()
            .unwrap_or(self.max_tokenization_line_length);
        if line.encode_utf16().count() >= max_length {
            return Ok(self.null_tokens(state));
        }
        let mut collector = Collector::new();
        let run = Run {
            lexer: &self.lexer,
            attrs: &self.attrs,
            embedded,
        };
        let end_state = run.tokenize(line, has_eol, state, &mut collector)?;
        Ok(MonarchLineTokens {
            tokens: collector.tokens,
            end_state,
        })
    }

    /// Monaco `nullTokenize`.
    fn null_tokens(&self, state: &MonarchState) -> MonarchLineTokens {
        MonarchLineTokens {
            tokens: vec![MonarchToken {
                offset: 0,
                token_type: String::new(),
                language: self.lexer.language_id.clone(),
            }],
            end_state: state.clone(),
        }
    }

    /// Tokenizes every line of `text` (split at `\r\n`, `\r` and `\n`)
    /// from the initial state, as Monaco's `editor.tokenize` does.
    pub fn tokenize_text(
        &self,
        text: &str,
        embedded: &dyn EmbeddedLanguages,
    ) -> Vec<Vec<MonarchToken>> {
        let mut state = self.initial_state();
        split_lines(text)
            .into_iter()
            .map(|line| {
                let result = self.tokenize(line, true, &state, embedded);
                state = result.end_state;
                result.tokens
            })
            .collect()
    }
}

/// Monaco `strings.splitLines`.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                lines.push(&text[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = i;
            }
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    lines.push(&text[start..]);
    lines
}
