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
use crate::view::bracket_pairs::{BracketToken, colorize_brackets};

#[derive(Default)]
pub(crate) struct BracketColors {
    tokens: Arc<Vec<BracketToken>>,
    /// The text version `tokens` belong to.
    version: Option<text::Version>,
    /// The version the running `task` computes.
    pending: Option<text::Version>,
    task: Option<Task<()>>,
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
        let Some(pairs) = pairs.filter(|p| !p.is_empty() && self.options.bracket_pair_colorization)
        else {
            self.bracket_colors = BracketColors::default();
            return;
        };
        let version = model.buffer().version();
        if self.bracket_colors.version == Some(version)
            || self.bracket_colors.pending == Some(version)
        {
            return;
        }
        let syntax = model.syntax_snapshot();
        // Wait for the parse of this version (`SyntaxUpdated` calls back).
        if syntax.as_ref().is_some_and(|s| s.text.version() != version) || model.is_parsing() {
            return;
        }
        let rope = model.buffer().rope().clone();
        self.bracket_colors.pending = Some(version);
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
            let tokens = cx
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
                    colorize_brackets(&rope, &pairs, &skip)
                })
                .await;
            this.update(cx, |this, cx| {
                this.bracket_colors.tokens = Arc::new(tokens);
                this.bracket_colors.version = Some(version);
                this.bracket_colors.pending = None;
                this.bracket_colors.task = None;
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
}
