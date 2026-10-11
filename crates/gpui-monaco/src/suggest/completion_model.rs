//! The filtered, sorted view of a completion session's items.
//!
//! Ports `CompletionModel` and `LineContext` from Monaco's
//! `src/vs/editor/contrib/suggest/browser/completionModel.ts` (MIT), the
//! provider-order comparators (`defaultComparator`, `snippetUpComparator`,
//! `snippetDownComparator`) and item validation of `CompletionItem` from
//! `src/vs/editor/contrib/suggest/browser/suggest.ts`, and the preselection
//! of `Memory.select` (the `first` mode, `NoMemory`) from
//! `src/vs/editor/contrib/suggest/browser/suggestMemory.ts`.
//!
//! Word distance (`editor.suggest.localityBonus`) is computed by the
//! controller and passed in with [`CompletionModel::set_distances`]. Not
//! ported: grouping by provider/incomplete providers (the session tracks
//! that), and label stats.

use std::cmp::Ordering;
use std::ops::Range;

use text::PointUtf16;

use super::fuzzy::{
    FuzzyScore, FuzzyScoreOptions, Scorer, Utf16Text, any_score_utf16,
    score_graceful_aggressive_utf16, score_utf16,
};
use crate::providers::{CompletionItem, CompletionItemKind};

/// What the user has typed since the session started (Monaco `LineContext`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineContext {
    /// The cursor's line up to the cursor.
    pub leading_line_content: String,
    /// Cursor column minus the session's start column, in UTF-16 code units.
    pub character_count_delta: usize,
}

/// An item that passed the filter.
#[derive(Clone, Debug)]
pub struct ScoredCompletion {
    /// Index into the items passed to [`CompletionModel::new`].
    pub index: usize,
    /// The typed word's score against the item's filter text (or label);
    /// [`FuzzyScore::default_score`] when the word is empty or whitespace.
    pub score: FuzzyScore,
    /// Ranges (UTF-16) in the item's label to highlight.
    pub highlights: Vec<Range<usize>>,
}

/// Monaco `editor.snippetSuggestions` (`none` is the host not offering
/// snippets).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SnippetSortOrder {
    /// Snippets before other items.
    Top,
    /// Snippets after other items.
    Bottom,
    /// Snippets sorted like other items.
    #[default]
    Inline,
}

/// The editor options [`CompletionModel`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompletionModelOptions {
    /// Monaco `editor.suggest.filterGraceful`: tolerate small typos (swapped
    /// neighbouring characters) when filtering.
    pub filter_graceful: bool,
    /// Monaco `editor.snippetSuggestions`.
    pub snippet_sort_order: SnippetSortOrder,
    /// Monaco passes `firstMatchCanBeWeak: !editor.suggest.matchOnWordStartOnly`.
    pub fuzzy_score_options: FuzzyScoreOptions,
}

impl Default for CompletionModelOptions {
    /// Monaco's defaults.
    fn default() -> Self {
        Self {
            filter_graceful: true,
            snippet_sort_order: SnippetSortOrder::Inline,
            fuzzy_score_options: FuzzyScoreOptions::MONACO_DEFAULT,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Refilter {
    All,
    Incremental,
}

/// Per-item data computed once (Monaco keeps it on its `CompletionItem`).
struct PreparedItem {
    label: Utf16Text,
    filter_text: Option<Utf16Text>,
    sort_text_low: Option<Vec<u16>>,
    /// Characters of the edit range before the session's cursor
    /// (`position.column - editStart.column`); `None` for invalid items.
    overwrite_before: Option<usize>,
}

/// Sorted, filtered completion view model (Monaco `CompletionModel`).
pub struct CompletionModel {
    items: Vec<CompletionItem>,
    prepared: Vec<PreparedItem>,
    /// Item indices in provider order (Monaco sorts the provider results
    /// with `getSuggestionComparator` before building the model).
    provider_order: Vec<usize>,
    line_context: LineContext,
    options: CompletionModelOptions,
    filtered: Vec<ScoredCompletion>,
    /// Monaco's word distance per item (`suggest.localityBonus`); smaller
    /// sorts first among equal scores.
    distances: Vec<u32>,
}

impl CompletionModel {
    /// `items` with the cursor `position` where the session started (for
    /// computing each item's word from `range`, defaulting to the word before
    /// the cursor via `word_start_column`).
    pub fn new(
        items: Vec<CompletionItem>,
        position: PointUtf16,
        word_start_column: u32,
        line_context: LineContext,
    ) -> Self {
        Self::new_with_options(
            items,
            position,
            word_start_column,
            line_context,
            CompletionModelOptions::default(),
        )
    }

    /// [`CompletionModel::new`] with non-default editor options.
    pub fn new_with_options(
        items: Vec<CompletionItem>,
        position: PointUtf16,
        word_start_column: u32,
        line_context: LineContext,
        options: CompletionModelOptions,
    ) -> Self {
        let prepared: Vec<PreparedItem> = items
            .iter()
            .map(|item| prepare(item, position, word_start_column))
            .collect();
        let mut provider_order: Vec<usize> = (0..items.len()).collect();
        provider_order.sort_by(|&a, &b| {
            provider_compare(
                (&items[a], &prepared[a]),
                (&items[b], &prepared[b]),
                options.snippet_sort_order,
            )
        });
        let mut model = Self {
            items,
            prepared,
            provider_order,
            line_context,
            options,
            filtered: Vec::new(),
            distances: Vec::new(),
        };
        model.refilter(Refilter::All);
        model
    }

    /// Updates what the user typed and re-filters (Monaco's `lineContext`
    /// setter; typing more only re-scores the items that still matched).
    pub fn set_line_context(&mut self, line_context: LineContext) {
        if self.line_context == line_context {
            return;
        }
        let kind = if self.line_context.character_count_delta < line_context.character_count_delta {
            Refilter::Incremental
        } else {
            Refilter::All
        };
        self.line_context = line_context;
        self.refilter(kind);
    }

    /// What the user has typed, as last set.
    pub fn line_context(&self) -> &LineContext {
        &self.line_context
    }

    /// Sets each item's word distance (an empty list means all 0) and
    /// re-sorts.
    pub fn set_distances(&mut self, distances: Vec<u32>) {
        if self.distances != distances {
            self.distances = distances;
            self.refilter(Refilter::All);
        }
    }

    /// Item `index`'s word distance (0 without distances).
    pub fn distance(&self, index: usize) -> u32 {
        self.distances.get(index).copied().unwrap_or(0)
    }

    /// Replaces item `index` with its resolved version (Monaco assigns the
    /// resolved fields onto the item); its filtering is unchanged.
    pub fn replace_item(&mut self, index: usize, item: CompletionItem) {
        if let Some(slot) = self.items.get_mut(index) {
            *slot = item;
        }
    }

    /// All items, unfiltered, in the order they were passed in.
    pub fn items(&self) -> &[CompletionItem] {
        &self.items
    }

    /// Filtered + sorted (Monaco order: score desc, then sortText, then
    /// label; snippets first/last per [`SnippetSortOrder`]).
    pub fn filtered(&self) -> &[ScoredCompletion] {
        &self.filtered
    }

    /// The index into [`CompletionModel::filtered`] to select initially:
    /// the first `preselect` item among those sharing the top score, else 0
    /// (Monaco `Memory.select` with `editor.suggestSelection: first`).
    pub fn preselected_index(&self) -> usize {
        let Some(top) = self.filtered.first() else {
            return 0;
        };
        self.filtered
            .iter()
            .take_while(|scored| scored.score.score == top.score.score)
            .position(|scored| self.items[scored.index].preselect)
            .unwrap_or(0)
    }

    /// Monaco `_createCachedState`.
    fn refilter(&mut self, kind: Refilter) {
        let source: Vec<usize> = match kind {
            Refilter::Incremental => self.filtered.iter().map(|s| s.index).collect(),
            Refilter::All => self.provider_order.clone(),
        };

        let scorer: Scorer = if !self.options.filter_graceful || source.len() > 2000 {
            score_utf16
        } else {
            score_graceful_aggressive_utf16
        };

        let leading = Utf16Text::new(&self.line_context.leading_line_content);
        let delta = self.line_context.character_count_delta;

        // (source position, scored item); the source position is Monaco's
        // `item.idx`, the tie breaker.
        let mut target: Vec<(usize, ScoredCompletion)> = Vec::new();
        let mut word = Utf16Text::new("");
        let mut word_len_cached = None;

        for (idx, &index) in source.iter().enumerate() {
            let prepared = &self.prepared[index];
            let Some(overwrite_before) = prepared.overwrite_before else {
                continue; // skip invalid items
            };

            // The word is the remainder of the line that we filter and score
            // against; items usually share it, so it's cached.
            let word_len = overwrite_before + delta;
            if word_len_cached != Some(word_len) {
                let start = leading.len().saturating_sub(word_len);
                word = Utf16Text {
                    units: leading.units[start..].to_vec(),
                    low: leading.low[start..].to_vec(),
                };
                word_len_cached = Some(word_len);
            }

            let (score, highlights) = if word_len == 0 {
                // Nothing to score against: a constant rank, falling back to
                // the provider order.
                (FuzzyScore::default_score(), Vec::new())
            } else {
                // Skip leading whitespace of the word until we hit the
                // replace range (overwriteBefore).
                let mut word_pos = 0;
                while word_pos < overwrite_before
                    && matches!(word.units.get(word_pos), Some(0x20 | 0x09))
                {
                    word_pos += 1;
                }

                if word_pos >= word_len {
                    // Only whitespace: same as having no word.
                    (FuzzyScore::default_score(), Vec::new())
                } else if let Some(filter_text) = &prepared.filter_text {
                    // A filterText must match the word; the label is then
                    // matched separately for highlights.
                    let Some(matched) = scorer(
                        &word,
                        word_pos,
                        filter_text,
                        0,
                        self.options.fuzzy_score_options,
                    ) else {
                        continue; // no match
                    };
                    if filter_text.low == prepared.label.low {
                        // filterText and label are the same: good highlights.
                        let highlights = matched.matches.clone();
                        (matched, highlights)
                    } else {
                        // Use the filterText's rank, the label's matches.
                        let label_match = any_score_utf16(&word, word_pos, &prepared.label, 0);
                        (matched, label_match.matches)
                    }
                } else {
                    let Some(matched) = scorer(
                        &word,
                        word_pos,
                        &prepared.label,
                        0,
                        self.options.fuzzy_score_options,
                    ) else {
                        continue; // no match
                    };
                    let highlights = matched.matches.clone();
                    (matched, highlights)
                }
            };

            target.push((
                idx,
                ScoredCompletion {
                    index,
                    score,
                    highlights,
                },
            ));
        }

        let snippet_order = self.options.snippet_sort_order;
        let items = &self.items;
        let distance = |index: usize| self.distances.get(index).copied().unwrap_or(0);
        target.sort_by(|(a_idx, a), (b_idx, b)| {
            snippet_compare(items[a.index].kind, items[b.index].kind, snippet_order)
                .then_with(|| b.score.score.cmp(&a.score.score))
                .then_with(|| distance(a.index).cmp(&distance(b.index)))
                .then_with(|| a_idx.cmp(b_idx))
        });
        self.filtered = target.into_iter().map(|(_, scored)| scored).collect();
    }
}

fn prepare(item: &CompletionItem, position: PointUtf16, word_start_column: u32) -> PreparedItem {
    let label = Utf16Text::new(&item.label);
    let overwrite_before = if item.label.is_empty() {
        None
    } else {
        match &item.range {
            // Monaco: ranges must be single-line, on the cursor's line.
            Some(range) if range.start.row != position.row || range.end.row != range.start.row => {
                None
            }
            Some(range) => Some(position.column.saturating_sub(range.start.column) as usize),
            None => Some(position.column.saturating_sub(word_start_column) as usize),
        }
    };
    PreparedItem {
        label,
        filter_text: item.filter_text.as_deref().map(Utf16Text::new),
        sort_text_low: item
            .sort_text
            .as_deref()
            .map(|sort_text| Utf16Text::new(sort_text).low),
        overwrite_before,
    }
}

/// Monaco `getSuggestionComparator(snippetSortOrder)` over provider results.
fn provider_compare(
    (a, a_prepared): (&CompletionItem, &PreparedItem),
    (b, b_prepared): (&CompletionItem, &PreparedItem),
    snippet_order: SnippetSortOrder,
) -> Ordering {
    snippet_compare(a.kind, b.kind, snippet_order).then_with(|| {
        // defaultComparator: sortText (when both have one), label, kind.
        let by_sort_text = match (&a_prepared.sort_text_low, &b_prepared.sort_text_low) {
            (Some(a), Some(b)) => a.cmp(b),
            _ => Ordering::Equal,
        };
        by_sort_text
            .then_with(|| a_prepared.label.units.cmp(&b_prepared.label.units))
            .then_with(|| (a.kind as u8).cmp(&(b.kind as u8)))
    })
}

fn snippet_compare(
    a: CompletionItemKind,
    b: CompletionItemKind,
    order: SnippetSortOrder,
) -> Ordering {
    let snippet_first = match order {
        SnippetSortOrder::Top => Ordering::Less,
        SnippetSortOrder::Bottom => Ordering::Greater,
        SnippetSortOrder::Inline => return Ordering::Equal,
    };
    match (
        a == CompletionItemKind::Snippet,
        b == CompletionItemKind::Snippet,
    ) {
        (true, false) => snippet_first,
        (false, true) => snippet_first.reverse(),
        _ => Ordering::Equal,
    }
}
