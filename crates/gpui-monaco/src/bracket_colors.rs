//! Bracket pair colorization (Monaco `bracketPairColorization`): brackets
//! are colored by nesting level, unmatched ones in
//! `editorBracketHighlight.unexpectedBracket.foreground`.
//!
//! The nesting is computed for the whole document on a background thread
//! once the syntax state matches the text (so brackets in strings and
//! comments are skipped), and shifted through newer edits until the next
//! pass lands.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Hsla, Task};

use crate::editor::CodeEditor;
use crate::view::bracket_pairs::{BracketPair, BracketToken, bracket_pairs, colorize_brackets};
use crate::view::guides::GuidePair;

#[derive(Default)]
pub(crate) struct BracketColors {
    tokens: Arc<Vec<BracketToken>>,
    /// The closed pairs, for bracket pair guides.
    pairs: Arc<Vec<BracketPair>>,
    /// Whether `pairs` were computed (only with guides on), and whether
    /// the running task computes them.
    has_pairs: bool,
    pending_pairs: bool,
    /// The text version `tokens` belong to.
    version: Option<text::Version>,
    /// The version the running `task` computes.
    pending: Option<text::Version>,
    task: Option<Task<()>>,
    /// A background pass is running: dropping `task` wouldn't stop it, so
    /// newer versions wait for it (huge documents take seconds).
    computing: bool,
    /// A newer version came in while `computing`.
    rerun: bool,
}

impl CodeEditor {
    /// Recomputes bracket nesting if the text or its syntax changed.
    pub(crate) fn refresh_bracket_colors(&mut self, cx: &mut Context<Self>) {
        let model = self.model().read(cx);
        let pairs = model.language().map(|language| {
            let config = &language.config;
            config
                .colorized_bracket_pairs
                .clone()
                .unwrap_or_else(|| config.brackets.clone())
        });
        let guides = self.options.guides.bracket_pairs != crate::options::BracketPairGuides::Off;
        let Some(pairs) = pairs.filter(|p| {
            !p.is_empty() && (self.options.bracket_pair_colorization.enabled || guides)
        }) else {
            self.bracket_colors = BracketColors::default();
            return;
        };
        let version = model.buffer().version();
        let state = &self.bracket_colors;
        if (state.version == Some(version) && (state.has_pairs || !guides))
            || (state.pending == Some(version) && (state.pending_pairs || !guides))
        {
            return;
        }
        let syntax = model.syntax_snapshot();
        // Wait for the parse of this version (`SyntaxUpdated` calls back).
        if syntax.as_ref().is_some_and(|s| s.text.version() != version) || model.is_parsing() {
            return;
        }
        if self.bracket_colors.computing {
            self.bracket_colors.rerun = true;
            return;
        }
        let rope = model.buffer().rope().clone();
        let tab_size = model.options().tab_size;
        self.bracket_colors.pending = Some(version);
        self.bracket_colors.pending_pairs = guides;
        // The first pass runs right away; later ones wait for a pause in
        // typing (stale colors are shifted through the edits meanwhile).
        let delay = if self.bracket_colors.version.is_some() {
            Duration::from_millis(150)
        } else {
            Duration::ZERO
        };
        self.bracket_colors.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            if this
                .update(cx, |this, _| this.bracket_colors.computing = true)
                .is_err()
            {
                return;
            }
            let (tokens, pairs) = cx
                .background_executor()
                .spawn(async move {
                    let skip: Vec<Range<usize>> = syntax
                        .map(|syntax| {
                            syntax
                                .highlighter
                                .highlights(&syntax.state, syntax.text.rope(), 0..rope.len())
                                .into_iter()
                                .filter(|span| {
                                    let name = span.highlight.name();
                                    name.starts_with("comment")
                                        || name.starts_with("string")
                                        || name.starts_with("character")
                                })
                                .map(|span| span.range)
                                .collect()
                        })
                        .unwrap_or_default();
                    if guides {
                        bracket_pairs(&rope, &pairs, &skip, tab_size)
                    } else {
                        (colorize_brackets(&rope, &pairs, &skip), Vec::new())
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.bracket_colors.tokens = Arc::new(tokens);
                this.bracket_colors.pairs = Arc::new(pairs);
                this.bracket_colors.has_pairs = guides;
                this.bracket_colors.version = Some(version);
                this.bracket_colors.pending = None;
                this.bracket_colors.task = None;
                this.bracket_colors.computing = false;
                if std::mem::take(&mut this.bracket_colors.rerun) {
                    this.refresh_bracket_colors(cx);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The colorized brackets of the whole text (for tests).
    #[doc(hidden)]
    pub fn bracket_colors_for_tests(&self, cx: &App) -> Vec<(Range<usize>, Hsla)> {
        let len = self.model().read(cx).buffer().len();
        self.bracket_colors_in(0..len, cx)
    }

    /// Bracket colors for byte `range` of the current text.
    pub(crate) fn bracket_colors_in(
        &self,
        range: Range<usize>,
        cx: &App,
    ) -> Vec<(Range<usize>, Hsla)> {
        let Some(version) = self.bracket_colors.version else {
            return Vec::new();
        };
        if !self.options.bracket_pair_colorization.enabled {
            return Vec::new();
        }
        let independent = self
            .options
            .bracket_pair_colorization
            .independent_color_pool_per_bracket_type;
        let tokens = &self.bracket_colors.tokens;
        let buffer = self.model().read(cx).buffer();
        let patch = (version != buffer.version()).then(|| buffer.edits_since(version));
        // The range in the tokens' (older) coordinates.
        let old_range = match &patch {
            Some(patch) => {
                let inverse = patch.invert();
                inverse.transform_offset(range.start, text::Bias::Left)
                    ..inverse.transform_offset(range.end, text::Bias::Right)
            }
            None => range,
        };
        let first = tokens.partition_point(|t| t.range.end <= old_range.start);
        let colors = &self.theme.colors;
        tokens[first..]
            .iter()
            .take_while(|t| t.range.start < old_range.end)
            .filter_map(|t| {
                let color = match t.level {
                    // Monaco `getInlineClassName`: each bracket type's own
                    // levels with `independentColorPoolPerBracketType`.
                    Some(_) if independent => {
                        colors.bracket_color(t.level_of_equal_bracket_type)?
                    }
                    Some(level) => colors.bracket_color(level)?,
                    None => colors.unexpected_bracket,
                };
                let range = match &patch {
                    Some(patch) => {
                        patch.transform_offset(t.range.start, text::Bias::Right)
                            ..patch.transform_offset(t.range.end, text::Bias::Left)
                    }
                    None => t.range.clone(),
                };
                (range.start < range.end).then_some((range, color))
            })
            .collect()
    }

    /// The closed bracket pairs intersecting `rows`, in current
    /// coordinates (shifted through edits since they were found).
    pub(crate) fn bracket_pairs_in_rows(&self, rows: Range<u32>, cx: &App) -> Vec<GuidePair> {
        let Some(version) = self.bracket_colors.version else {
            return Vec::new();
        };
        let buffer = self.model().read(cx).buffer();
        let rope = buffer.rope();
        let patch = (version != buffer.version()).then(|| buffer.edits_since(version));
        let start = rope.point_to_offset(text::Point::new(rows.start, 0));
        let end = rope.point_to_offset(text::Point::new(rows.end, 0).min(rope.max_point()));
        let (old_start, old_end) = match &patch {
            Some(patch) => {
                let inverse = patch.invert();
                (
                    inverse.transform_offset(start, text::Bias::Left),
                    inverse.transform_offset(end, text::Bias::Right),
                )
            }
            None => (start, end),
        };
        let map = |offset: usize, bias: text::Bias| match &patch {
            Some(patch) => patch.transform_offset(offset, bias),
            None => offset,
        };
        let point = |offset: usize| rope.offset_to_point_utf16(offset.min(rope.len()));
        let pairs = &self.bracket_colors.pairs;
        let independent = self
            .options
            .bracket_pair_colorization
            .independent_color_pool_per_bracket_type;
        let last = pairs.partition_point(|p| p.open.start <= old_end);
        pairs[..last]
            .iter()
            .filter(|p| p.close.end >= old_start)
            .map(|p| GuidePair {
                open: point(map(p.open.start, text::Bias::Left))
                    ..point(map(p.open.end, text::Bias::Right)),
                close: point(map(p.close.start, text::Bias::Left))
                    ..point(map(p.close.end, text::Bias::Right)),
                // Colored like the brackets (Monaco's guides use the same
                // `getInlineClassName`).
                level: if independent {
                    p.level_of_equal_bracket_type
                } else {
                    p.level
                },
                min_indentation: p.min_indentation,
            })
            .collect()
    }
}
