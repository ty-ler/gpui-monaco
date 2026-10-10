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
use crate::suggest_widget::SuggestWidget;
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

/// Monaco stops collecting document words past this many.
const MAX_WORDS: usize = 10_000;

/// Selection memories, shared by all editors (Monaco keeps them per
/// profile or workspace).
#[derive(Default)]
struct Memories(SuggestMemories);

impl gpui::Global for Memories {}

pub(crate) struct Session {
    /// Where the request was made.
    position: PointUtf16,
    /// Start column of the word being completed.
    word_start: u32,
    /// End column of the word at `position` (the default replace range).
    word_end: u32,
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
    fn word_start_at(&self, position: PointUtf16) -> u32 {
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
        let words = (self.options.word_based_suggestions != WordBasedSuggestions::Off).then(|| {
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
        if items.is_empty() || head.row != position.row || head.column < word_start {
            self.hide_suggest(cx);
            return;
        }
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
        let model =
            CompletionModel::new_with_options(items, position, word_start, line_context, options);
        self.suggest.session = Some(Session {
            position,
            word_start,
            word_end,
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
        self.refresh_suggest_widget(cx);
        self.resolve_selected_suggestion(cx);
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
        if session.model.filtered().is_empty() {
            self.hide_suggest(cx);
            return;
        }
        let anchor = PointUtf16::new(session.position.row, session.word_start);
        let state = crate::suggest_widget::SuggestWidgetState::from_session(session);
        match &self.suggest.widget {
            Some((widget, id)) => {
                let id = *id;
                widget.update(cx, |widget, cx| widget.set_state(state, cx));
                self.set_content_widget_position(id, anchor, cx);
            }
            None => {
                let editor = cx.entity().downgrade();
                let theme = self.theme.clone();
                let style = self.markdown_style();
                let line_height = self.options.resolved_line_height();
                let widget =
                    cx.new(|_| SuggestWidget::new(editor, state, theme, style, line_height));
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
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let providers = completion_providers(cx, language.as_deref());
        if self.options.suggest_on_trigger_characters
            && providers
                .iter()
                .any(|p| p.trigger_characters().contains(&ch))
        {
            self.request_suggestions(CompletionTrigger::TriggerCharacter(ch), true, cx);
            return;
        }
        if self.suggest.session.is_some() || self.suggest.task.is_some() {
            if is_word_char(ch) {
                self.refilter_suggest(cx);
            } else {
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

    /// Monaco `SuggestModel._doTriggerQuickSuggest`.
    fn quick_suggest(&mut self, cx: &mut Context<Self>) {
        if self.suggest.session.is_some() || self.suggest.task.is_some() {
            return;
        }
        let quick = self.options.quick_suggestions;
        if quick == crate::options::QuickSuggestions::OFF {
            return;
        }
        // `shouldAutoTrigger`: a word ending at the cursor (or one character
        // in), not a number.
        let head = self.cursors.primary().position;
        let start = self.word_start_at(head);
        let end = self.word_end_at(head);
        if start == head.column || (end != head.column && start + 1 != head.column) {
            return;
        }
        let word: String = self
            .snapshot
            .line(head.row)
            .encode_utf16()
            .skip(start as usize)
            .take((end - start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        if word.parse::<f64>().is_ok() {
            return;
        }
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

    /// Re-filters after typing or deleting, or closes when the cursor left
    /// the word.
    pub(crate) fn refilter_suggest(&mut self, cx: &mut Context<Self>) {
        let head = self.cursors.primary().position;
        let Some(session) = &self.suggest.session else {
            return;
        };
        if head.row != session.position.row || head.column < session.word_start {
            self.hide_suggest(cx);
            return;
        }
        if session.incomplete {
            let auto = session.auto;
            self.request_suggestions(CompletionTrigger::Incomplete, auto, cx);
            return;
        }
        let line_context = self.line_context_at(head, session.position.column);
        if let Some(session) = self.suggest.session.as_mut() {
            session.model.set_line_context(line_context);
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
        if !item.additional_text_edits.is_empty() || item.kind == CompletionItemKind::Snippet {
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
        self.insert_completion(&item, &session, replace, cx);
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
            if command.id == "editor.action.triggerSuggest" {
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
        session: &Session,
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
        let text = item.insert_text().to_string();
        let inserted_utf16 = text.encode_utf16().count() as i64;
        let mut edits = Vec::new();
        let mut selections_after = Vec::new();
        let mut shift: i64 = 0;
        let mut last_row = None;
        let mut selections = self.cursors.selections();
        selections.sort_by_key(|s| s.head);
        for selection in &selections {
            let at = selection.head;
            let start = PointUtf16::new(at.row, at.column.saturating_sub(overwrite_before));
            let end = PointUtf16::new(
                at.row,
                (at.column + overwrite_after).min(self.snapshot.line_len_utf16(at.row)),
            );
            if last_row != Some(at.row) {
                shift = 0;
                last_row = Some(at.row);
            }
            edits.push((start..end, text.clone()));
            let column = (start.column as i64 + shift + inserted_utf16) as u32;
            selections_after.push(Selection::collapsed(PointUtf16::new(at.row, column)));
            shift += inserted_utf16 - (end.column - start.column) as i64;
        }
        if text.contains('\n') {
            // Multi-line insert text: place the cursor at the end of the
            // last line of the inserted text.
            selections_after = edits
                .iter()
                .map(|(range, text)| {
                    let lines: Vec<&str> = text.split('\n').collect();
                    let last = lines.last().copied().unwrap_or("");
                    Selection::collapsed(PointUtf16::new(
                        range.start.row + lines.len() as u32 - 1,
                        last.encode_utf16().count() as u32,
                    ))
                })
                .collect();
        }
        if item.additional_text_edits.is_empty() {
            self.apply(
                EditOperation {
                    edits,
                    selections_after,
                    kind: EditOperationType::Other,
                    push_stack_element_before: true,
                    push_stack_element_after: true,
                    auto_closed: Vec::new(),
                    follow_up: None,
                },
                cx,
            );
            return;
        }
        // Monaco applies additional edits first: they go before the
        // insertions at the same place, and the cursors (at the end of each
        // insertion, relative to its start) shift with them.
        let main_count = edits.len();
        let heads: Vec<PointUtf16> = selections_after.iter().map(|s| s.head).collect();
        let mut all: Vec<(bool, std::ops::Range<PointUtf16>, String)> = item
            .additional_text_edits
            .iter()
            .map(|(range, text)| (false, range.clone(), text.clone()))
            .chain(edits.into_iter().map(|(range, text)| (true, range, text)))
            .collect();
        all.sort_by_key(|(_, range, _)| range.start);
        // Each insertion's start in the new text, and the cursor's offset
        // from that start.
        let mut delta: isize = 0;
        let mut cursor_offsets = Vec::with_capacity(main_count);
        let mut main_ix = 0;
        for (is_main, range, text) in &all {
            let start = self.snapshot.point_utf16_to_offset(range.start);
            let end = self.snapshot.point_utf16_to_offset(range.end);
            if *is_main {
                let head = heads[main_ix];
                main_ix += 1;
                // The cursor relative to the insertion start, in bytes of
                // the inserted text.
                let rel = if head.row == range.start.row {
                    let columns = head.column.saturating_sub(range.start.column) as usize;
                    let mut utf16 = 0;
                    text.chars()
                        .take_while(|ch| {
                            utf16 += ch.len_utf16();
                            utf16 <= columns
                        })
                        .map(char::len_utf8)
                        .sum()
                } else {
                    text.len()
                };
                cursor_offsets.push((start as isize + delta) as usize + rel);
            }
            delta += text.len() as isize - (end - start) as isize;
        }
        let edits: Vec<_> = all
            .into_iter()
            .map(|(_, range, text)| (range, text))
            .collect();
        self.apply(
            EditOperation {
                edits,
                selections_after,
                kind: EditOperationType::Other,
                push_stack_element_before: true,
                push_stack_element_after: true,
                auto_closed: Vec::new(),
                follow_up: None,
            },
            cx,
        );
        let selections: Vec<Selection> = cursor_offsets
            .into_iter()
            .map(|offset| {
                let offset = offset.min(self.snapshot.len());
                Selection::collapsed(self.snapshot.offset_to_point_utf16(offset))
            })
            .collect();
        self.set_selections(&selections, cx);
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
