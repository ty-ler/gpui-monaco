//! The suggest controller (Monaco's `contrib/suggest` `SuggestController`
//! and `SuggestModel`): when to ask completion providers, live filtering
//! while typing, keyboard navigation and accepting items.

use gpui::{AppContext as _, Context, Entity, Task};
use text::PointUtf16;

use crate::cursor::{EditOperation, EditOperationType, Selection, StandardTokenType};
use crate::editor::CodeEditor;
use crate::providers::{CompletionItem, CompletionList, CompletionTrigger, completion_providers};
use crate::suggest::{CompletionModel, LineContext};
use crate::suggest_widget::SuggestWidget;
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

pub(crate) struct Session {
    /// Where the request was made.
    position: PointUtf16,
    /// Start column of the word being completed.
    word_start: u32,
    pub(crate) model: CompletionModel,
    pub(crate) selected: usize,
    incomplete: bool,
}

#[derive(Default)]
pub(crate) struct SuggestState {
    pub(crate) session: Option<Session>,
    task: Option<Task<()>>,
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
        self.suggest.session = None;
        if let Some((_, id)) = self.suggest.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Asks the completion providers for suggestions at the cursor
    /// (Ctrl+Space or a trigger).
    pub fn trigger_suggest(&mut self, trigger: CompletionTrigger, cx: &mut Context<Self>) {
        let position = self.cursors.primary().position;
        let model = self.model().read(cx);
        let context = model.provider_context();
        let language = model.language_id().map(|l| l.to_string());
        let tasks: Vec<_> = completion_providers(cx, language.as_deref())
            .into_iter()
            .map(|provider| provider.provide_completions(&context, position, trigger.clone(), cx))
            .collect();
        if tasks.is_empty() {
            return;
        }
        let word_start = self.word_start_at(position);
        self.suggest.task = Some(cx.spawn(async move |this, cx| {
            let mut list = CompletionList::default();
            for task in tasks {
                if let Ok(result) = task.await {
                    list.incomplete |= result.incomplete;
                    list.items.extend(result.items);
                }
            }
            this.update(cx, |this, cx| {
                this.start_session(list, position, word_start, cx)
            })
            .ok();
        }));
    }

    fn start_session(
        &mut self,
        list: CompletionList,
        position: PointUtf16,
        word_start: u32,
        cx: &mut Context<Self>,
    ) {
        let head = self.cursors.primary().position;
        if list.items.is_empty() || head.row != position.row || head.column < word_start {
            self.hide_suggest(cx);
            return;
        }
        let line_context = self.line_context_at(head, position.column);
        let model = CompletionModel::new(list.items, position, word_start, line_context);
        let selected = model.preselected_index();
        self.suggest.session = Some(Session {
            position,
            word_start,
            model,
            selected,
            incomplete: list.incomplete,
        });
        self.refresh_suggest_widget(cx);
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

    /// Called after `text` was typed (before signature help).
    pub(crate) fn suggest_after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(ch) = text.chars().last() else {
            return;
        };
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let providers = completion_providers(cx, language.as_deref());
        if providers.is_empty() {
            return;
        }
        if providers
            .iter()
            .any(|p| p.trigger_characters().contains(&ch))
        {
            self.trigger_suggest(CompletionTrigger::TriggerCharacter(ch), cx);
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
        // Monaco quickSuggestions: on for code, off in strings and comments.
        if is_word_char(ch) && self.options.quick_suggestions {
            let head = self.cursors.primary().position;
            let offset = self.snapshot.point_utf16_to_offset(head).saturating_sub(1);
            let token = self.model().read(cx).token_type_at(offset);
            if !matches!(
                token,
                StandardTokenType::String | StandardTokenType::Comment
            ) {
                self.trigger_suggest(CompletionTrigger::Invoke, cx);
            }
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
            self.trigger_suggest(CompletionTrigger::Incomplete, cx);
            return;
        }
        let line_context = self.line_context_at(head, session.position.column);
        if let Some(session) = self.suggest.session.as_mut() {
            session.model.set_line_context(line_context);
            session.selected = session.model.preselected_index();
        }
        self.refresh_suggest_widget(cx);
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
        true
    }

    pub(crate) fn select_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(session) = self.suggest.session.as_mut() {
            session.selected = index;
        }
        self.refresh_suggest_widget(cx);
    }

    /// Accepts the selected suggestion (Enter/Tab). Returns whether one was
    /// accepted.
    pub fn accept_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.suggest.session.take() else {
            return false;
        };
        let Some(scored) = session.model.filtered().get(session.selected) else {
            self.hide_suggest(cx);
            return false;
        };
        let item: CompletionItem = session.model.items()[scored.index].clone();
        self.hide_suggest(cx);
        self.insert_completion(&item, session.position, session.word_start, cx);
        true
    }

    fn insert_completion(
        &mut self,
        item: &CompletionItem,
        position: PointUtf16,
        word_start: u32,
        cx: &mut Context<Self>,
    ) {
        let head = self.cursors.primary().position;
        // Each cursor replaces as much before it as the primary does (Monaco
        // `overwriteBefore`), up to the cursor (insert mode).
        let (overwrite_before, overwrite_after) = match &item.range {
            Some(range) if range.start.row == head.row => (
                head.column.saturating_sub(range.start.column),
                range.end.column.saturating_sub(head.column),
            ),
            _ => (head.column.saturating_sub(word_start), 0),
        };
        let _ = position;
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
        edits.extend(item.additional_text_edits.iter().cloned());
        edits.sort_by_key(|(range, _)| range.start);
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
    }

    /// If `ch` is a commit character of the selected suggestion, accepts it
    /// first. Returns whether it did.
    pub(crate) fn commit_suggestion_on(&mut self, ch: char, cx: &mut Context<Self>) -> bool {
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
