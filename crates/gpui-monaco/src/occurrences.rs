//! Occurrence highlighting: the symbol at the cursor (Monaco's
//! `contrib/wordHighlighter`, with its textual fallback provider) and other
//! occurrences of the selected text (`SelectionHighlighter` in
//! `contrib/multicursor`).

use std::ops::Range;
use std::time::Duration;

use gpui::{Context, Task};
use text::PointUtf16;

use crate::cursor::{self, Selection};
use crate::decorations::{
    DecorationId, DecorationOptions, OverviewRulerDecoration, OverviewRulerLane,
};
use crate::editor::CodeEditor;
use crate::providers::{DocumentHighlight, DocumentHighlightKind, document_highlight_providers};
use crate::search::{self, SearchParams};

/// Monaco's `SelectionHighlighter` waits this long after edits.
const SELECTION_UPDATE_DELAY: Duration = Duration::from_millis(300);

/// Monaco `WordHighlighter.runDelayer`: the word highlights are computed
/// once the cursor has rested this long.
const WORD_HIGHLIGHT_RUN_DELAY: Duration = Duration::from_millis(50);

/// Monaco `mergeHighlights`: the answers in provider order, keeping the
/// first highlight of each range.
fn merge_highlights(answers: Vec<Vec<DocumentHighlight>>) -> Vec<DocumentHighlight> {
    let mut seen = std::collections::HashSet::new();
    answers
        .into_iter()
        .flatten()
        .filter(|h| seen.insert((h.range.start, h.range.end)))
        .collect()
}

#[derive(Default)]
pub(crate) struct OccurrenceHighlights {
    /// Word highlights: decorations and their kinds' ranges at `word_version`.
    word: Vec<DecorationId>,
    word_task: Option<Task<()>>,
    /// The text version the word highlights were computed for (edits clear
    /// them, like Monaco's `onDidChangeModelContent`).
    word_version: Option<text::Version>,
    selection: Vec<DecorationId>,
    selection_task: Option<Task<()>>,
    /// The text version at the last cursor change, to tell edits (which
    /// don't start word highlights, like Monaco's non-explicit cursor
    /// changes) from cursor moves.
    seen_version: Option<text::Version>,
}

impl CodeEditor {
    fn word_pattern(&self, cx: &gpui::App) -> Option<syntax::Pattern> {
        self.model()
            .read(cx)
            .language()
            .and_then(|l| l.config.word_pattern.clone())
    }

    /// The word containing `position` (Monaco `model.getWordAtPosition`).
    fn word_range_at(&self, position: PointUtf16, cx: &gpui::App) -> Option<Range<PointUtf16>> {
        let pattern = self.word_pattern(cx);
        let regex = match pattern.as_ref().and_then(syntax::Pattern::regex) {
            Some(regex) => regex,
            None => cursor::in_place_replace::default_word_regex(),
        };
        let line = self.snapshot.rope().line(position.row);
        let columns = cursor::in_place_replace::word_at(&line, position.column, regex)?;
        Some(
            PointUtf16::new(position.row, columns.start)
                ..PointUtf16::new(position.row, columns.end),
        )
    }

    fn set_word_highlights(&mut self, highlights: Vec<DocumentHighlight>, cx: &mut Context<Self>) {
        let rope = self.snapshot.rope().clone();
        let colors = self.theme.colors.clone();
        let new = highlights
            .into_iter()
            .map(|h| {
                let range = rope.point_utf16_to_offset(h.range.start)
                    ..rope.point_utf16_to_offset(h.range.end);
                let (background, border, overview) = match h.kind {
                    DocumentHighlightKind::Write => (
                        colors.word_highlight_strong_background,
                        colors.word_highlight_strong_border,
                        colors.word_highlight_strong_overview,
                    ),
                    DocumentHighlightKind::Text => (
                        colors.word_highlight_text_background,
                        colors.word_highlight_text_border,
                        colors.word_highlight_text_overview,
                    ),
                    _ => (
                        colors.word_highlight_background,
                        colors.word_highlight_border,
                        colors.word_highlight_overview,
                    ),
                };
                (
                    range,
                    DecorationOptions {
                        background: Some(background),
                        border,
                        overview_ruler: Some(OverviewRulerDecoration::new(
                            overview,
                            OverviewRulerLane::Center,
                        )),
                        // Monaco `minimap.selectionOccurrenceHighlight`.
                        minimap: Some(crate::MinimapDecoration {
                            color: colors
                                .minimap_selection_occurrence_highlight
                                .map(Into::into),
                            ..Default::default()
                        }),
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let old = std::mem::take(&mut self.occurrences.word);
        let owner = Some(cx.entity_id());
        self.occurrences.word = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, new, cx)
        });
        cx.notify();
    }

    fn clear_word_highlights(&mut self, cx: &mut Context<Self>) {
        self.occurrences.word_task = None;
        if !self.occurrences.word.is_empty() {
            self.set_word_highlights(Vec::new(), cx);
        }
    }

    /// Whether symbol occurrences are highlighted (Monaco
    /// `hasWordHighlights`).
    pub(crate) fn has_word_highlights(&self) -> bool {
        !self.occurrences.word.is_empty()
    }

    /// The current word highlight ranges, in order.
    fn word_highlight_ranges(&self, cx: &gpui::App) -> Vec<Range<PointUtf16>> {
        let model = self.model().read(cx);
        let buffer = model.buffer();
        let mut ranges: Vec<Range<PointUtf16>> = self
            .occurrences
            .word
            .iter()
            .filter_map(|id| model.decoration(*id))
            .map(|d| {
                buffer.offset_to_point_utf16(d.range.start)
                    ..buffer.offset_to_point_utf16(d.range.end)
            })
            .collect();
        ranges.sort_by_key(|r| r.start);
        ranges
    }

    /// Monaco `WordHighlighter._run`: after the cursor rests in a word
    /// (50ms), highlight the symbol's occurrences from the document
    /// highlight providers, or the word's textual occurrences, no sooner
    /// than `occurrences_highlight_delay` after the cursor moved (Monaco
    /// `_beginRenderDecorations`).
    fn update_word_highlights(&mut self, force: bool, cx: &mut Context<Self>) {
        // Monaco has no word highlighter for models too large to tokenize.
        if (self.options.occurrences_highlight == crate::options::OccurrencesHighlight::Off
            && !force)
            || self.model().read(cx).is_too_large_for_tokenization()
        {
            self.clear_word_highlights(cx);
            return;
        }
        // Monaco `_run`: only with text focus (the highlights from before
        // stay while another editor has it).
        if !force && !self.has_focus {
            return;
        }
        let selection = self.cursors.primary().selection();
        if selection.start().row != selection.end().row {
            self.clear_word_highlights(cx);
            return;
        }
        let Some(word) = self.word_range_at(selection.start(), cx) else {
            self.clear_word_highlights(cx);
            return;
        };
        if word.start > selection.start() || word.end < selection.end() {
            self.clear_word_highlights(cx);
            return;
        }
        // Still on one of the highlighted occurrences: keep them.
        let head = selection.head;
        let version = self.snapshot.version();
        if !force
            && self.occurrences.word_version == Some(version)
            && self
                .word_highlight_ranges(cx)
                .iter()
                .any(|r| r.start <= head && head <= r.end)
        {
            return;
        }
        self.clear_word_highlights(cx);

        let model = self.model().read(cx);
        let context = model.provider_context();
        let providers = document_highlight_providers(cx, self.provider_target(cx));
        let rope = self.snapshot.rope().clone();
        let text = rope.text_for_range(
            rope.point_utf16_to_offset(word.start)..rope.point_utf16_to_offset(word.end),
        );
        let (run_delay, render_delay) = if force {
            (Duration::ZERO, Duration::ZERO)
        } else {
            (
                WORD_HIGHLIGHT_RUN_DELAY,
                self.options.occurrences_highlight_delay,
            )
        };
        let moved = cx.background_executor().now();
        let merge = self.options.occurrences_highlight_from_all_providers;
        self.occurrences.word_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(run_delay).await;
            let highlights = if merge && !providers.is_empty() {
                // Monaco `getOccurrencesAtPosition` with `mergeProviders`:
                // every provider at once, their answers merged in provider
                // order without repeated ranges. The textual highlights are
                // only for when no provider matches.
                let Ok(tasks) = this.update(cx, |_, cx| {
                    providers
                        .iter()
                        .map(|p| p.provide_document_highlights(&context, head, cx))
                        .collect::<Vec<_>>()
                }) else {
                    return;
                };
                let mut answers = Vec::new();
                for task in tasks {
                    if let Ok(Some(found)) = task.await {
                        answers.push(found);
                    }
                }
                Some(merge_highlights(answers))
            } else {
                // Monaco asks the providers in turn until one answers; an
                // empty answer counts, and stops the search too.
                let mut highlights = None;
                for provider in &providers {
                    let Ok(task) = this.update(cx, |_, cx| {
                        provider.provide_document_highlights(&context, head, cx)
                    }) else {
                        return;
                    };
                    if let Ok(Some(found)) = task.await {
                        highlights = Some(found);
                        break;
                    }
                }
                highlights
            };
            // Monaco's `TextualDocumentHighlightProvider`: case-sensitive,
            // whole-word matches of the word.
            let highlights = match highlights {
                Some(highlights) => highlights,
                None => {
                    cx.background_executor()
                        .spawn(async move {
                            let Some(data) = SearchParams::new(
                                text,
                                false,
                                true,
                                Some(cursor::USUAL_WORD_SEPARATORS),
                            )
                            .parse() else {
                                return Vec::new();
                            };
                            search::find_matches(
                                &rope,
                                &data,
                                0..rope.len(),
                                crate::find::MATCHES_LIMIT,
                            )
                            .into_iter()
                            .map(|m| DocumentHighlight {
                                range: rope.offset_to_point_utf16(m.start)
                                    ..rope.offset_to_point_utf16(m.end),
                                kind: DocumentHighlightKind::Text,
                            })
                            .collect()
                        })
                        .await
                }
            };
            // Monaco `_beginRenderDecorations`: the highlights show no
            // sooner than the delay after the cursor moved.
            let elapsed = cx.background_executor().now() - moved;
            if let Some(wait) = render_delay.checked_sub(elapsed)
                && !wait.is_zero()
            {
                cx.background_executor().timer(wait).await;
            }
            this.update(cx, |this, cx| {
                if this.snapshot.version() != version {
                    return;
                }
                this.occurrences.word_version = Some(version);
                this.set_word_highlights(highlights, cx);
            })
            .ok();
        }));
    }

    /// Monaco `WordHighlighter`'s `_stopAll` then `_run`: drops the word
    /// highlights and computes them again (unless occurrence highlighting is
    /// off).
    pub(crate) fn restart_word_highlights(&mut self, cx: &mut Context<Self>) {
        self.occurrences.word_version = None;
        self.clear_word_highlights(cx);
        if self.options.occurrences_highlight != crate::options::OccurrencesHighlight::Off {
            self.update_word_highlights(false, cx);
        }
    }

    /// Monaco `WordHighlighter`'s focus listeners: focusing the text
    /// highlights the symbol there; focus leaving the editor and its widgets
    /// drops the highlights unless another editor took it.
    pub(crate) fn word_highlights_focus_changed(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.options.occurrences_highlight == crate::options::OccurrencesHighlight::Off {
            return;
        }
        if focused {
            if self.occurrences.word_task.is_none() {
                self.update_word_highlights(false, cx);
            }
            return;
        }
        let id = cx.entity_id();
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| {
                let other_focused = crate::editors(cx)
                    .iter()
                    .any(|editor| editor.entity_id() != id && editor.read(cx).has_focus);
                if !this.has_focus && !other_focused {
                    this.clear_word_highlights(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Highlights the symbol's occurrences right away, even with
    /// [`EditorOptions::occurrences_highlight`](crate::EditorOptions::occurrences_highlight)
    /// off (Monaco `editor.action.wordHighlight.trigger`).
    pub fn trigger_word_highlight(&mut self, cx: &mut Context<Self>) {
        self.update_word_highlights(true, cx);
    }

    /// Moves the caret to the start of the next (or previous) highlighted
    /// occurrence, revealing it, and reads it to screen readers (Monaco
    /// `WordHighlighter.moveNext` / `moveBack`,
    /// `editor.action.wordHighlight.next` / `.prev`, F7 / Shift+F7).
    ///
    /// The highlights are taken in order from the one containing the caret.
    /// From outside every highlight, F7 goes to the first; Shift+F7 keeps
    /// Monaco's arithmetic (`(-1 - 1 + n) % n`), which lands on the second
    /// to last of two or more.
    pub fn go_to_word_highlight(&mut self, next: bool, cx: &mut Context<Self>) {
        let ranges = self.word_highlight_ranges(cx);
        if ranges.is_empty() {
            return;
        }
        let head = self.cursors.primary().position;
        let len = ranges.len() as i64;
        // Monaco `Range.containsPosition`: both ends included.
        let index = ranges
            .iter()
            .position(|r| r.start <= head && head <= r.end)
            .map_or(-1, |i| i as i64);
        let new_index = if next {
            (index + 1).rem_euclid(len)
        } else {
            (index - 1 + len).rem_euclid(len)
        } as usize;
        let dest = ranges[new_index].clone();
        self.set_selections(&[Selection::collapsed(dest.start)], cx);
        self.reveal_range_in_center_if_outside_viewport(dest.clone(), cx);
        let word = self.model().read(cx).word_at_position(dest.start);
        if let Some(word) = word {
            let line = self.snapshot.rope().line(dest.start.row);
            let message = format!(
                "{line}, {} of {} for '{}'",
                new_index + 1,
                ranges.len(),
                word.word
            );
            self.announce(message, cx);
        }
    }

    // ----- selection highlights -----------------------------------------------

    fn set_selection_highlights(&mut self, ranges: Vec<Range<usize>>, cx: &mut Context<Self>) {
        let colors = self.theme.colors.clone();
        let new = ranges
            .into_iter()
            .map(|range| {
                (
                    range,
                    DecorationOptions {
                        background: Some(colors.selection_highlight_background),
                        border: colors.selection_highlight_border,
                        overview_ruler: Some(OverviewRulerDecoration::new(
                            colors.selection_highlight_overview,
                            OverviewRulerLane::Center,
                        )),
                        minimap: Some(crate::MinimapDecoration {
                            color: colors
                                .minimap_selection_occurrence_highlight
                                .map(Into::into),
                            ..Default::default()
                        }),
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let old = std::mem::take(&mut self.occurrences.selection);
        let owner = Some(cx.entity_id());
        self.occurrences.selection = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, new, cx)
        });
        cx.notify();
    }

    /// Monaco `SelectionHighlighter._createState`: what to search for, or
    /// `None` when the selection shouldn't highlight anything.
    fn selection_highlight_search(&self, cx: &gpui::App) -> Option<SearchParams> {
        if !self.options.selection_highlight {
            return None;
        }
        // Monaco doesn't search models too large to tokenize.
        if self.model().read(cx).is_too_large_for_tokenization() {
            return None;
        }
        let primary = self.cursors.primary().selection();
        if primary.is_empty() {
            return None;
        }
        if !self.options.selection_highlight_multiline && primary.start().row != primary.end().row {
            return None;
        }
        let rope = self.snapshot.rope();
        let text_of = |s: &Selection| {
            rope.text_for_range(
                rope.point_utf16_to_offset(s.start())..rope.point_utf16_to_offset(s.end()),
            )
        };
        // A running Cmd/Ctrl+D session's search (whole-word after starting
        // from a word); otherwise the selected text, matched as Cmd+D
        // would (Monaco `_createState` reuses the session or creates one).
        let (text, whole_word, match_case) = match self.multi_cursor_session() {
            Some((text, whole_word, match_case)) => (text.to_string(), whole_word, match_case),
            None => {
                let (whole_word, match_case) = self.selected_text_match_options();
                (text_of(&primary), whole_word, match_case)
            }
        };
        let selections = self.cursors.selections();
        if self.multi_cursor_session().is_none() && selections.len() > 1 {
            let same = selections.iter().all(|s| {
                let other = text_of(s);
                if match_case {
                    other == text
                } else {
                    other.to_lowercase() == text.to_lowercase()
                }
            });
            if !same {
                return None;
            }
        }
        if text.chars().all(|c| c == ' ' || c == '\t') {
            return None;
        }
        let max = self.options.selection_highlight_max_length as usize;
        if max > 0 && text.encode_utf16().count() > max {
            return None;
        }
        // The find widget already shows these matches.
        if self.find.visible {
            let normalize = |s: &str| {
                if match_case {
                    s.to_string()
                } else {
                    s.to_lowercase()
                }
            };
            let find = &self.find.options;
            if normalize(&find.query) == normalize(&text)
                && find.match_case == match_case
                && find.whole_word == whole_word
                && !find.is_regex
            {
                return None;
            }
        }
        let separators = whole_word.then(|| self.options.word_separators.to_string());
        Some(SearchParams::new(
            text,
            false,
            match_case,
            separators.as_deref(),
        ))
    }

    /// Recomputes the selection highlights, after `delay`.
    fn update_selection_highlights(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let Some(params) = self.selection_highlight_search(cx) else {
            self.occurrences.selection_task = None;
            if !self.occurrences.selection.is_empty() {
                self.set_selection_highlights(Vec::new(), cx);
            }
            return;
        };
        let rope = self.snapshot.rope().clone();
        let version = self.snapshot.version();
        let selected: Vec<Range<usize>> = self
            .cursors
            .selections()
            .iter()
            .map(|s| rope.point_utf16_to_offset(s.start())..rope.point_utf16_to_offset(s.end()))
            .collect();
        self.occurrences.selection_task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let matches = cx
                .background_executor()
                .spawn(async move {
                    let Some(data) = params.parse() else {
                        return Vec::new();
                    };
                    search::find_matches(&rope, &data, 0..rope.len(), crate::find::MATCHES_LIMIT)
                        .into_iter()
                        // Don't cover the selections themselves.
                        .filter(|m| !selected.iter().any(|s| s.start < m.end && m.start < s.end))
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                if this.snapshot.version() == version {
                    this.set_selection_highlights(matches, cx);
                }
            })
            .ok();
        }));
    }

    /// Updates both kinds of occurrence highlights after the cursors moved.
    pub(crate) fn update_occurrence_highlights(&mut self, cx: &mut Context<Self>) {
        let version = self.snapshot.version();
        let edited = self.occurrences.seen_version.is_some_and(|v| v != version);
        self.occurrences.seen_version = Some(version);
        if edited {
            // Monaco stops word highlights on edits until the cursor moves
            // explicitly again.
            self.occurrences.word_version = None;
            self.clear_word_highlights(cx);
        } else {
            self.update_word_highlights(false, cx);
        }
        self.update_selection_highlights(Duration::ZERO, cx);
    }

    /// After an edit: word highlights stop, selection highlights refresh
    /// shortly (Monaco's `onDidChangeModelContent` handlers).
    pub(crate) fn occurrences_after_edit(&mut self, cx: &mut Context<Self>) {
        self.occurrences.word_version = None;
        self.clear_word_highlights(cx);
        self.update_selection_highlights(SELECTION_UPDATE_DELAY, cx);
    }

    /// Drops the highlights from the current model (before switching models).
    pub(crate) fn clear_occurrence_highlights(&mut self, cx: &mut Context<Self>) {
        self.clear_word_highlights(cx);
        self.occurrences.word_version = None;
        self.occurrences.seen_version = None;
        self.occurrences.selection_task = None;
        if !self.occurrences.selection.is_empty() {
            self.set_selection_highlights(Vec::new(), cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(start: u32, end: u32, kind: DocumentHighlightKind) -> DocumentHighlight {
        DocumentHighlight {
            range: PointUtf16::new(0, start)..PointUtf16::new(0, end),
            kind,
        }
    }

    /// Monaco `wordHighlighter.test.ts` "queries different priorities
    /// concurrently and deduplicates ranges in provider order".
    #[test]
    fn merged_highlights_keep_the_first_of_each_range() {
        use DocumentHighlightKind::*;
        let (first, second) = (h(0, 4, Read), h(5, 9, Write));
        let (text, overlapping) = (h(10, 14, Text), h(1, 4, Write));
        let higher = vec![first.clone(), first.clone(), text.clone()];
        let lower = vec![
            h(0, 4, Write),
            second.clone(),
            h(10, 14, Read),
            overlapping.clone(),
        ];
        assert_eq!(
            merge_highlights(vec![higher, lower]),
            vec![first, text, second, overlapping]
        );
        assert_eq!(merge_highlights(Vec::new()), Vec::new());
    }
}
