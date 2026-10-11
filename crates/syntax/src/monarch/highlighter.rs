//! [`MonarchHighlighter`]: a [`Highlighter`] running a Monarch grammar line
//! by line, keeping each line's end state like Monaco's
//! `TokenizationStateStore`.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use serde_json::Value;
use text::{BufferSnapshot, Patch, Point, Rope};

use super::{EmbeddedLanguages, MonarchError, MonarchState, MonarchToken, MonarchTokenizer};
use crate::highlighter::{HighlightState, Highlighter};
use crate::language::{HighlightId, HighlighterFactory, Language, LanguageRegistry};
use crate::syntax_map::HighlightSpan;

/// A token as the highlighter keeps it.
#[derive(Clone, Debug)]
struct LineToken {
    /// Byte offset in the line.
    offset: u32,
    /// `None` for `""` tokens.
    highlight: Option<HighlightId>,
    language: Arc<str>,
}

#[derive(Clone, Debug)]
struct Line {
    tokens: Arc<[LineToken]>,
    end_state: MonarchState,
}

/// The [`HighlightState`] of a [`MonarchHighlighter`]: every line's tokens
/// and end state.
#[derive(Clone, Debug, Default)]
pub struct MonarchHighlightState {
    lines: Vec<Line>,
}

impl MonarchHighlightState {
    /// The number of lines tokenized.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The tokens of `row` (0-based), with Monaco token types such as
    /// `keyword.rust` and byte offsets in the line.
    pub fn line_tokens(&self, row: usize) -> Option<Vec<MonarchToken>> {
        let line = self.lines.get(row)?;
        Some(
            line.tokens
                .iter()
                .map(|token| MonarchToken {
                    offset: token.offset as usize,
                    token_type: token
                        .highlight
                        .map(|id| id.name().to_string())
                        .unwrap_or_default(),
                    language: token.language.clone(),
                })
                .collect(),
        )
    }

    /// The state after `row`.
    pub fn end_state(&self, row: usize) -> Option<&MonarchState> {
        self.lines.get(row).map(|line| &line.end_state)
    }

    /// The language of the token at byte `column` of `row`: the grammar's,
    /// or an embedded language's.
    pub fn language_at(&self, row: usize, column: usize) -> Option<Arc<str>> {
        let tokens = &self.lines.get(row)?.tokens;
        let index = tokens
            .iter()
            .rposition(|token| token.offset as usize <= column)
            .or((!tokens.is_empty()).then_some(0))?;
        Some(tokens[index].language.clone())
    }
}

/// A [`Highlighter`] for a Monarch grammar (Monaco
/// `languages.setMonarchTokensProvider`). Spans are Monaco token names
/// interned with [`HighlightId::intern_monaco_token`], which themes match
/// against their Monaco rules (`keyword` matches `keyword.rust`).
///
/// Edits re-tokenize from the first changed line until a line's end state
/// matches the one it had before, as Monaco does.
pub struct MonarchHighlighter {
    tokenizer: Arc<MonarchTokenizer>,
    registry: Option<Arc<LanguageRegistry>>,
}

impl MonarchHighlighter {
    /// A highlighter running `tokenizer`, finding `nextEmbedded` languages
    /// in `registry`.
    pub fn new(tokenizer: Arc<MonarchTokenizer>, registry: Option<Arc<LanguageRegistry>>) -> Self {
        Self {
            tokenizer,
            registry,
        }
    }

    /// A [`HighlighterFactory`] for [`Language::highlighter`] creating
    /// highlighters that share `tokenizer`.
    pub fn factory(tokenizer: Arc<MonarchTokenizer>) -> HighlighterFactory {
        Arc::new(move |_language, registry| {
            Arc::new(MonarchHighlighter::new(
                tokenizer.clone(),
                Some(registry.clone()),
            ))
        })
    }

    /// The highlighter as a `MonarchHighlighter`, if it is one.
    pub fn downcast(highlighter: &dyn Highlighter) -> Option<&MonarchHighlighter> {
        highlighter.as_any()?.downcast_ref()
    }

    /// The Monarch tokenizer of `language`, if its highlighter is one.
    pub fn tokenizer_of(
        language: &Arc<Language>,
        registry: &Arc<LanguageRegistry>,
    ) -> Option<Arc<MonarchTokenizer>> {
        let factory = language.highlighter.as_ref()?;
        let highlighter = factory(language, registry);
        Self::downcast(&*highlighter).map(|monarch| monarch.tokenizer.clone())
    }

    /// The compiled grammar.
    pub fn tokenizer(&self) -> &Arc<MonarchTokenizer> {
        &self.tokenizer
    }

    /// The registry embedded languages are found in.
    pub fn registry(&self) -> Option<&Arc<LanguageRegistry>> {
        self.registry.as_ref()
    }

    /// Tokenizes every line of `text` from the initial state (Monaco
    /// `editor.tokenize`), with embedded languages from the registry.
    pub fn tokenize_text(&self, text: &str) -> Vec<Vec<MonarchToken>> {
        let embedded = RegistryLanguages::new(self.registry.as_ref());
        self.tokenizer.tokenize_text(text, &embedded)
    }
}

/// [`EmbeddedLanguages`] from a [`LanguageRegistry`]: Monarch languages
/// by id, name, alias or MIME type.
struct RegistryLanguages<'a> {
    registry: Option<&'a Arc<LanguageRegistry>>,
    tokenizers: RefCell<HashMap<String, Option<Arc<MonarchTokenizer>>>>,
}

impl<'a> RegistryLanguages<'a> {
    fn new(registry: Option<&'a Arc<LanguageRegistry>>) -> Self {
        Self {
            registry,
            tokenizers: RefCell::default(),
        }
    }
}

impl EmbeddedLanguages for RegistryLanguages<'_> {
    fn language_id(&self, name: &str) -> Option<Arc<str>> {
        let registry = self.registry?;
        registry
            .language_for_name(name)
            .or_else(|| registry.language_for_mime_type(name))
            .map(|language| language.id.clone())
    }

    fn tokenizer(&self, language_id: &str) -> Option<Arc<MonarchTokenizer>> {
        let registry = self.registry?;
        if let Some(cached) = self.tokenizers.borrow().get(language_id) {
            return cached.clone();
        }
        // Monaco's lexer requests basic features for embedded languages
        // (`onLanguageEncountered`).
        registry.note_language_encountered(language_id);
        let tokenizer = registry
            .language_for_id(language_id)
            .and_then(|language| MonarchHighlighter::tokenizer_of(&language, registry));
        self.tokenizers
            .borrow_mut()
            .insert(language_id.to_string(), tokenizer.clone());
        tokenizer
    }
}

fn line_tokens(
    tokens: Vec<MonarchToken>,
    ids: &mut HashMap<String, HighlightId>,
) -> Arc<[LineToken]> {
    tokens
        .into_iter()
        .map(|token| LineToken {
            offset: token.offset as u32,
            highlight: (!token.token_type.is_empty()).then(|| {
                *ids.entry(token.token_type)
                    .or_insert_with_key(|name| HighlightId::intern_monaco_token(name))
            }),
            language: token.language,
        })
        .collect()
}

impl Highlighter for MonarchHighlighter {
    fn update(
        &self,
        previous: Option<(&HighlightState, &Rope, &Patch)>,
        text: &BufferSnapshot,
    ) -> HighlightState {
        let rope = text.rope();
        let line_count = rope.max_point().row as usize + 1;
        let previous = previous.and_then(|(state, old_rope, patch)| {
            Some((
                state,
                state.downcast_ref::<MonarchHighlightState>()?,
                old_rope,
                patch,
            ))
        });

        let mut lines: Vec<Line> = Vec::with_capacity(line_count);
        // The old lines, the last edited row in the new text, and that
        // row in the old text: rows after it are unchanged.
        let mut tail: Option<(&MonarchHighlightState, usize, usize)> = None;
        if let Some((state, old, old_rope, patch)) = previous {
            let (Some(first), Some(last)) = (patch.edits().first(), patch.edits().last()) else {
                if old.lines.len() == line_count {
                    return state.clone();
                }
                return self.update(None, text);
            };
            let first_row =
                (old_rope.offset_to_point(first.old.start).row as usize).min(old.lines.len());
            lines.extend_from_slice(&old.lines[..first_row]);
            tail = Some((
                old,
                rope.offset_to_point(last.new.end).row as usize,
                old_rope.offset_to_point(last.old.end).row as usize,
            ));
        }

        let embedded = RegistryLanguages::new(self.registry.as_ref());
        let mut ids = HashMap::new();
        let start_row = lines.len();
        let mut state = lines.last().map_or_else(
            || self.tokenizer.initial_state(),
            |line| line.end_state.clone(),
        );
        let mut row = start_row;
        let mut converged_at = None;
        let start = rope.point_to_offset(Point::new(start_row as u32, 0));
        rope.for_each_line(start..rope.len(), |_, line| {
            let result = self.tokenizer.tokenize(line, true, &state, &embedded);
            state = result.end_state;
            lines.push(Line {
                tokens: line_tokens(result.tokens, &mut ids),
                end_state: state.clone(),
            });
            if let Some((old, last_new_row, last_old_row)) = tail
                && row > last_new_row
            {
                let old_row = row - last_new_row + last_old_row;
                if old
                    .lines
                    .get(old_row)
                    .is_some_and(|old_line| old_line.end_state == state)
                {
                    converged_at = Some(old_row);
                    return false;
                }
            }
            row += 1;
            true
        });
        if let (Some(old_row), Some((old, ..))) = (converged_at, tail) {
            lines.extend_from_slice(&old.lines[old_row + 1..]);
        }
        debug_assert_eq!(lines.len(), line_count);
        HighlightState::new(MonarchHighlightState { lines })
    }

    fn highlights(
        &self,
        state: &HighlightState,
        text: &Rope,
        range: Range<usize>,
    ) -> Vec<HighlightSpan> {
        let Some(state) = state.downcast_ref::<MonarchHighlightState>() else {
            return Vec::new();
        };
        let start_row = text.offset_to_point(range.start).row as usize;
        let end_row = text.offset_to_point(range.end).row as usize;
        let mut spans = Vec::new();
        for row in start_row..=end_row {
            let Some(line) = state.lines.get(row) else {
                break;
            };
            let line_start = text.point_to_offset(Point::new(row as u32, 0));
            let line_len = text.line_len(row as u32) as usize;
            for (ix, token) in line.tokens.iter().enumerate() {
                let Some(highlight) = token.highlight else {
                    continue;
                };
                let start = line_start + (token.offset as usize).min(line_len);
                let end = line_start
                    + line
                        .tokens
                        .get(ix + 1)
                        .map_or(line_len, |next| (next.offset as usize).min(line_len));
                if start < end && end > range.start && start < range.end.max(range.start + 1) {
                    spans.push(HighlightSpan {
                        range: start..end,
                        highlight,
                    });
                }
            }
        }
        spans
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl Language {
    /// Highlights this language with a Monarch grammar (Monaco
    /// `languages.setMonarchTokensProvider`), compiled for the language's
    /// id; see [`MonarchTokenizer::new`].
    pub fn with_monarch_grammar(mut self, grammar: &Value) -> Result<Self, MonarchError> {
        let tokenizer = Arc::new(MonarchTokenizer::new(&self.id, grammar)?);
        self.highlighter = Some(MonarchHighlighter::factory(tokenizer));
        Ok(self)
    }
}
