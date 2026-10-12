//! Monaco's line-state tokenizers (`languages.TokensProvider`, set with
//! `setTokensProvider`): a [`TokensProvider`] tokenizes one line at a
//! time from the state the previous line ended in, and
//! [`TokensProviderHighlighter`] runs it as a [`Highlighter`], keeping each
//! line's end state like Monaco's `TokenizationStateStore` (and like
//! [`MonarchHighlighter`](crate::MonarchHighlighter)).

use std::any::Any;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use text::{BufferSnapshot, Patch, Point, Rope};

use crate::highlighter::{HighlightState, Highlighter};
use crate::language::{HighlightId, HighlighterFactory};
use crate::syntax_map::HighlightSpan;

/// A token (Monaco `IToken`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Token {
    /// Where it starts: a byte offset in the line (Monaco's `startIndex`
    /// counts UTF-16 units). The first token is taken to start at 0, and
    /// one starting before the previous token at the previous token's
    /// start, as Monaco does for buggy tokenizers.
    pub start_index: usize,
    /// Its Monaco token type, such as `keyword.js`, matched against the
    /// theme's rules; empty for no style (Monaco `scopes`).
    pub scopes: String,
}

impl Token {
    /// A token of type `scopes` starting at byte `start_index`.
    pub fn new(start_index: usize, scopes: impl Into<String>) -> Self {
        Self {
            start_index,
            scopes: scopes.into(),
        }
    }
}

/// A line's tokens and the state after it (Monaco `ILineTokens`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineTokens<S> {
    /// The tokens, in order.
    pub tokens: Vec<Token>,
    /// The state the next line starts in.
    pub end_state: S,
}

/// A "manual" tokenizer (Monaco `TokensProvider`): tokenizes a line given
/// the state the previous line ended in. States are compared to stop
/// re-tokenizing after an edit once a line ends as it did before (Monaco
/// `IState.equals`).
///
/// Monaco's `EncodedTokensProvider` has no equivalent: its tokens carry
/// encoded color-map indices, while tokens here are named and the theme
/// styles them by name.
pub trait TokensProvider: Send + Sync + 'static {
    /// The tokenizer's state between lines (Monaco `IState`).
    type State: Clone + PartialEq + Send + Sync + 'static;

    /// The state the first line starts in (Monaco `getInitialState`).
    fn initial_state(&self) -> Self::State;

    /// Tokenizes `line` (without its line break) starting in `state`
    /// (Monaco `tokenize`).
    fn tokenize(&self, line: &str, state: &Self::State) -> LineTokens<Self::State>;
}

#[derive(Clone)]
struct Line<S> {
    /// Each token's byte offset in the line and highlight (`None` for
    /// tokens without a type).
    tokens: Arc<[(u32, Option<HighlightId>)]>,
    end_state: S,
}

/// The [`HighlightState`] of a [`TokensProviderHighlighter`].
struct ProviderHighlightState<S> {
    lines: Vec<Line<S>>,
}

/// A [`Highlighter`] running a [`TokensProvider`] line by line (Monaco's
/// `TokenizationSupportAdapter`). Spans are Monaco token names interned
/// with [`HighlightId::intern_monaco_token`].
///
/// Edits re-tokenize from the first changed line until a line's end state
/// matches the one it had before, as Monaco does.
pub struct TokensProviderHighlighter<P: TokensProvider> {
    provider: Arc<P>,
}

impl<P: TokensProvider> TokensProviderHighlighter<P> {
    /// A highlighter running `provider`.
    pub fn new(provider: Arc<P>) -> Self {
        Self { provider }
    }

    /// A [`HighlighterFactory`] for
    /// [`Language::highlighter`](crate::Language::highlighter) creating
    /// highlighters that share `provider`.
    pub fn factory(provider: Arc<P>) -> HighlighterFactory {
        Arc::new(move |_, _| Arc::new(TokensProviderHighlighter::new(provider.clone())))
    }

    /// The provider.
    pub fn provider(&self) -> &Arc<P> {
        &self.provider
    }

    fn line(
        &self,
        text: &str,
        state: &P::State,
        ids: &mut HashMap<String, HighlightId>,
    ) -> Line<P::State> {
        let result = self.provider.tokenize(text, state);
        // Monaco `_toClassicTokens`: the first token starts at 0, and
        // tokens don't go backwards.
        let mut previous = 0;
        let tokens = result
            .tokens
            .into_iter()
            .enumerate()
            .map(|(ix, token)| {
                let start = if ix == 0 {
                    0
                } else {
                    token.start_index.max(previous)
                };
                previous = start;
                let highlight = (!token.scopes.is_empty()).then(|| {
                    *ids.entry(token.scopes)
                        .or_insert_with_key(|name| HighlightId::intern_monaco_token(name))
                });
                (start.min(u32::MAX as usize) as u32, highlight)
            })
            .collect();
        Line {
            tokens,
            end_state: result.end_state,
        }
    }
}

impl<P: TokensProvider> Highlighter for TokensProviderHighlighter<P> {
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
                state.downcast_ref::<ProviderHighlightState<P::State>>()?,
                old_rope,
                patch,
            ))
        });

        let mut lines: Vec<Line<P::State>> = Vec::with_capacity(line_count);
        // The old lines, the last edited row in the new text, and that row
        // in the old text: rows after it are unchanged.
        let mut tail: Option<(&ProviderHighlightState<P::State>, usize, usize)> = None;
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

        let mut ids = HashMap::new();
        let start_row = lines.len();
        let mut state = lines.last().map_or_else(
            || self.provider.initial_state(),
            |line| line.end_state.clone(),
        );
        let mut row = start_row;
        let mut converged_at = None;
        let start = rope.point_to_offset(Point::new(start_row as u32, 0));
        rope.for_each_line(start..rope.len(), |_, line| {
            let line = self.line(line, &state, &mut ids);
            state = line.end_state.clone();
            lines.push(line);
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
        HighlightState::new(ProviderHighlightState { lines })
    }

    fn highlights(
        &self,
        state: &HighlightState,
        text: &Rope,
        range: Range<usize>,
    ) -> Vec<HighlightSpan> {
        let Some(state) = state.downcast_ref::<ProviderHighlightState<P::State>>() else {
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
            for (ix, (offset, highlight)) in line.tokens.iter().enumerate() {
                let Some(highlight) = *highlight else {
                    continue;
                };
                let start = line_start + (*offset as usize).min(line_len);
                let end = line_start
                    + line
                        .tokens
                        .get(ix + 1)
                        .map_or(line_len, |(next, _)| (*next as usize).min(line_len));
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

    /// Monaco's `TokenizationSupportAdapter` tokenizes lines of any length.
    fn limits_line_length(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monaco's classic example: `/* */` comments spanning lines, and
    /// `x` words as keywords.
    struct Comments;

    impl TokensProvider for Comments {
        type State = bool;

        fn initial_state(&self) -> bool {
            false
        }

        fn tokenize(&self, line: &str, in_comment: &bool) -> LineTokens<bool> {
            let mut tokens = Vec::new();
            let mut in_comment = *in_comment;
            let mut ix = 0;
            while ix < line.len() {
                let rest = &line[ix..];
                if in_comment {
                    tokens.push(Token::new(ix, "comment"));
                    match rest.find("*/") {
                        Some(end) => {
                            in_comment = false;
                            ix += end + 2;
                        }
                        None => ix = line.len(),
                    }
                } else if rest.starts_with("/*") {
                    in_comment = true;
                } else {
                    let word_end = rest.find("/*").map_or(line.len(), |end| ix + end);
                    tokens.push(Token::new(
                        ix,
                        if rest.starts_with('x') { "keyword" } else { "" },
                    ));
                    ix = word_end;
                }
            }
            LineTokens {
                tokens,
                end_state: in_comment,
            }
        }
    }

    fn spans(text: &str) -> Vec<(String, String)> {
        let buffer = text::Buffer::new(text);
        let snapshot = buffer.snapshot();
        let highlighter = TokensProviderHighlighter::new(Arc::new(Comments));
        let state = highlighter.update(None, &snapshot);
        highlighter
            .highlights(&state, snapshot.rope(), 0..snapshot.len())
            .into_iter()
            .map(|span| {
                (
                    snapshot.rope().text_for_range(span.range.clone()),
                    span.highlight.name().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn states_carry_across_lines() {
        assert_eq!(
            spans("x /* a\nb */x"),
            [
                ("x ".to_string(), "keyword".to_string()),
                ("/* a".to_string(), "comment".to_string()),
                ("b */".to_string(), "comment".to_string()),
                ("x".to_string(), "keyword".to_string()),
            ]
        );
    }

    #[test]
    fn edits_retokenize_until_the_state_matches() {
        let mut buffer = text::Buffer::new("a\n/* b\nc */\nx");
        let highlighter = TokensProviderHighlighter::new(Arc::new(Comments));
        let old = buffer.snapshot();
        let state = highlighter.update(None, &old);
        // Closing the comment on line 2 changes how line 3 tokenizes.
        let version = buffer.version();
        buffer
            .edit([(old.len() - 7..old.len() - 7, " */")])
            .unwrap();
        let new = buffer.snapshot();
        let patch = buffer.edits_since(version);
        let state = highlighter.update(Some((&state, old.rope(), &patch)), &new);
        let names: Vec<_> = highlighter
            .highlights(&state, new.rope(), 0..new.len())
            .into_iter()
            .map(|span| {
                (
                    new.rope().text_for_range(span.range.clone()),
                    span.highlight.name().to_string(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("/* b */".to_string(), "comment".to_string()),
                ("x".to_string(), "keyword".to_string()),
            ]
        );
    }

    #[test]
    fn tokens_are_forced_into_order() {
        struct Backwards;
        impl TokensProvider for Backwards {
            type State = ();
            fn initial_state(&self) {}
            fn tokenize(&self, _: &str, _: &()) -> LineTokens<()> {
                LineTokens {
                    tokens: vec![Token::new(2, "a"), Token::new(4, "b"), Token::new(1, "c")],
                    end_state: (),
                }
            }
        }
        let buffer = text::Buffer::new("abcdef");
        let snapshot = buffer.snapshot();
        let highlighter = TokensProviderHighlighter::new(Arc::new(Backwards));
        let state = highlighter.update(None, &snapshot);
        let ranges: Vec<_> = highlighter
            .highlights(&state, snapshot.rope(), 0..6)
            .into_iter()
            .map(|span| span.range)
            .collect();
        // `a` from 0 (the first token), `b` from 4, `c` clamped to 4 (empty).
        assert_eq!(ranges, [0..4, 4..6]);
    }
}
