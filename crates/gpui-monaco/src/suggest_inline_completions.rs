//! Completion items as ghost text (Monaco
//! `contrib/suggest/browser/suggestInlineCompletions.ts`
//! `SuggestInlineCompletions`): where `quick_suggestions` is
//! [`QuickSuggestionsValue::Inline`] for the token being typed, an inline
//! completions provider asks the completion providers at the end of a word
//! (or after a trigger character) and offers the items that match what's
//! typed, the one the selection memory picks first, as ghost text.
//!
//! Monaco registers it for every language; here each editor has one, asked
//! after the registered inline completions providers while one of its
//! `quick_suggestions` values is `Inline`.

use std::sync::{Arc, Mutex};

use gpui::{App, SharedString, Task, WeakEntity};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::model::TextModel;
use crate::options::{QuickSuggestions, QuickSuggestionsValue, SuggestSelection};
use crate::providers::{
    CompletionItem, CompletionItemKind, CompletionProvider, CompletionTrigger, InlineCompletion,
    InlineCompletionContext, InlineCompletionList, InlineCompletionsProvider, ProviderContext,
    ProviderTarget, completion_provider_groups, completion_providers, inline_completions_providers,
};
use crate::suggest::memory::{Candidate, MemoryContext};
use crate::suggest::{
    CompletionModel, CompletionModelOptions, FuzzyScore, FuzzyScoreOptions, LineContext,
};

/// The editor options the provider reads, refreshed before each request.
#[derive(Clone)]
struct Config {
    model: WeakEntity<TextModel>,
    quick_suggestions: QuickSuggestions,
    /// Monaco `SuggestModel.createSuggestFilter`'s kinds.
    hidden_kinds: Vec<CompletionItemKind>,
    model_options: CompletionModelOptions,
    selection: SuggestSelection,
    /// Whose selection memories pick the first item.
    memory: crate::completion::MemoryScope,
}

/// Monaco `InlineCompletionResults`: a request's completion model, refiltered
/// while the same word grows.
struct Results {
    uri: Option<SharedString>,
    row: u32,
    /// The word at the request (start, end columns).
    word: (u32, u32),
    model: CompletionModel,
    incomplete: bool,
}

impl Results {
    /// Monaco `canBeReused`: the same model and line, a non-empty word that
    /// starts in the same place and got longer, and nothing incomplete.
    fn can_be_reused(&self, uri: Option<&SharedString>, row: u32, word: (u32, u32)) -> bool {
        self.uri.as_ref() == uri
            && self.row == row
            && self.word.1 > self.word.0
            && self.word.0 == word.0
            && self.word.1 < word.1
            && !self.incomplete
    }

    /// Monaco `InlineCompletionResults.items`: the items with a match, from
    /// the remembered one (`suggest_selection`) to the end, then the ones
    /// before it.
    fn items(
        &self,
        language: &str,
        line: &str,
        selection: SuggestSelection,
        memory: crate::completion::MemoryScope,
        cx: &mut App,
    ) -> Vec<InlineCompletion> {
        let delta = self.model.line_context().character_count_delta as u32;
        let filtered = self.model.filtered();
        let items = self.model.items();
        let column = self.word.1 + delta;
        let word_until: String = line
            .encode_utf16()
            .skip(self.word.0 as usize)
            .take(column.saturating_sub(self.word.0) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        let candidates: Vec<Candidate> = filtered
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
            language,
            line,
            column,
            word_until: &word_until,
        };
        let selected =
            crate::completion::memory_select(memory, selection, &context, &candidates, cx)
                .min(filtered.len());
        filtered[selected..]
            .iter()
            .chain(&filtered[..selected])
            // Skip items that have no overlap.
            .filter(|scored| {
                scored.score.score != FuzzyScore::DEFAULT_SCORE || !scored.score.matches.is_empty()
            })
            .map(|scored| {
                let item = &items[scored.index];
                let start = item
                    .range
                    .as_ref()
                    .map_or(PointUtf16::new(self.row, self.word.0), |r| r.start);
                let end = item
                    .range
                    .as_ref()
                    .map_or(PointUtf16::new(self.row, self.word.1), |r| r.end);
                InlineCompletion {
                    insert_text: item.insert_text().to_string(),
                    is_snippet: item.insert_text_is_snippet,
                    // The end plus what was typed since.
                    range: Some(start..PointUtf16::new(end.row, end.column + delta)),
                    command: item.command.clone().or_else(|| item.action.clone()),
                    additional_text_edits: item.additional_text_edits.clone(),
                    ..Default::default()
                }
            })
            .collect()
    }
}

/// Monaco `SuggestInlineCompletions`.
pub(crate) struct SuggestInlineCompletions {
    config: Mutex<Option<Config>>,
    /// Monaco `_lastResult`.
    last: Arc<Mutex<Option<Results>>>,
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

/// The word around `column` on `line` (Monaco `getWordAtPosition`), as
/// UTF-16 start and end columns; empty when there's none.
fn word_at(line: &str, column: u32) -> (u32, u32) {
    let units: Vec<(u32, char)> = {
        let mut at = 0;
        line.chars()
            .map(|ch| {
                let start = at;
                at += ch.len_utf16() as u32;
                (start, ch)
            })
            .collect()
    };
    let mut start = column;
    for &(at, ch) in units.iter().rev() {
        if at >= column {
            continue;
        }
        if !is_word_char(ch) {
            break;
        }
        start = at;
    }
    let mut end = column;
    for &(at, ch) in &units {
        if at < column {
            continue;
        }
        if !is_word_char(ch) {
            break;
        }
        end = at + ch.len_utf16() as u32;
    }
    (start, end)
}

impl SuggestInlineCompletions {
    fn new() -> Self {
        Self {
            config: Mutex::new(None),
            last: Arc::new(Mutex::new(None)),
        }
    }

    /// Monaco `_getTriggerCharacterInfo`: the character before the cursor,
    /// if it's a trigger character of some providers (those providers).
    fn trigger_character_providers(
        line: &str,
        column: u32,
        providers: &[Arc<dyn CompletionProvider>],
    ) -> Option<(char, Vec<Arc<dyn CompletionProvider>>)> {
        let mut at = 0;
        let ch = line.chars().find(|ch| {
            at += ch.len_utf16() as u32;
            at == column
        })?;
        let providers: Vec<_> = providers
            .iter()
            .filter(|p| p.trigger_characters().contains(&ch))
            .cloned()
            .collect();
        (!providers.is_empty()).then_some((ch, providers))
    }
}

impl InlineCompletionsProvider for SuggestInlineCompletions {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        completion_context: InlineCompletionContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        let empty = || Task::ready(Ok(InlineCompletionList::default()));
        if completion_context.selected_suggestion_info.is_some() {
            return empty();
        }
        let Some(config) = self.config.lock().ok().and_then(|c| c.clone()) else {
            return empty();
        };
        if config.quick_suggestions.is_all_off() {
            // Quick suggestions are off (for this model or language).
            return empty();
        }
        let Some(model) = config.model.upgrade() else {
            return empty();
        };
        let (line, token) = {
            let model = model.read(cx);
            let rope = model.buffer().rope();
            let offset =
                rope.point_utf16_to_offset(rope.clip_point_utf16(position, text::Bias::Left));
            (
                rope.line(position.row),
                model.token_type_at(offset.saturating_sub(1)),
            )
        };
        if config.quick_suggestions.value_for(token) != QuickSuggestionsValue::Inline {
            // Quick suggestions are off for this token.
            return empty();
        }
        let target = ProviderTarget::from(model.read(cx));
        let all_providers = completion_providers(cx, target.clone());
        // A word, or else a trigger character (word characters win over
        // trigger characters).
        let mut word = word_at(&line, position.column);
        let mut trigger = None;
        if word.0 == word.1 {
            trigger = Self::trigger_character_providers(&line, position.column, &all_providers);
            if trigger.is_none() {
                return empty();
            }
            // Monaco `getWordUntilPosition`.
            word = (position.column, position.column);
        }
        if word.1 != position.column {
            // Not at the end of the word: no quick suggestions inside words.
            return empty();
        }
        let leading: String = line
            .encode_utf16()
            .take(position.column as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        let language = context
            .language_id
            .as_ref()
            .map_or_else(|| "plaintext".to_string(), |l| l.to_string());
        // Reuse the last results when only the word grew: just refilter.
        if trigger.is_none()
            && let Ok(mut last) = self.last.lock()
            && let Some(results) = last.as_mut()
            && results.can_be_reused(context.uri.as_ref(), position.row, word)
        {
            let delta = position.column - results.word.1;
            results.model.set_line_context(LineContext {
                leading_line_content: leading,
                character_count_delta: delta as usize,
            });
            let items = results.items(&language, &line, config.selection, config.memory, cx);
            return Task::ready(Ok(InlineCompletionList {
                items,
                ..Default::default()
            }));
        }
        // Monaco `provideSuggestionItems`: provider groups best first, the
        // first with items winning; a trigger character asks only the
        // providers declaring it (`providerFilter`).
        let (filter, kind) = match trigger {
            Some((ch, providers)) => (Some(providers), CompletionTrigger::TriggerCharacter(ch)),
            None => (None, CompletionTrigger::Invoke),
        };
        // Monaco's kind filter (deprecated items stay).
        let hidden_kinds = config.hidden_kinds.clone();
        let request = crate::completion::provide_suggestion_items(
            completion_provider_groups(cx, target),
            crate::completion::SuggestRequestOptions {
                filter,
                reuse: Vec::new(),
            },
            context.clone(),
            position,
            kind,
            move |item| !hidden_kinds.contains(&item.kind),
            cx,
        );
        let uri = context.uri.clone();
        let last = self.last.clone();
        cx.spawn(async move |cx| {
            let mut items: Vec<CompletionItem> = Vec::new();
            let mut incomplete = false;
            for list in request.await {
                incomplete |= list.incomplete;
                items.extend(list.items);
            }
            let model = CompletionModel::new_with_options(
                items,
                position,
                word.0,
                LineContext {
                    leading_line_content: leading,
                    character_count_delta: 0,
                },
                config.model_options,
            );
            let results = Results {
                uri,
                row: position.row,
                word,
                model,
                incomplete,
            };
            let items = cx
                .update(|cx| results.items(&language, &line, config.selection, config.memory, cx));
            if let Ok(mut last) = last.lock() {
                *last = Some(results);
            }
            Ok(InlineCompletionList {
                items,
                ..Default::default()
            })
        })
    }
}

impl CodeEditor {
    /// The inline completions providers this editor asks: the registered
    /// ones, then its [`SuggestInlineCompletions`] while a
    /// `quick_suggestions` value is `Inline`.
    pub(crate) fn editor_inline_completions_providers(
        &mut self,
        cx: &App,
    ) -> Vec<Arc<dyn InlineCompletionsProvider>> {
        let mut providers = inline_completions_providers(cx, self.provider_target(cx));
        let quick = self.options.quick_suggestions;
        if ![quick.other, quick.comments, quick.strings].contains(&QuickSuggestionsValue::Inline) {
            return providers;
        }
        let mut hidden_kinds = self.options.suggest.hidden_kinds.clone();
        if self.options.snippet_suggestions.is_none() {
            hidden_kinds.push(CompletionItemKind::Snippet);
        }
        let config = Config {
            model: self.model().downgrade(),
            quick_suggestions: quick,
            hidden_kinds,
            model_options: CompletionModelOptions {
                filter_graceful: self.options.suggest.filter_graceful,
                snippet_sort_order: self.options.snippet_suggestions.unwrap_or_default(),
                fuzzy_score_options: FuzzyScoreOptions {
                    boost_full_match: false,
                    first_match_can_be_weak: false,
                },
            },
            selection: self.options.suggest_selection,
            memory: self.memory_scope(),
        };
        let provider = self
            .suggest
            .inline_provider
            .get_or_insert_with(|| Arc::new(SuggestInlineCompletions::new()));
        if let Ok(mut current) = provider.config.lock() {
            *current = Some(config);
        }
        providers.push(provider.clone());
        providers
    }
}

#[cfg(test)]
mod tests {
    //! Monaco `suggestInlineCompletions.test.ts`.

    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};

    use super::*;
    use crate::providers::{
        CompletionList, InlineCompletionTriggerKind, LanguageSelector, register_completion_provider,
    };

    /// Monaco's test provider: `hello`, `hell` and the snippet `hey`.
    struct Hello(Rc<Cell<usize>>);

    impl CompletionProvider for Hello {
        fn provide_completions(
            &self,
            _: &ProviderContext,
            _: PointUtf16,
            _: CompletionTrigger,
            _: &mut App,
        ) -> Task<anyhow::Result<CompletionList>> {
            self.0.set(self.0.get() + 1);
            Task::ready(Ok(CompletionList {
                items: vec![
                    CompletionItem::new("hello", CompletionItemKind::Class),
                    CompletionItem::new("hell", CompletionItemKind::Class),
                    CompletionItem::new("hey", CompletionItemKind::Snippet),
                ],
                incomplete: false,
            }))
        }
    }

    fn setup(
        cx: &mut TestAppContext,
    ) -> (Entity<CodeEditor>, &mut VisualTestContext, Rc<Cell<usize>>) {
        let requests = Rc::new(Cell::new(0));
        cx.update(|cx| {
            crate::init(cx);
            register_completion_provider(cx, LanguageSelector::All, Hello(requests.clone()));
        });
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let model = cx.new(|_| TextModel::new("he"));
            let mut editor = CodeEditor::new(model, window, cx);
            let mut options = editor.options().clone();
            options.quick_suggestions = QuickSuggestions::INLINE;
            editor.update_options(options, cx);
            editor
        });
        (editor, cx, requests)
    }

    /// What the editor's suggest inline completions offer at `position`,
    /// sorted, or `None` without any.
    async fn provide(
        editor: &Entity<CodeEditor>,
        position: PointUtf16,
        cx: &mut VisualTestContext,
    ) -> Option<Vec<String>> {
        let task = editor.update(cx, |editor, cx| {
            let provider = editor.editor_inline_completions_providers(cx).pop()?;
            let context = editor.model().read(cx).provider_context();
            Some(provider.provide_inline_completions(
                &context,
                position,
                InlineCompletionContext {
                    trigger_kind: InlineCompletionTriggerKind::Explicit,
                    selected_suggestion_info: None,
                    include_inline_edits: false,
                    include_inline_completions: true,
                    ..Default::default()
                },
                cx,
            ))
        })?;
        let list = task.await.ok()?;
        let mut items: Vec<String> = list.items.into_iter().map(|i| i.insert_text).collect();
        items.sort();
        (!items.is_empty()).then_some(items)
    }

    #[gpui::test]
    async fn aggressive_inline_completions_when_typing_within_line_146948(cx: &mut TestAppContext) {
        let (editor, cx, _) = setup(cx);
        // End of word: suggestions.
        let items = provide(&editor, PointUtf16::new(0, 2), cx).await;
        assert_eq!(items.map(|i| i.len()), Some(3));
        // Middle of word: none.
        assert_eq!(provide(&editor, PointUtf16::new(0, 1), cx).await, None);
    }

    #[gpui::test]
    async fn snippets_show_in_inline_suggestions_even_though_they_are_turned_off_175190(
        cx: &mut TestAppContext,
    ) {
        let (editor, cx, _) = setup(cx);
        // Unfiltered.
        let items = provide(&editor, PointUtf16::new(0, 2), cx).await;
        assert_eq!(items.map(|i| i.len()), Some(3));
        // Filtered (Monaco `suggest.showSnippets: false`).
        editor.update(cx, |editor, cx| {
            let mut options = editor.options().clone();
            options.suggest.hidden_kinds = vec![CompletionItemKind::Snippet];
            editor.update_options(options, cx);
        });
        let items = provide(&editor, PointUtf16::new(0, 2), cx).await;
        assert_eq!(items.map(|i| i.len()), Some(2));
    }

    #[gpui::test]
    async fn reuses_results_while_the_word_grows(cx: &mut TestAppContext) {
        let (editor, cx, requests) = setup(cx);
        let items = provide(&editor, PointUtf16::new(0, 2), cx).await;
        assert_eq!(items.map(|i| i.len()), Some(3));
        editor.update(cx, |editor, cx| {
            editor.set_selections(
                &[crate::cursor::Selection::collapsed(PointUtf16::new(0, 2))],
                cx,
            );
            editor.type_text("l", cx);
        });
        // Refiltered without asking again: `hey` no longer matches.
        let items = provide(&editor, PointUtf16::new(0, 3), cx).await;
        assert_eq!(items, Some(vec!["hell".to_string(), "hello".to_string()]));
        assert_eq!(requests.get(), 1);
    }

    #[gpui::test]
    fn not_asked_without_an_inline_value(cx: &mut TestAppContext) {
        let (editor, cx, _) = setup(cx);
        editor.update(cx, |editor, cx| {
            let mut options = editor.options().clone();
            options.quick_suggestions = QuickSuggestions::ON;
            editor.update_options(options, cx);
            assert!(editor.editor_inline_completions_providers(cx).is_empty());
        });
    }

    #[test]
    fn words_around_a_column() {
        assert_eq!(word_at("he", 2), (0, 2));
        assert_eq!(word_at("he", 1), (0, 2));
        assert_eq!(word_at("a.b", 2), (2, 3));
        assert_eq!(word_at("a. b", 2), (2, 2));
    }
}
