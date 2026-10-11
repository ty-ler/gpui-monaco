//! The suggest controller (Monaco's `contrib/suggest` `SuggestController`
//! and `SuggestModel`): when to ask completion providers, live filtering
//! while typing, selection memories, resolving items, keyboard navigation
//! and accepting items in insert or replace mode.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, Task};
use text::PointUtf16;

use crate::cursor::{EditOperation, EditOperationType, Selection, StandardTokenType};
use crate::editor::CodeEditor;
use crate::options::{AcceptSuggestionOnEnter, SuggestInsertMode, WordBasedSuggestions};
use crate::providers::{
    CompletionItem, CompletionItemKind, CompletionItemTag, CompletionProvider, CompletionTrigger,
    completion_providers,
};
use crate::suggest::memory::{Candidate, MemoryContext, SuggestMemories};
use crate::suggest::{CompletionModel, CompletionModelOptions, LineContext};
use crate::suggest_widget::{
    LOADING_MESSAGE, SuggestUi, SuggestWidget, SuggestWidgetMetrics, SuggestWidgetState,
};
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

/// Monaco stops collecting document words past this many.
const MAX_WORDS: usize = 10_000;

/// Selection memories, shared by all editors (Monaco keeps them per
/// profile or workspace).
#[derive(Default)]
struct Memories(SuggestMemories);

impl gpui::Global for Memories {}

/// Where a session's items insert: what `getOverwriteInfo` needs.
#[derive(Clone, Copy)]
struct InsertContext {
    /// Where the request was made.
    position: PointUtf16,
    /// Start column of the word being completed.
    word_start: u32,
    /// End column of the word at `position` (the default replace range).
    word_end: u32,
}

/// Monaco `SuggestAlternatives`: after Tab or Enter accepted an item with
/// `tab_completion` on, Tab and Shift+Tab replace it with the next or
/// previous one.
struct Alternatives {
    items: Vec<CompletionItem>,
    index: usize,
    context: InsertContext,
    replace: bool,
}

impl Alternatives {
    /// Monaco `_moveIndex`: the next item without additional edits.
    fn move_index(&self, forward: bool) -> usize {
        let len = self.items.len();
        let mut index = self.index;
        for _ in 0..len {
            index = if forward {
                (index + 1) % len
            } else {
                (index + len - 1) % len
            };
            if index == self.index || self.items[index].additional_text_edits.is_empty() {
                break;
            }
        }
        index
    }
}

pub(crate) struct Session {
    /// Where the request was made.
    position: PointUtf16,
    /// Start column of the word being completed.
    word_start: u32,
    /// End column of the word at `position` (the default replace range).
    word_end: u32,
    /// The line's leading whitespace at the request.
    leading_whitespace: String,
    /// Monaco's frozen state: an explicit session whose matches no longer
    /// match what was typed.
    pub(crate) frozen: bool,
    pub(crate) model: CompletionModel,
    pub(crate) selected: usize,
    incomplete: bool,
    /// Opened by itself (typing, a trigger character), not Ctrl+Space
    /// (Monaco's `State.Auto`).
    auto: bool,
    /// The provider of each item; `None` for word-based suggestions.
    item_providers: Vec<Option<usize>>,
    providers: Vec<Arc<dyn CompletionProvider>>,
    /// Items resolved (or being resolved).
    resolved: Vec<bool>,
    resolve_task: Option<Task<()>>,
}

#[derive(Default)]
pub(crate) struct SuggestState {
    pub(crate) session: Option<Session>,
    task: Option<Task<()>>,
    /// The quick suggestions delay.
    quick_task: Option<Task<()>>,
    /// Additional edits resolved after inserting.
    late_edits: Option<Task<()>>,
    widget: Option<(Entity<SuggestWidget>, WidgetId)>,
    /// Tab completion's pending request: insert the best item instead of
    /// showing the widget, or type a tab if the cursor is still here.
    accept_best: Option<PointUtf16>,
    alternatives: Option<Alternatives>,
    /// Set while replacing an alternative, so its cursor change keeps them.
    keep_alternatives: bool,
    /// Shows "Loading..." if an explicit request takes a while.
    loading_task: Option<Task<()>>,
}

impl Session {
    fn insert_context(&self) -> InsertContext {
        InsertContext {
            position: self.position,
            word_start: self.word_start,
            word_end: self.word_end,
        }
    }
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

impl CodeEditor {
    /// Whether the suggest widget is showing.
    pub fn is_suggest_visible(&self) -> bool {
        self.suggest.widget.is_some()
    }

    /// Labels in the open suggest widget, in order.
    #[doc(hidden)]
    pub fn suggest_labels_for_tests(&self) -> Option<Vec<String>> {
        self.suggest.widget.as_ref()?;
        let session = self.suggest.session.as_ref()?;
        Some(
            session
                .model
                .filtered()
                .iter()
                .map(|s| session.model.items()[s.index].label.to_string())
                .collect(),
        )
    }

    /// The suggest widget's message ("Loading..." or "No suggestions."), if
    /// it shows one instead of a list.
    #[doc(hidden)]
    pub fn suggest_message_for_tests(&self, cx: &gpui::App) -> Option<&'static str> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        widget.read(cx).message()
    }

    /// Whether the suggest widget shows the selected item's details panel.
    pub fn is_suggestion_details_visible(&self, cx: &gpui::App) -> bool {
        self.suggest
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).details_shown(cx))
    }

    /// What accepting the selected suggestion in insert mode would replace
    /// and insert (Monaco's `SuggestWidgetAdaptor` selected item), for the
    /// suggest preview.
    pub(crate) fn selected_suggestion_edit(&self) -> Option<(std::ops::Range<PointUtf16>, String)> {
        let session = self.suggest.session.as_ref()?;
        let scored = session.model.filtered().get(session.selected)?;
        let item = &session.model.items()[scored.index];
        let head = self.cursors.primary().position;
        let start = item
            .range
            .as_ref()
            .filter(|r| r.start.row == head.row)
            .map_or(session.word_start, |r| r.start.column);
        let text = if item.insert_text_is_snippet {
            crate::snippet::parser::SnippetParser::new()
                .parse(item.insert_text(), false, false)
                .to_string()
        } else {
            item.insert_text().to_string()
        };
        Some((PointUtf16::new(head.row, start)..head, text))
    }

    /// The selected suggestion's label and its index in the list.
    pub fn selected_suggestion(&self) -> Option<(usize, CompletionItem)> {
        let session = self.suggest.session.as_ref()?;
        let scored = session.model.filtered().get(session.selected)?;
        Some((
            session.selected,
            session.model.items()[scored.index].clone(),
        ))
    }

    /// Start column of the word ending at the cursor (Monaco
    /// `getWordUntilPosition`).
    pub(crate) fn word_start_at(&self, position: PointUtf16) -> u32 {
        let line = self.snapshot.line(position.row);
        let mut column = 0u32;
        let mut start = 0u32;
        for ch in line.chars() {
            if column >= position.column {
                break;
            }
            column += ch.len_utf16() as u32;
            if !is_word_char(ch) {
                start = column;
            }
        }
        start
    }

    /// End column of the word continuing after `position`.
    fn word_end_at(&self, position: PointUtf16) -> u32 {
        let line = self.snapshot.line(position.row);
        let mut column = 0u32;
        let mut end = position.column;
        for ch in line.chars() {
            if column >= position.column {
                if !is_word_char(ch) {
                    break;
                }
                end = column + ch.len_utf16() as u32;
            }
            column += ch.len_utf16() as u32;
        }
        end
    }

    fn line_context_at(&self, position: PointUtf16, session_column: u32) -> LineContext {
        let line = self.snapshot.line(position.row);
        let mut leading = String::new();
        let mut column = 0u32;
        for ch in line.chars() {
            if column >= position.column {
                break;
            }
            leading.push(ch);
            column += ch.len_utf16() as u32;
        }
        LineContext {
            leading_line_content: leading,
            character_count_delta: position.column.saturating_sub(session_column) as usize,
        }
    }

    /// Closes the suggest widget (Escape, cursor moved away).
    pub fn hide_suggest(&mut self, cx: &mut Context<Self>) {
        self.suggest.accept_best = None;
        self.suggest.loading_task = None;
        // Monaco: a list resized below about four rows is restored to that.
        if let Some(ui) = cx.try_global::<SuggestUi>()
            && let Some(stored) = ui.size
        {
            let min = (self.suggest_item_height() * 4.3).ceil();
            if stored.height < min {
                cx.global_mut::<SuggestUi>().size = Some(gpui::size(stored.width, min));
            }
        }
        self.suggest.task = None;
        self.suggest.quick_task = None;
        self.suggest.session = None;
        if let Some((_, id)) = self.suggest.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Asks the completion providers for suggestions at the cursor
    /// (Ctrl+Space or a trigger).
    pub fn trigger_suggest(&mut self, trigger: CompletionTrigger, cx: &mut Context<Self>) {
        let auto = !matches!(trigger, CompletionTrigger::Invoke);
        self.request_suggestions(trigger, auto, cx);
    }

    fn request_suggestions(
        &mut self,
        trigger: CompletionTrigger,
        auto: bool,
        cx: &mut Context<Self>,
    ) {
        if self.options.read_only {
            return;
        }
        let position = self.cursors.primary().position;
        let model = self.model().read(cx);
        let context = model.provider_context();
        let language = model.language_id().map(|l| l.to_string());
        let providers = completion_providers(cx, language.as_deref());
        let tasks: Vec<_> = providers
            .iter()
            .map(|provider| provider.provide_completions(&context, position, trigger.clone(), cx))
            .collect();
        let word_start = self.word_start_at(position);
        let word_end = self.word_end_at(position);
        // Monaco `offWithInlineSuggestions`: off while an inline
        // completions provider serves this language and they're enabled.
        let words_enabled = match self.options.word_based_suggestions {
            WordBasedSuggestions::Off => false,
            WordBasedSuggestions::OffWithInlineSuggestions => {
                !self.options.inline_suggest.enabled
                    || crate::providers::inline_completions_providers(cx, language.as_deref())
                        .is_empty()
            }
            _ => true,
        };
        let words = words_enabled.then(|| {
            let (documents, regex) = self.word_sources(cx);
            let current: String = self
                .snapshot
                .line(position.row)
                .encode_utf16()
                .skip(word_start as usize)
                .take(word_end.saturating_sub(word_start) as usize)
                .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
                .collect();
            (documents, regex, current)
        });
        if tasks.is_empty() && words.is_none() {
            return;
        }
        // Monaco: explicit requests show "Loading..." after 50ms.
        if !auto && self.suggest.accept_best.is_none() && self.suggest.session.is_none() {
            self.suggest.loading_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                this.update(cx, |this, cx| {
                    this.suggest.loading_task = None;
                    if this.suggest.task.is_some() && this.suggest.session.is_none() {
                        this.show_suggest_widget(
                            SuggestWidgetState::message(LOADING_MESSAGE),
                            PointUtf16::new(position.row, word_start),
                            cx,
                        );
                    }
                })
                .ok();
            }));
        }
        self.suggest.task = Some(cx.spawn(async move |this, cx| {
            let mut items = Vec::new();
            let mut item_providers = Vec::new();
            let mut incomplete = false;
            for (ix, task) in tasks.into_iter().enumerate() {
                match task.await {
                    Ok(result) => {
                        incomplete |= result.incomplete;
                        item_providers.extend(std::iter::repeat_n(Some(ix), result.items.len()));
                        items.extend(result.items);
                    }
                    Err(err) => log::warn!("completions failed: {err:#}"),
                }
            }
            let Ok((mut items, mut item_providers)) = this.update(cx, |this, _| {
                this.filter_suggestion_kinds(items, item_providers)
            }) else {
                return;
            };
            // Monaco: word-based suggestions are a lower-ranked provider
            // group, used only when the others have nothing; collected off
            // the main thread.
            if items.is_empty()
                && let Some((documents, regex, current)) = words
            {
                let words = cx
                    .background_executor()
                    .spawn(async move {
                        collect_words(
                            &documents,
                            &regex,
                            &current,
                            position,
                            (word_start, word_end),
                        )
                    })
                    .await;
                let item_count = words.len();
                let Ok(filtered) = this.update(cx, |this, _| {
                    this.filter_suggestion_kinds(words, vec![None; item_count])
                }) else {
                    return;
                };
                (items, item_providers) = filtered;
            }
            this.update(cx, |this, cx| {
                this.start_session(
                    items,
                    item_providers,
                    providers,
                    incomplete,
                    (position, word_start, word_end),
                    auto,
                    cx,
                )
            })
            .ok();
        }));
    }

    /// Drops hidden kinds, deprecated items (`suggest.show_deprecated`) and
    /// snippets (`snippet_suggestions: None`), as Monaco does when
    /// collecting.
    fn filter_suggestion_kinds(
        &self,
        items: Vec<CompletionItem>,
        item_providers: Vec<Option<usize>>,
    ) -> (Vec<CompletionItem>, Vec<Option<usize>>) {
        let suggest = &self.options.suggest;
        items
            .into_iter()
            .zip(item_providers)
            .filter(|(item, _)| {
                !suggest.hidden_kinds.contains(&item.kind)
                    && (suggest.show_deprecated
                        || !item.tags.contains(&CompletionItemTag::Deprecated))
                    && (self.options.snippet_suggestions.is_some()
                        || item.kind != CompletionItemKind::Snippet)
            })
            .unzip()
    }

    /// The texts and word pattern for word-based suggestions: the documents
    /// `word_based_suggestions` names.
    fn word_sources(&self, cx: &gpui::App) -> (Vec<text::Rope>, syntax::fancy_regex::Regex) {
        let model = self.model().read(cx);
        let language = model.language_id().cloned();
        let regex = model
            .language()
            .and_then(|language| language.config.word_pattern.clone());
        let regex = match regex.as_ref().and_then(syntax::Pattern::regex) {
            Some(regex) => regex.clone(),
            None => crate::cursor::in_place_replace::default_word_regex().clone(),
        };
        let mut documents = vec![model.buffer().rope().clone()];
        let others = crate::model::registered_models(cx)
            .into_iter()
            .filter(|m| m != self.model());
        match self.options.word_based_suggestions {
            WordBasedSuggestions::Off | WordBasedSuggestions::CurrentDocument => {}
            WordBasedSuggestions::OffWithInlineSuggestions
            | WordBasedSuggestions::MatchingDocuments => documents.extend(
                others
                    .filter(|m| m.read(cx).language_id() == language.as_ref())
                    .map(|m| m.read(cx).buffer().rope().clone()),
            ),
            WordBasedSuggestions::AllDocuments => {
                documents.extend(others.map(|m| m.read(cx).buffer().rope().clone()))
            }
        }
        (documents, regex)
    }

    #[allow(clippy::too_many_arguments)]
    fn start_session(
        &mut self,
        items: Vec<CompletionItem>,
        item_providers: Vec<Option<usize>>,
        providers: Vec<Arc<dyn CompletionProvider>>,
        incomplete: bool,
        (position, word_start, word_end): (PointUtf16, u32, u32),
        auto: bool,
        cx: &mut Context<Self>,
    ) {
        let head = self.cursors.primary().position;
        self.suggest.loading_task = None;
        // Monaco shows "No suggestions." for explicit requests.
        let show_empty = !auto && self.suggest.accept_best.is_none();
        if (items.is_empty() && !show_empty) || head.row != position.row || head.column < word_start
        {
            self.finish_accept_best(cx);
            self.hide_suggest(cx);
            return;
        }
        let distances = if self.options.suggest.locality_bonus {
            self.word_distances(position, &items, cx)
        } else {
            Vec::new()
        };
        let line_context = self.line_context_at(head, position.column);
        let options = CompletionModelOptions {
            filter_graceful: self.options.suggest.filter_graceful,
            snippet_sort_order: self.options.snippet_suggestions.unwrap_or_default(),
            fuzzy_score_options: crate::suggest::FuzzyScoreOptions {
                first_match_can_be_weak: !self.options.suggest.match_on_word_start_only,
                ..crate::suggest::FuzzyScoreOptions::MONACO_DEFAULT
            },
        };
        let resolved = vec![false; items.len()];
        let mut model =
            CompletionModel::new_with_options(items, position, word_start, line_context, options);
        // Monaco: distances only count while the cursor is where the request
        // was made.
        if head == position {
            model.set_distances(distances);
        }
        let leading_whitespace = leading_whitespace(&self.snapshot.line(position.row)).to_string();
        self.suggest.session = Some(Session {
            position,
            word_start,
            word_end,
            leading_whitespace,
            frozen: false,
            model,
            selected: 0,
            incomplete,
            auto,
            item_providers,
            providers,
            resolved,
            resolve_task: None,
        });
        self.select_by_memory(cx);
        if self.suggest.accept_best.is_some() {
            self.finish_accept_best(cx);
            return;
        }
        self.refresh_suggest_widget(cx);
        self.resolve_selected_suggestion(cx);
    }

    /// Shows `items` replacing `range` (Monaco `showSimpleSuggestions`),
    /// e.g. a snippet choice's options.
    pub(crate) fn show_simple_suggestions(
        &mut self,
        items: Vec<CompletionItem>,
        range: std::ops::Range<PointUtf16>,
        cx: &mut Context<Self>,
    ) {
        self.hide_suggest(cx);
        let head = self.cursors.primary().position;
        let count = items.len();
        self.start_session(
            items,
            vec![None; count],
            Vec::new(),
            false,
            (head, range.start.column, range.end.column),
            false,
            cx,
        );
    }

    /// Tab completion (Monaco `insertBestCompletion`): at the end of a word,
    /// asks for completions without showing them and inserts the one
    /// `suggest_selection` picks, or types a tab if none changes the text.
    /// Returns false if it didn't apply (no word ends at the cursor).
    pub(crate) fn insert_best_completion(&mut self, cx: &mut Context<Self>) -> bool {
        let head = self.cursors.primary().position;
        if self.options.read_only
            || self.is_suggest_visible()
            || self.word_start_at(head) == head.column
            || self.word_end_at(head) != head.column
        {
            return false;
        }
        self.hide_suggest(cx);
        self.suggest.accept_best = Some(head);
        self.request_suggestions(CompletionTrigger::Invoke, false, cx);
        if self.suggest.task.is_none() {
            // Nothing to ask.
            self.finish_accept_best(cx);
        }
        true
    }

    /// Completes a pending [`Self::insert_best_completion`] with the session
    /// (if any): inserts its selected item, or falls back to a tab.
    fn finish_accept_best(&mut self, cx: &mut Context<Self>) {
        let Some(requested_at) = self.suggest.accept_best.take() else {
            return;
        };
        let makes_edit = self.suggest.session.as_ref().is_some_and(|session| {
            session
                .model
                .filtered()
                .get(session.selected)
                .is_some_and(|scored| {
                    self.makes_text_edit(&session.model.items()[scored.index], session.word_start)
                })
        });
        if makes_edit {
            self.accept_suggestion(cx);
            return;
        }
        self.hide_suggest(cx);
        if self.cursors.primary().position == requested_at {
            self.insert_tab(cx);
        }
    }

    /// Monaco `triggerSuggestAndAcceptBest`'s `makesTextEdit`: snippets and
    /// items with additional edits always do; others unless the text before
    /// the cursor already is their insert text.
    fn makes_text_edit(&self, item: &CompletionItem, word_start: u32) -> bool {
        if item.insert_text_is_snippet || !item.additional_text_edits.is_empty() {
            return true;
        }
        let head = self.cursors.primary().position;
        let start = item.range.as_ref().map_or(word_start, |r| r.start.column);
        let typed: String = self
            .snapshot
            .line(head.row)
            .encode_utf16()
            .skip(start as usize)
            .take(head.column.saturating_sub(start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        typed != item.insert_text()
    }

    /// Whether Tab or Shift+Tab can replace the last inserted suggestion.
    pub(crate) fn has_other_suggestions(&self) -> bool {
        self.suggest.alternatives.is_some()
    }

    /// Replaces the last suggestion Tab or Enter inserted with the next
    /// (`forward`) or previous one (Monaco `insertNextSuggestion`,
    /// `insertPrevSuggestion`).
    pub(crate) fn insert_other_suggestion(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(alternatives) = self.suggest.alternatives.as_mut() else {
            return;
        };
        alternatives.index = alternatives.move_index(forward);
        let item = alternatives.items[alternatives.index].clone();
        let (context, replace) = (alternatives.context, alternatives.replace);
        self.suggest.late_edits = None;
        self.suggest.keep_alternatives = true;
        // Monaco undoes the previous insertion and inserts the next item.
        self.undo(cx);
        self.insert_completion(&item, context, replace, cx);
        self.suggest.keep_alternatives = false;
    }

    /// Drops the alternatives when the cursor moves (Monaco resets them on
    /// any cursor change it didn't cause).
    pub(crate) fn suggest_cursor_changed(&mut self) {
        if !self.suggest.keep_alternatives {
            self.suggest.alternatives = None;
        }
    }

    /// Monaco `WordDistance`: with `suggest.locality_bonus`, words used in
    /// the innermost bracket block around `position` sort before others.
    fn word_distances(
        &self,
        position: PointUtf16,
        items: &[CompletionItem],
        cx: &gpui::App,
    ) -> Vec<u32> {
        use crate::cursor::smart_select::bracket_selection_ranges;
        const FAR: u32 = 2 << 20;
        let ranges = self
            .with_bracket_context(cx, |rope, config, skip| {
                bracket_selection_ranges(rope, config, position, skip)
            })
            .unwrap_or_else(|| {
                bracket_selection_ranges(
                    self.snapshot.rope(),
                    &syntax::LanguageConfiguration::default(),
                    position,
                    &|_| false,
                )
            });
        let Some(block) = ranges.first() else {
            return Vec::new();
        };
        // Monaco `computeWordRanges`: the words of the block's lines (its
        // last line excluded, as in Monaco), except numbers.
        let (_, regex) = self.word_sources(cx);
        let mut words: std::collections::HashMap<String, Vec<std::ops::Range<PointUtf16>>> =
            Default::default();
        for row in block.start.row..block.end.row {
            let line = self.snapshot.line(row);
            let mut start = 0;
            while start < line.len() {
                let Ok(Some(found)) = regex.find_from_pos(&line, start) else {
                    break;
                };
                start = if found.end() > found.start() {
                    found.end()
                } else {
                    found.end() + line[found.end()..].chars().next().map_or(1, char::len_utf8)
                };
                let word = found.as_str();
                if word.is_empty() || word.parse::<f64>().is_ok() {
                    continue;
                }
                let column = |byte: usize| line[..byte].encode_utf16().count() as u32;
                words.entry(word.to_string()).or_default().push(
                    PointUtf16::new(row, column(found.start()))
                        ..PointUtf16::new(row, column(found.end())),
                );
            }
        }
        // The word being typed doesn't count.
        let word_start = self.word_start_at(position);
        let current: String = self
            .snapshot
            .line(position.row)
            .encode_utf16()
            .skip(word_start as usize)
            .take((position.column - word_start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        words.remove(&current);
        items
            .iter()
            .map(|item| {
                if item.kind == CompletionItemKind::Keyword {
                    return FAR;
                }
                let Some(lines) = words.get(item.label.as_ref()) else {
                    return FAR;
                };
                // The occurrence at or last before the request position.
                let anchor = position..position;
                let best = match lines
                    .binary_search_by(|r| r.start.cmp(&anchor.start).then(r.end.cmp(&anchor.end)))
                {
                    Ok(ix) => &lines[ix],
                    Err(ix) => &lines[ix.saturating_sub(1)],
                };
                let mut distance = ranges.len() as u32;
                for range in &ranges {
                    if !(range.start <= best.start && best.end <= range.end) {
                        break;
                    }
                    distance -= 1;
                }
                distance
            })
            .collect()
    }

    /// The memories' view of the cursor.
    fn memory_context(&self, cx: &gpui::App) -> (String, String, u32, String) {
        let head = self.cursors.primary().position;
        let line = self.snapshot.line(head.row);
        let language = self
            .model()
            .read(cx)
            .language_id()
            .map_or_else(|| "plaintext".to_string(), |l| l.to_string());
        let word_until: String = line
            .encode_utf16()
            .skip(self.word_start_at(head) as usize)
            .take(head.column.saturating_sub(self.word_start_at(head)) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        (language, line, head.column, word_until)
    }

    /// Selects the item `suggest_selection` picks (Monaco's memory
    /// `select`).
    fn select_by_memory(&mut self, cx: &mut Context<Self>) {
        let (language, line, column, word_until) = self.memory_context(cx);
        let mode = self.options.suggest_selection;
        let memories = cx.default_global::<Memories>();
        let Some(session) = self.suggest.session.as_mut() else {
            return;
        };
        let items = session.model.items();
        let candidates: Vec<Candidate> = session
            .model
            .filtered()
            .iter()
            .map(|scored| {
                let item = &items[scored.index];
                Candidate {
                    label: &item.label,
                    kind: item.kind,
                    insert_text: item.insert_text(),
                    preselect: item.preselect,
                    score: scored.score.score,
                }
            })
            .collect();
        let context = MemoryContext {
            language: &language,
            line: &line,
            column,
            word_until: &word_until,
        };
        session.selected = memories.0.select(mode, &context, &candidates);
    }

    fn refresh_suggest_widget(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.suggest.session else {
            return;
        };
        if session.model.filtered().is_empty() && session.auto {
            self.hide_suggest(cx);
            return;
        }
        let anchor = PointUtf16::new(session.position.row, session.word_start);
        let state = SuggestWidgetState::from_session(session);
        self.show_suggest_widget(state, anchor, cx);
    }

    /// The suggest widget's row height (`suggest.line_height`, or the
    /// editor's, clamped to 8–1000 as in Monaco).
    pub(crate) fn suggest_item_height(&self) -> gpui::Pixels {
        let line_height = self.options.suggest.line_height;
        let height = if line_height > 0. {
            gpui::px(line_height)
        } else {
            self.options.resolved_line_height()
        };
        height.clamp(gpui::px(8.), gpui::px(1000.))
    }

    fn suggest_metrics(&self, anchor: PointUtf16) -> SuggestWidgetMetrics {
        let font_size = if self.options.suggest.font_size > 0. {
            gpui::px(self.options.suggest.font_size)
        } else {
            self.options.font_size
        };
        let available_width = self.layout.as_ref().and_then(|layout| {
            let origin = self.scrolled_visible_position(anchor)?;
            Some(layout.bounds.right() - origin.x)
        });
        SuggestWidgetMetrics {
            item_height: self.suggest_item_height(),
            font_size,
            available_width,
        }
    }

    fn show_suggest_widget(
        &mut self,
        state: SuggestWidgetState,
        anchor: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        let metrics = self.suggest_metrics(anchor);
        match &self.suggest.widget {
            Some((widget, id)) => {
                let id = *id;
                widget.update(cx, |widget, cx| widget.set_state(state, metrics, cx));
                self.set_content_widget_position(id, anchor, cx);
            }
            None => {
                let editor = cx.entity().downgrade();
                let theme = self.theme.clone();
                let style = self.markdown_style();
                let options = self.options.suggest.clone();
                let widget = cx.new(|cx| {
                    SuggestWidget::new(editor, state, theme, style, options, metrics, cx)
                });
                let id = self.add_content_widget(
                    ContentWidget {
                        position: anchor,
                        preference: vec![
                            ContentWidgetPreference::Below,
                            ContentWidgetPreference::Above,
                        ],
                        view: widget.clone().into(),
                    },
                    cx,
                );
                self.suggest.widget = Some((widget, id));
            }
        }
    }

    /// Shows or hides the details panel (Monaco `toggleSuggestionDetails`,
    /// Ctrl+Space while suggestions show); remembered for later sessions.
    pub fn toggle_suggestion_details(&mut self, cx: &mut Context<Self>) {
        if self.suggest.session.is_none() {
            return;
        }
        let ui = cx.default_global::<SuggestUi>();
        ui.details_expanded = !ui.details_expanded;
        self.refresh_suggest_widget(cx);
    }

    /// Rows the list pages by (Page Up / Page Down).
    pub(crate) fn suggest_page_size(&self, cx: &gpui::App) -> usize {
        self.suggest.widget.as_ref().map_or(11, |(widget, _)| {
            widget.read(cx).visible_rows(cx).saturating_sub(1).max(1)
        })
    }

    /// Resolves the selected item if its provider can (Monaco resolves on
    /// focus): its docs, detail and edits replace the original's.
    fn resolve_selected_suggestion(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.suggest.session.as_mut() else {
            return;
        };
        let Some(index) = session
            .model
            .filtered()
            .get(session.selected)
            .map(|s| s.index)
        else {
            return;
        };
        if session.resolved[index] {
            return;
        }
        let Some(provider) = session.item_providers[index].map(|p| session.providers[p].clone())
        else {
            session.resolved[index] = true;
            return;
        };
        let item = session.model.items()[index].clone();
        let Some(task) = provider.resolve_completion_item(&item, cx) else {
            if let Some(session) = self.suggest.session.as_mut() {
                session.resolved[index] = true;
            }
            return;
        };
        let session = self.suggest.session.as_mut().expect("checked above");
        session.resolve_task = Some(cx.spawn(async move |this, cx| {
            let resolved = task.await;
            this.update(cx, |this, cx| {
                let Some(session) = this.suggest.session.as_mut() else {
                    return;
                };
                match resolved {
                    Ok(item) => {
                        session.model.replace_item(index, item);
                        session.resolved[index] = true;
                    }
                    Err(err) => log::warn!("resolving a completion failed: {err:#}"),
                }
                session.resolve_task = None;
                this.refresh_suggest_widget(cx);
            })
            .ok();
        }));
    }

    /// Called after `text` was typed (before signature help).
    pub(crate) fn suggest_after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(ch) = text.chars().last() else {
            return;
        };
        if self.options.read_only {
            return;
        }
        // Typing triggers inline completions too.
        self.trigger_inline_completions(cx);
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let providers = completion_providers(cx, language.as_deref());
        // Monaco `suppressSuggestions`: ghost text keeps the widget closed.
        if self.inline_suppresses_suggestions(cx) && self.suggest.session.is_none() {
            return;
        }
        if self.options.suggest_on_trigger_characters
            && providers
                .iter()
                .any(|p| p.trigger_characters().contains(&ch))
        {
            self.request_suggestions(CompletionTrigger::TriggerCharacter(ch), true, cx);
            return;
        }
        if self.suggest.session.is_some() {
            self.refilter_suggest(cx);
            return;
        }
        if self.suggest.task.is_some() {
            // Still computing: the results are filtered by what's typed
            // then, unless the word ended.
            if !is_word_char(ch) {
                self.hide_suggest(cx);
            }
            return;
        }
        if !is_word_char(ch) {
            return;
        }
        // Monaco quick suggestions: after `quick_suggestions_delay`, if the
        // cursor ends a word (or its first character) and the token type
        // allows it.
        let delay = self.options.quick_suggestions_delay;
        if delay.is_zero() {
            self.quick_suggest(cx);
        } else {
            self.suggest.quick_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                this.update(cx, |this, cx| {
                    this.suggest.quick_task = None;
                    this.quick_suggest(cx);
                })
                .ok();
            }));
        }
    }

    /// Monaco `LineContext.shouldAutoTrigger`: a word ends at the cursor (or
    /// the cursor is one character into it) and isn't a number.
    fn should_auto_trigger(&self, _cx: &gpui::App) -> bool {
        let head = self.cursors.primary().position;
        let start = self.word_start_at(head);
        let end = self.word_end_at(head);
        if start == head.column || (end != head.column && start + 1 != head.column) {
            return false;
        }
        let word: String = self
            .snapshot
            .line(head.row)
            .encode_utf16()
            .skip(start as usize)
            .take((end - start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        word.parse::<f64>().is_err()
    }

    /// Monaco `SuggestModel._doTriggerQuickSuggest`.
    fn quick_suggest(&mut self, cx: &mut Context<Self>) {
        if self.suggest.session.is_some() || self.suggest.task.is_some() {
            return;
        }
        let quick = self.options.quick_suggestions;
        if quick == crate::options::QuickSuggestions::OFF {
            return;
        }
        if !self.should_auto_trigger(cx) {
            return;
        }
        let head = self.cursors.primary().position;
        let offset = self.snapshot.point_utf16_to_offset(head).saturating_sub(1);
        let allowed = match self.model().read(cx).token_type_at(offset) {
            StandardTokenType::Comment => quick.comments,
            StandardTokenType::String => quick.strings,
            _ => quick.other,
        };
        if allowed {
            self.request_suggestions(CompletionTrigger::Invoke, true, cx);
        }
    }

    /// Monaco `SuggestModel._onNewContext`, after typing or deleting:
    /// closes, re-requests, or re-filters the open session.
    pub(crate) fn refilter_suggest(&mut self, cx: &mut Context<Self>) {
        let head = self.cursors.primary().position;
        let Some(session) = &self.suggest.session else {
            return;
        };
        let line = self.snapshot.line(head.row);
        if head.row != session.position.row
            || leading_whitespace(&line) != session.leading_whitespace
        {
            // Another line (Enter), or the word was outdented.
            self.hide_suggest(cx);
            return;
        }
        let auto = session.auto;
        let word_start = self.word_start_at(head);
        let on_word = word_start < head.column;
        if head.column < session.position.column {
            // Moved left of the request: ask again while on a word.
            if on_word {
                self.request_suggestions(CompletionTrigger::Invoke, auto, cx);
            } else {
                self.hide_suggest(cx);
            }
            return;
        }
        if on_word && word_start > session.word_start {
            // Started a new word: ask again if it would have opened by
            // itself.
            if self.should_auto_trigger(cx) {
                self.request_suggestions(CompletionTrigger::Invoke, auto, cx);
            }
            return;
        }
        if head.column > session.position.column && session.incomplete && on_word {
            self.request_suggestions(CompletionTrigger::Incomplete, auto, cx);
            return;
        }
        let request_column = session.position.column;
        let line_context = self.line_context_at(head, request_column);
        let Some(session) = self.suggest.session.as_mut() else {
            return;
        };
        let old_context = session.model.line_context().clone();
        session.model.set_line_context(line_context);
        if head != session.position {
            session.model.set_distances(Vec::new());
        }
        if session.model.filtered().is_empty() {
            if self.should_auto_trigger(cx) && request_column < word_start {
                // Heading into a new word.
                self.request_suggestions(CompletionTrigger::Invoke, auto, cx);
                return;
            }
            if auto {
                self.hide_suggest(cx);
                return;
            }
            // Requested explicitly: keep showing the last matches (frozen),
            // unless the cursor left the word.
            let Some(session) = self.suggest.session.as_mut() else {
                return;
            };
            session.model.set_line_context(old_context);
            let frozen = !session.model.filtered().is_empty();
            if frozen && !on_word {
                self.hide_suggest(cx);
                return;
            }
            session.frozen = frozen;
        } else {
            session.frozen = false;
        }
        self.select_by_memory(cx);
        self.refresh_suggest_widget(cx);
        self.resolve_selected_suggestion(cx);
    }

    /// Moves the selection by `delta` rows (Up/Down/Page). Returns whether
    /// the widget handled it.
    pub(crate) fn suggest_navigate(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.suggest.session.as_mut() else {
            return false;
        };
        let count = session.model.filtered().len();
        if count == 0 || self.suggest.widget.is_none() {
            return false;
        }
        let selected = session.selected as isize + delta;
        session.selected = if delta.abs() == 1 {
            selected.rem_euclid(count as isize) as usize
        } else {
            selected.clamp(0, count as isize - 1) as usize
        };
        self.refresh_suggest_widget(cx);
        self.resolve_selected_suggestion(cx);
        true
    }

    pub(crate) fn select_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(session) = self.suggest.session.as_mut() {
            session.selected = index;
        }
        self.refresh_suggest_widget(cx);
        self.resolve_selected_suggestion(cx);
    }

    /// Selects the first suggestion (Monaco `selectFirstSuggestion`).
    pub fn select_first_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_suggest_visible() {
            return false;
        }
        self.select_suggestion(0, cx);
        true
    }

    /// Selects the last suggestion (Monaco `selectLastSuggestion`).
    pub fn select_last_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(count) = self
            .suggest
            .session
            .as_ref()
            .map(|s| s.model.filtered().len())
        else {
            return false;
        };
        if !self.is_suggest_visible() || count == 0 {
            return false;
        }
        self.select_suggestion(count - 1, cx);
        true
    }

    /// Accepts the selected suggestion (Tab, or Enter). Returns whether one
    /// was accepted.
    pub fn accept_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        self.accept_suggestion_with(false, cx)
    }

    /// Accepts the selected suggestion in the other insert mode (Monaco
    /// `acceptAlternativeSelectedSuggestion`, Shift+Enter or Shift+Tab).
    pub fn accept_alternative_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        self.accept_suggestion_with(true, cx)
    }

    /// Enter with the widget open: accepts unless
    /// `accept_suggestion_on_enter` says not to (then Enter types a new
    /// line).
    pub(crate) fn accept_suggestion_on_enter(&mut self, cx: &mut Context<Self>) -> bool {
        match self.options.accept_suggestion_on_enter {
            AcceptSuggestionOnEnter::Off => false,
            AcceptSuggestionOnEnter::On => self.accept_suggestion(cx),
            AcceptSuggestionOnEnter::Smart => {
                if self.suggestion_makes_text_edit() {
                    self.accept_suggestion(cx)
                } else {
                    self.hide_suggest(cx);
                    false
                }
            }
        }
    }

    /// Monaco's `suggestionMakesTextEdit` for `smart` Enter: false in a
    /// session that opened by itself when the item has no other edits, isn't
    /// a snippet, and the text before the cursor already is its insert text.
    fn suggestion_makes_text_edit(&self) -> bool {
        let Some(session) = &self.suggest.session else {
            return false;
        };
        let Some(scored) = session.model.filtered().get(session.selected) else {
            return false;
        };
        if !session.auto {
            return true;
        }
        let item = &session.model.items()[scored.index];
        if !item.additional_text_edits.is_empty() || item.insert_text_is_snippet {
            return true;
        }
        let head = self.cursors.primary().position;
        let start = item
            .range
            .as_ref()
            .map_or(session.word_start, |r| r.start.column);
        let typed: String = self
            .snapshot
            .line(head.row)
            .encode_utf16()
            .skip(start as usize)
            .take(head.column.saturating_sub(start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        typed != item.insert_text()
    }

    fn accept_suggestion_with(&mut self, toggle: bool, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.suggest.session.take() else {
            return false;
        };
        let Some(scored) = session.model.filtered().get(session.selected) else {
            self.hide_suggest(cx);
            return false;
        };
        let item: CompletionItem = session.model.items()[scored.index].clone();
        let selected = session.selected;
        let provider = session.item_providers[scored.index].map(|p| session.providers[p].clone());
        let resolved = session.resolved[scored.index];
        // Monaco memorizes before inserting.
        let (language, line, column, word_until) = self.memory_context(cx);
        let mode = self.options.suggest_selection;
        cx.default_global::<Memories>().0.memorize(
            mode,
            &MemoryContext {
                language: &language,
                line: &line,
                column,
                word_until: &word_until,
            },
            &Candidate {
                label: &item.label,
                kind: item.kind,
                insert_text: item.insert_text(),
                preselect: item.preselect,
                score: scored.score.score,
            },
        );
        self.hide_suggest(cx);
        let replace = (self.options.suggest.insert_mode == SuggestInsertMode::Replace) ^ toggle;
        let version = self.model().read(cx).buffer().version();
        let context = session.insert_context();
        self.insert_completion(&item, context, replace, cx);
        // Monaco keeps the other items for Tab and Shift+Tab.
        if self.options.tab_completion == crate::options::TabCompletion::On {
            let items: Vec<CompletionItem> = session
                .model
                .filtered()
                .iter()
                .map(|scored| session.model.items()[scored.index].clone())
                .collect();
            let alternatives = Alternatives {
                items,
                index: selected,
                context,
                replace,
            };
            if alternatives.move_index(true) != selected {
                self.suggest.alternatives = Some(alternatives);
            }
        }
        // Additional edits an unresolved item may still have arrive later.
        if item.additional_text_edits.is_empty()
            && !resolved
            && let Some(task) = provider.and_then(|p| p.resolve_completion_item(&item, cx))
        {
            self.suggest.late_edits = Some(cx.spawn(async move |this, cx| {
                let Ok(resolved) = task.await else {
                    return;
                };
                this.update(cx, |this, cx| this.apply_late_edits(resolved, version, cx))
                    .ok();
            }));
        }
        // Monaco runs the item's command after inserting.
        if let Some(command) = &item.command {
            if command.id == crate::snippet_session::NEXT_PLACEHOLDER_COMMAND {
                self.next_snippet_placeholder(cx);
            } else if command.id == "editor.action.triggerSuggest" {
                self.request_suggestions(CompletionTrigger::Invoke, true, cx);
            } else if let Err(err) = crate::commands::execute_command(cx, command) {
                log::warn!("{err:#}");
            }
        }
        true
    }

    /// Applies additional edits resolved after inserting, unless one starts
    /// after the earliest change since `version` (from before inserting):
    /// Monaco drops them then, since only text before every change still
    /// has its old positions.
    fn apply_late_edits(
        &mut self,
        item: CompletionItem,
        version: text::Version,
        cx: &mut Context<Self>,
    ) {
        if item.additional_text_edits.is_empty() || self.options.read_only {
            return;
        }
        let model = self.model().clone();
        let buffer = model.read(cx).buffer();
        let patch = buffer.edits_since(version);
        // Where the earliest change since then ends, in the old text.
        let earliest = patch.edits().first().map(|edit| edit.old.end);
        let rope = buffer.rope();
        let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for (range, text) in item.additional_text_edits {
            let start =
                rope.point_utf16_to_offset(rope.clip_point_utf16(range.start, text::Bias::Left));
            let end =
                rope.point_utf16_to_offset(rope.clip_point_utf16(range.end, text::Bias::Left));
            if earliest.is_some_and(|earliest| start > earliest) {
                return;
            }
            edits.push((start..end, text));
        }
        edits.sort_by_key(|(range, _)| range.start);
        model.update(cx, |model, cx| {
            model.push_stack_element();
            model.edit(edits, cx).ok();
            model.push_stack_element();
        });
    }

    /// Monaco `getOverwriteInfo`: replaces `overwrite_before` characters
    /// before each cursor and `overwrite_after` after it.
    fn insert_completion(
        &mut self,
        item: &CompletionItem,
        session: InsertContext,
        replace: bool,
        cx: &mut Context<Self>,
    ) {
        let head = self.cursors.primary().position;
        let position = session.position;
        let start = item
            .range
            .as_ref()
            .filter(|r| r.start.row == head.row)
            .map_or(session.word_start, |r| r.start.column);
        let end = if replace {
            item.replace_range
                .as_ref()
                .or(item.range.as_ref())
                .map_or(session.word_end, |r| r.end.column)
        } else {
            item.range
                .as_ref()
                .map_or(position.column, |r| r.end.column)
        };
        let overwrite_before = head.column.saturating_sub(start);
        let overwrite_after = end.saturating_sub(position.column);
        // Monaco inserts every item as a snippet (plain text escaped), after
        // its additional edits, as one undo step.
        self.model().update(cx, |model, _| {
            model.push_stack_element();
            model.start_transaction();
        });
        if !item.additional_text_edits.is_empty() {
            self.apply_additional_text_edits(&item.additional_text_edits, cx);
        }
        let template = if item.insert_text_is_snippet {
            item.insert_text().to_string()
        } else {
            crate::snippet::parser::SnippetParser::escape(item.insert_text())
        };
        self.insert_snippet(
            &template,
            crate::snippet_session::SnippetInsertOptions {
                overwrite_before,
                overwrite_after,
                adjust_whitespace: !item.keep_whitespace,
                undo_stop_before: false,
                undo_stop_after: false,
                clipboard_text: None,
            },
            cx,
        );
        self.model().update(cx, |model, _| {
            model.end_transaction();
            model.push_stack_element();
        });
    }

    fn apply_additional_text_edits(
        &mut self,
        edits: &[(std::ops::Range<PointUtf16>, String)],
        cx: &mut Context<Self>,
    ) {
        self.apply_edits_moving_cursors(edits, EditOperationType::Other, cx);
    }

    /// Applies `edits` elsewhere in the text as one operation of `kind`,
    /// moving the cursors with them (text inserted at a cursor goes before
    /// it).
    pub(crate) fn apply_edits_moving_cursors(
        &mut self,
        edits: &[(std::ops::Range<PointUtf16>, String)],
        kind: EditOperationType,
        cx: &mut Context<Self>,
    ) {
        let mut offsets: Vec<(usize, usize, &str)> = edits
            .iter()
            .map(|(range, text)| {
                (
                    self.snapshot.point_utf16_to_offset(range.start),
                    self.snapshot.point_utf16_to_offset(range.end),
                    text.as_str(),
                )
            })
            .collect();
        offsets.sort_by_key(|(start, _, _)| *start);
        let transform = |offset: usize| -> usize {
            let mut delta: isize = 0;
            for &(start, end, text) in &offsets {
                if end <= offset {
                    delta += text.len() as isize - (end - start) as isize;
                } else if start < offset {
                    // Inside a replaced range: to the end of its new text.
                    return (start as isize + delta) as usize + text.len();
                }
            }
            (offset as isize + delta) as usize
        };
        let new_points: Vec<(usize, usize)> = self
            .cursors
            .selections()
            .iter()
            .map(|s| {
                (
                    transform(self.snapshot.point_utf16_to_offset(s.anchor)),
                    transform(self.snapshot.point_utf16_to_offset(s.head)),
                )
            })
            .collect();
        let mut sorted = edits.to_vec();
        sorted.sort_by_key(|(range, _)| range.start);
        self.apply(
            EditOperation {
                edits: sorted,
                selections_after: self.cursors.selections(),
                kind,
                push_stack_element_before: false,
                push_stack_element_after: false,
                auto_closed: Vec::new(),
                follow_up: None,
                auto_whitespace: Vec::new(),
            },
            cx,
        );
        let selections: Vec<Selection> = new_points
            .into_iter()
            .map(|(anchor, head)| {
                Selection::new(
                    self.snapshot
                        .offset_to_point_utf16(anchor.min(self.snapshot.len())),
                    self.snapshot
                        .offset_to_point_utf16(head.min(self.snapshot.len())),
                )
            })
            .collect();
        self.cursors.set_selections(&selections);
    }

    /// If `ch` is a commit character of the selected suggestion, accepts it
    /// first (`accept_suggestion_on_commit_character`). Returns whether it
    /// did.
    pub(crate) fn commit_suggestion_on(&mut self, ch: char, cx: &mut Context<Self>) -> bool {
        if !self.options.accept_suggestion_on_commit_character {
            return false;
        }
        let Some(session) = &self.suggest.session else {
            return false;
        };
        let Some(scored) = session.model.filtered().get(session.selected) else {
            return false;
        };
        if session.model.items()[scored.index]
            .commit_characters
            .contains(&ch)
        {
            self.accept_suggestion(cx)
        } else {
            false
        }
    }
}

/// Monaco's word-based suggestions (`WordDistance`-free
/// `wordCompletionProvider`): the words of `documents`, except `current`
/// (the word at the cursor) and numbers, deduplicated and capped.
fn collect_words(
    documents: &[text::Rope],
    regex: &syntax::fancy_regex::Regex,
    current: &str,
    position: PointUtf16,
    (word_start, word_end): (u32, u32),
) -> Vec<CompletionItem> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for document in documents {
        let text = document.to_string();
        let mut start = 0;
        while start < text.len() {
            let Ok(Some(found)) = regex.find_from_pos(&text, start) else {
                break;
            };
            start = if found.end() > found.start() {
                found.end()
            } else {
                found.end() + text[found.end()..].chars().next().map_or(1, char::len_utf8)
            };
            let word = found.as_str();
            if word.is_empty() || word == current || word.parse::<f64>().is_ok() {
                continue;
            }
            if seen.insert(word.to_string()) {
                items.push(CompletionItem {
                    label: word.to_string().into(),
                    kind: CompletionItemKind::Text,
                    range: Some(PointUtf16::new(position.row, word_start)..position),
                    replace_range: Some(
                        PointUtf16::new(position.row, word_start)
                            ..PointUtf16::new(position.row, word_end),
                    ),
                    ..Default::default()
                });
                if items.len() >= MAX_WORDS {
                    return items;
                }
            }
        }
    }
    items
}
