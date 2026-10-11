//! Occurrence highlighting: the symbol at the cursor (Monaco's
//! `contrib/wordHighlighter`, with its textual fallback provider) and other
//! occurrences of the selected text (`SelectionHighlighter` in
//! `contrib/multicursor`).

use std::ops::Range;
use std::time::Duration;

use gpui::{Context, Task};
use text::PointUtf16;

use crate::cursor::{self, Selection};
use crate::decorations::{DecorationId, DecorationOptions, OverviewRulerLane};
use crate::editor::CodeEditor;
use crate::providers::{DocumentHighlight, DocumentHighlightKind, document_highlight_providers};
use crate::search::{self, SearchParams};

/// Monaco's `SelectionHighlighter` waits this long after edits.
const SELECTION_UPDATE_DELAY: Duration = Duration::from_millis(300);

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
                let (background, overview) = match h.kind {
                    DocumentHighlightKind::Write => (
                        colors.word_highlight_strong_background,
                        colors.word_highlight_strong_overview,
                    ),
                    _ => (
                        colors.word_highlight_background,
                        colors.word_highlight_overview,
                    ),
                };
                (
                    range,
                    DecorationOptions {
                        background: Some(background),
                        overview_ruler: Some((overview, OverviewRulerLane::Center)),
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

    /// Monaco `WordHighlighter._run`: after the cursor rests in a word,
    /// highlight the symbol's occurrences from the document highlight
    /// providers, or the word's textual occurrences.
    fn update_word_highlights(&mut self, force: bool, cx: &mut Context<Self>) {
        // Monaco has no word highlighter for models too large to tokenize.
        if (self.options.occurrences_highlight == crate::options::OccurrencesHighlight::Off
            && !force)
            || self.model().read(cx).is_too_large_for_tokenization()
        {
            self.clear_word_highlights(cx);
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
        let language = model.language_id().map(|l| l.to_string());
        let providers = document_highlight_providers(cx, language.as_deref());
        let rope = self.snapshot.rope().clone();
        let text = rope.text_for_range(
            rope.point_utf16_to_offset(word.start)..rope.point_utf16_to_offset(word.end),
        );
        let delay = if force {
            Duration::ZERO
        } else {
            self.options.occurrences_highlight_delay
        };
        self.occurrences.word_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(tasks) = this.update(cx, |_, cx| {
                providers
                    .iter()
                    .map(|p| p.provide_document_highlights(&context, head, cx))
                    .collect::<Vec<_>>()
            }) else {
                return;
            };
            let mut highlights = None;
            for task in tasks {
                if let Ok(Some(found)) = task.await
                    && !found.is_empty()
                {
                    highlights = Some(found);
                    break;
                }
            }
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

    /// Highlights the symbol's occurrences right away, even with
    /// [`EditorOptions::occurrences_highlight`](crate::EditorOptions::occurrences_highlight)
    /// off (Monaco `editor.action.wordHighlight.trigger`).
    pub fn trigger_word_highlight(&mut self, cx: &mut Context<Self>) {
        self.update_word_highlights(true, cx);
    }

    /// Selects the next (or previous) highlighted occurrence (Monaco
    /// `editor.action.wordHighlight.next` / `.prev`, F7 / Shift+F7).
    pub fn go_to_word_highlight(&mut self, next: bool, cx: &mut Context<Self>) {
        let ranges = self.word_highlight_ranges(cx);
        if ranges.is_empty() {
            return;
        }
        let head = self.cursors.primary().position;
        let current = ranges.iter().position(|r| r.start <= head && head <= r.end);
        let index = match (current, next) {
            (Some(i), true) => (i + 1) % ranges.len(),
            (Some(i), false) => (i + ranges.len() - 1) % ranges.len(),
            (None, true) => ranges.iter().position(|r| r.start > head).unwrap_or(0),
            (None, false) => ranges
                .iter()
                .rposition(|r| r.end < head)
                .unwrap_or(ranges.len() - 1),
        };
        let range = ranges[index].clone();
        self.set_selections(
            &[Selection {
                anchor: range.start,
                head: range.end,
            }],
            cx,
        );
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
                        overview_ruler: Some((
                            colors.selection_highlight_overview,
                            OverviewRulerLane::Center,
                        )),
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
        let text = text_of(&primary);
        // Same as Cmd+D: selected text matches per
        // `selected_text_match_mode`; never whole-word here.
        let (_, match_case) = self.selected_text_match_options();
        let selections = self.cursors.selections();
        if selections.len() > 1 {
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
                && !find.whole_word
                && !find.is_regex
            {
                return None;
            }
        }
        Some(SearchParams::new(text, false, match_case, None))
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
