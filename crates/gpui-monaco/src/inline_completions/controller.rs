//! The inline completions controller (Monaco's `InlineCompletionsModel`,
//! `InlineCompletionsSource` and `InlineCompletionsController`): requests
//! completions while typing, keeps them through edits that match them,
//! shows the selected one as ghost text after the cursor (extra lines in a
//! view zone), and accepts all of it, a word or a line.
//!
//! Inline edits (items with `is_inline_edit`) are diffed into replacements
//! that follow the user's typing (Monaco `InlineEditItem`); the last one
//! shows when no completion is visible at the cursor, through
//! [`super::inline_edits_view`]. Tab jumps to it or accepts it, as Monaco's
//! `tabShouldJumpToInlineEdit` and `tabShouldAcceptInlineEdit` say.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use web_time::Instant;

use gpui::{
    AppContext as _, Context, IntoElement, ParentElement, Render, SharedString, Styled, Task,
    Window, div, px,
};
use text::PointUtf16;

use super::ghost_text::{
    GhostText, GhostTextMode, TextReplacement, compute_ghost_text, single_text_remove_common_prefix,
};
use super::inline_edits::{Changes, Replacements, UpdatedEdit, diffed_edits, new_ranges};
use crate::cursor::Selection;
use crate::decorations::{DecorationId, DecorationOptions, InjectedText};
use crate::editor::CodeEditor;
use crate::events::{CursorChangeReason, CursorChangeSource, SelectionChange};
use crate::options::InlineSuggestMode;
use crate::providers::{
    Command, InlineCompletion, InlineCompletionContext, InlineCompletionEndOfLifeReason,
    InlineCompletionList, InlineCompletionTriggerKind, InlineCompletionsProvider,
    inline_completions_providers,
};
use crate::theme::TokenStyle;
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};

/// The editor's debounce when providers don't set one (Monaco configures
/// its debounce service with 50ms).
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(50);

/// The visible items (index, current edit), the selected one, and its
/// ghost text.
type InlineView = (Vec<(usize, TextReplacement)>, usize, GhostText);

/// A completion and where it applies, followed through edits.
#[derive(Clone)]
pub(crate) struct Item {
    pub(crate) provider: Arc<dyn InlineCompletionsProvider>,
    pub(crate) completion: InlineCompletion,
    /// Its range as byte offsets in the text of `version`.
    range: Range<usize>,
    version: text::Version,
    /// Where it started, to drop it if text before it is deleted.
    original_start: PointUtf16,
    /// It showed as ghost text (Monaco `_didShow`).
    shown: bool,
    /// Its identity, kept by an equal item from a newer request (Monaco
    /// `InlineSuggestionIdentity`).
    pub(crate) id: u64,
    /// Its list's commands (Monaco `source.inlineSuggestions.commands`).
    pub(crate) commands: Vec<Command>,
    /// Where it may show, in the text of `version` (Monaco `showRange`).
    show_range: Option<Range<usize>>,
    /// An inline edit's replacements (Monaco `InlineEditItem`).
    edit: Option<InlineEditItem>,
}

/// An inline edit's replacements, followed through every change.
#[derive(Clone)]
struct InlineEditItem {
    edits: Vec<UpdatedEdit>,
    /// The text version `edits` are in.
    version: text::Version,
    /// The change count when it was created or typing last matched it
    /// (Monaco `updatedEditModelVersion`).
    updated_at: u64,
}

/// Monaco drops an inline edit ignored for this many changes.
const INLINE_EDIT_MAX_IGNORED_CHANGES: u64 = 20;

/// Why inline completions stop (Monaco `stop`'s reason, or accepting).
#[derive(Clone, Copy, PartialEq, Eq)]
enum StopReason {
    /// Escape or `hide`: the shown completion is rejected.
    Explicit,
    /// The cursor moved or the editor lost focus.
    Automatic,
    /// The item at this index was accepted.
    Accepted(usize),
}

/// What is rendered: the ghost text and its decorations and zone.
struct Rendered {
    key: (text::Version, PointUtf16, usize, bool),
    decorations: Vec<DecorationId>,
    zone: Option<WidgetId>,
}

#[derive(Default)]
pub(crate) struct InlineCompletionsState {
    /// Monaco's `_isActive`: typing (or a trigger) turned it on.
    active: bool,
    pub(crate) items: Vec<Item>,
    /// The selected item's identity (its edit), kept through updates.
    selected: Option<(String, Range<PointUtf16>)>,
    /// The last request was explicit (cycling shows a count).
    explicit: bool,
    /// The version and position the last request was for.
    requested: Option<(text::Version, PointUtf16, InlineCompletionTriggerKind)>,
    task: Option<Task<()>>,
    rendered: Option<Rendered>,
    /// Edits the controller makes itself (accepting) keep the items.
    updating: bool,
    /// Items already reported as shown.
    shown: HashSet<String>,
    /// The list asked to keep the suggest widget closed.
    suppress_suggestions: bool,
    /// The lists' extra commands for the toolbar (Monaco `commands`).
    pub(crate) commands: Vec<Command>,
    /// The always-shown toolbar and where it is (Monaco
    /// `InlineCompletionsHintsWidget`).
    pub(crate) toolbar: Option<(gpui::Entity<super::toolbar::InlineToolbar>, WidgetId)>,
    /// The toolbar's position this session, so it doesn't move right as
    /// the ghost text shrinks (Monaco `sessionPosition`).
    toolbar_session: Option<PointUtf16>,
    /// Text changes seen (Monaco's model version id, for inline edits).
    change_count: u64,
    /// The next item identity.
    next_id: u64,
    /// Monaco `_inAcceptFlow`: an edit was just accepted (or requested
    /// explicitly), and no other change came since.
    in_accept_flow: bool,
    /// The inline edit jumped to, until the cursor leaves it (Monaco
    /// `_jumpedToId`).
    jumped_to: Option<u64>,
    /// The inline edit's view.
    pub(crate) edit_view: super::inline_edits_view::InlineEditViewState,
}

/// Monaco `provideInlineCompletions`' yield graph: for each provider, the
/// providers in the groups it yields to, without edges that close a cycle.
fn yield_graph(providers: &[Arc<dyn InlineCompletionsProvider>]) -> Vec<Vec<usize>> {
    let groups: Vec<Option<SharedString>> = providers.iter().map(|p| p.group_id()).collect();
    let mut graph: Vec<Vec<usize>> = providers
        .iter()
        .map(|p| {
            let yields = p.yields_to_group_ids();
            (0..providers.len())
                .filter(|&j| groups[j].as_ref().is_some_and(|g| yields.contains(g)))
                .collect()
        })
        .collect();
    // Monaco `DirectedGraph.removeCycles`: a depth-first search drops edges
    // back to a provider still being visited.
    fn visit(node: usize, graph: &mut Vec<Vec<usize>>, state: &mut Vec<u8>) {
        state[node] = 1;
        let mut ix = 0;
        while ix < graph[node].len() {
            let next = graph[node][ix];
            match state[next] {
                1 => {
                    log::warn!("inline completions: cyclic yield-to dependency");
                    graph[node].remove(ix);
                    continue;
                }
                0 => visit(next, graph, state),
                _ => {}
            }
            ix += 1;
        }
        state[node] = 2;
    }
    let mut state = vec![0u8; providers.len()];
    for node in 0..providers.len() {
        if state[node] == 0 {
            visit(node, &mut graph, &mut state);
        }
    }
    graph
}

/// The providers in an order where each comes after those it yields to.
fn yield_order(graph: &[Vec<usize>]) -> Vec<usize> {
    fn visit(node: usize, graph: &[Vec<usize>], done: &mut Vec<bool>, order: &mut Vec<usize>) {
        if done[node] {
            return;
        }
        done[node] = true;
        for &next in &graph[node] {
            visit(next, graph, done, order);
        }
        order.push(node);
    }
    let mut done = vec![false; graph.len()];
    let mut order = Vec::new();
    for node in 0..graph.len() {
        visit(node, graph, &mut done, &mut order);
    }
    order
}

/// Ghost text split into pieces with their style (Monaco's ghost text
/// tokens).
type GhostSegments = Vec<(String, TokenStyle)>;

/// The additional lines of a ghost text, below its line.
pub struct GhostLines {
    lines: Vec<GhostSegments>,
    font: SharedString,
    font_size: gpui::Pixels,
    line_height: gpui::Pixels,
    left: gpui::Pixels,
    /// The editor's text color, for pieces without their own.
    foreground: gpui::Hsla,
}

/// `text` (a ghost text piece starting at byte `start` of the tokenized
/// text) split by the highlight `spans` covering it.
fn split_by_spans(
    text: &str,
    start: usize,
    spans: &[(Range<usize>, syntax::HighlightId)],
) -> Vec<(String, Option<syntax::HighlightId>)> {
    let end = start + text.len();
    // The innermost (last) span covering each byte.
    let mut ids: Vec<Option<syntax::HighlightId>> = vec![None; text.len()];
    for (range, id) in spans {
        if range.end <= start || range.start >= end {
            continue;
        }
        for slot in &mut ids[range.start.max(start) - start..range.end.min(end) - start] {
            *slot = Some(*id);
        }
    }
    let mut pieces: Vec<(String, Option<syntax::HighlightId>)> = Vec::new();
    for (ix, ch) in text.char_indices() {
        let id = ids[ix];
        match pieces.last_mut() {
            Some((piece, last)) if *last == id => piece.push(ch),
            _ => pieces.push((ch.to_string(), id)),
        }
    }
    pieces
}

impl Render for GhostLines {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .pl(self.left)
            .font_family(self.font.clone())
            .text_size(self.font_size)
            .line_height(self.line_height)
            .children(self.lines.iter().map(|line| {
                div()
                    .h(self.line_height)
                    .flex()
                    .whitespace_nowrap()
                    .children(line.iter().map(|(text, style)| {
                        let mut color = style.foreground.unwrap_or(self.foreground);
                        color.a *= style.opacity.unwrap_or(1.);
                        let span = div().text_color(color).child(text.clone());
                        let span = if style.italic { span.italic() } else { span };
                        if style.bold {
                            span.font_weight(gpui::FontWeight::BOLD)
                        } else {
                            span
                        }
                    }))
            }))
    }
}

/// Monaco `matchesSubString`: `word` is a case-insensitive subsequence of
/// `target`.
fn matches_sub_string(word: &str, target: &str) -> bool {
    let mut target = target.chars().flat_map(char::to_lowercase);
    word.chars()
        .flat_map(char::to_lowercase)
        .all(|c| target.any(|t| t == c))
}

fn utf16_split(text: &str, at: usize) -> (String, String) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let at = at.min(units.len());
    (
        String::from_utf16_lossy(&units[..at]),
        String::from_utf16_lossy(&units[at..]),
    )
}

/// Monaco `inlineCompletionIsVisible`: whether `edit` shows as ghost text
/// at `cursor`, i.e. the text typed so far matches it.
fn is_visible(
    edit: &TextReplacement,
    original_start: PointUtf16,
    cursor: PointUtf16,
    line_text: &dyn Fn(u32) -> String,
) -> bool {
    if edit.range.start != original_start {
        return false;
    }
    let m = single_text_remove_common_prefix(edit, line_text);
    if cursor.row != m.range.start.row || m.range.end.row != m.range.start.row {
        return false;
    }
    if m.range.start == m.range.end && m.text.is_empty() {
        return false;
    }
    let line = line_text(m.range.start.row);
    let units: Vec<u16> = line.encode_utf16().collect();
    let start = (m.range.start.column as usize).min(units.len());
    let end = (m.range.end.column as usize).min(units.len());
    let original = String::from_utf16_lossy(&units[start..end]);
    let filter = m.text.replace("\r\n", "\n");
    let index = cursor.column.saturating_sub(m.range.start.column) as usize;
    let (mut filter_before, mut filter_after) = utf16_split(&filter, index);
    let (mut original_before, mut original_after) = utf16_split(&original, index);
    let indentation = line.encode_utf16().count() as u32
        - line.trim_start_matches([' ', '\t']).encode_utf16().count() as u32;
    if m.range.start.column <= indentation {
        original_before = original_before.trim_start().to_string();
        if original_before.is_empty() {
            original_after = original_after.trim_start().to_string();
        }
        filter_before = filter_before.trim_start().to_string();
        if filter_before.is_empty() {
            filter_after = filter_after.trim_start().to_string();
        }
    }
    filter_before.starts_with(&original_before)
        && matches_sub_string(&original_after, &filter_after)
}

/// Monaco's partial accept: how much of `text` the next word is.
fn next_word_length(text: &str, word: &syntax::fancy_regex::Regex) -> usize {
    let units: Vec<u16> = text.encode_utf16().collect();
    let utf16_at = |byte: usize| text[..byte].encode_utf16().count();
    let mut length = match word.find(text) {
        Ok(Some(found)) if found.start() == 0 => utf16_at(found.end()),
        Ok(Some(found)) => utf16_at(found.start()),
        _ => units.len(),
    };
    // A whitespace run before that ends the word instead.
    if let Ok(whitespace) = regex::Regex::new(r"\s+")
        && let Some(found) = whitespace.find(text)
    {
        let end = utf16_at(found.end());
        if end < length {
            length = end;
        }
    }
    length.max(1).min(units.len())
}

impl CodeEditor {
    fn ghost_mode(mode: InlineSuggestMode) -> GhostTextMode {
        match mode {
            InlineSuggestMode::Prefix => GhostTextMode::Prefix,
            InlineSuggestMode::Subword => GhostTextMode::Subword,
            InlineSuggestMode::SubwordSmart => GhostTextMode::SubwordSmart,
        }
    }

    /// Monaco's `trigger`: typing, pasting, Tab, deleting and accepting turn
    /// inline completions on (until stopped) and ask for them.
    pub(crate) fn trigger_inline_completions(&mut self, cx: &mut Context<Self>) {
        if !self.options.inline_suggest.enabled || self.options.read_only {
            return;
        }
        self.inline.active = true;
        self.fetch_inline_completions(InlineCompletionTriggerKind::Automatic, cx);
    }

    /// Asks for inline completions now (Monaco
    /// `editor.action.inlineSuggest.trigger`, Alt+\).
    pub fn trigger_inline_suggest(&mut self, cx: &mut Context<Self>) {
        if self.options.read_only {
            return;
        }
        self.inline.active = true;
        // Monaco `trigger({ explicit: true })` starts an accept flow.
        self.inline.in_accept_flow = true;
        self.fetch_inline_completions(InlineCompletionTriggerKind::Explicit, cx);
    }

    /// The text changed: inline edits follow it, and while active it
    /// requests again.
    pub(crate) fn inline_completions_after_edit(&mut self, cx: &mut Context<Self>) {
        self.inline.change_count += 1;
        // Monaco resets `_inAcceptFlow` on every change; accepting sets it
        // again after its edit.
        self.inline.in_accept_flow = false;
        self.update_inline_edit_items(cx);
        if self.inline.active && !self.inline.updating {
            self.fetch_inline_completions(InlineCompletionTriggerKind::Automatic, cx);
        }
    }

    /// Monaco `InlineEditItem.withEdit`: moves each inline edit through the
    /// changes since it was last updated, dropping it when a change
    /// conflicts, when it was ignored for 20 changes, or when nothing is
    /// left to change.
    fn update_inline_edit_items(&mut self, cx: &mut Context<Self>) {
        let buffer = self.model().read(cx).buffer();
        let version = buffer.version();
        let rope = buffer.rope().clone();
        let changes: Vec<Option<Changes>> = self
            .inline
            .items
            .iter()
            .map(|item| {
                let edit = item.edit.as_ref()?;
                (edit.version != version).then(|| {
                    buffer
                        .edits_since(edit.version)
                        .edits()
                        .iter()
                        .map(|e| (e.old.clone(), rope.text_for_range(e.new.clone())))
                        .collect()
                })
            })
            .collect();
        let count = self.inline.change_count;
        let mut dropped = Vec::new();
        for (ix, (item, changes)) in self.inline.items.iter_mut().zip(changes).enumerate() {
            let (Some(edit), Some(changes)) = (item.edit.as_mut(), changes) else {
                continue;
            };
            edit.version = version;
            for updated in &mut edit.edits {
                updated.apply_changes(&changes);
            }
            if edit.edits.iter().any(|updated| updated.edit.is_none()) {
                dropped.push(ix);
                continue;
            }
            if edit
                .edits
                .iter()
                .any(|updated| updated.last_change_updated_edit)
            {
                edit.updated_at = count;
            }
            if edit.updated_at + INLINE_EDIT_MAX_IGNORED_CHANGES < count {
                dropped.push(ix);
                continue;
            }
            edit.edits.retain(|updated| {
                updated
                    .edit
                    .as_ref()
                    .is_some_and(|(range, text)| !range.is_empty() || !text.is_empty())
            });
            if edit.edits.is_empty() {
                // The user typed it.
                dropped.push(ix);
            }
        }
        for ix in dropped.into_iter().rev() {
            let item = self.inline.items.remove(ix);
            item.provider.handle_end_of_lifetime(
                &item.completion,
                &InlineCompletionEndOfLifeReason::Ignored {
                    superseded_by: None,
                    user_typing_disagreed: false,
                },
            );
        }
    }

    /// Monaco `InlineCompletionsSource.fetch`: debounced for automatic
    /// requests; an equal request already made satisfies it.
    fn fetch_inline_completions(
        &mut self,
        kind: InlineCompletionTriggerKind,
        cx: &mut Context<Self>,
    ) {
        let model = self.model().read(cx);
        let version = model.buffer().version();
        let position = self.cursors.primary().position;
        if let Some((v, p, k)) = &self.inline.requested
            && *v == version
            && *p == position
            && (kind == InlineCompletionTriggerKind::Automatic
                || *k == InlineCompletionTriggerKind::Explicit)
        {
            return;
        }
        let providers = inline_completions_providers(cx, self.provider_target(cx));
        if providers.is_empty() {
            return;
        }
        let updating = self.inline.task.is_some();
        self.inline.requested = Some((version, position, kind));
        self.inline.explicit = kind == InlineCompletionTriggerKind::Explicit;
        let debounce = providers
            .iter()
            .filter_map(|p| p.debounce_delay())
            .max()
            .unwrap_or(DEFAULT_DEBOUNCE);
        let delay = if updating || kind == InlineCompletionTriggerKind::Automatic {
            debounce
        } else {
            Duration::ZERO
        };
        let min_show = if kind == InlineCompletionTriggerKind::Explicit {
            Duration::ZERO
        } else {
            self.options.inline_suggest.min_show_delay
        };
        let started = Instant::now();
        let include_inline_edits = self.options.inline_suggest.edits.enabled;
        self.inline.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            // Stale if the text or cursor moved meanwhile.
            let stale = |this: &CodeEditor, cx: &gpui::App| {
                let model = this.model().read(cx);
                model.buffer().version() != version || this.cursors.primary().position != position
            };
            let query = move |this: &mut CodeEditor,
                              provider: &Arc<dyn InlineCompletionsProvider>,
                              cx: &mut Context<CodeEditor>| {
                let context = this.model().read(cx).provider_context();
                provider.provide_inline_completions(
                    &context,
                    position,
                    InlineCompletionContext {
                        trigger_kind: kind,
                        selected_suggestion_info: None,
                        include_inline_edits,
                        include_inline_completions: true,
                    },
                    cx,
                )
            };
            // Monaco: providers that yield to no one are asked at once; the
            // others after the groups they yield to, unless those have a
            // visible completion.
            let graph = yield_graph(&providers);
            let Ok(mut tasks) = this.update(cx, |this, cx| {
                if stale(this, cx) {
                    return None;
                }
                Some(
                    providers
                        .iter()
                        .enumerate()
                        .map(|(ix, provider)| {
                            graph[ix].is_empty().then(|| query(this, provider, cx))
                        })
                        .collect::<Vec<_>>(),
                )
            }) else {
                return;
            };
            let Some(tasks) = tasks.as_mut() else {
                this.update(cx, |this, _| this.inline.task = None).ok();
                return;
            };
            let mut results: Vec<Option<InlineCompletionList>> = vec![None; providers.len()];
            for ix in yield_order(&graph) {
                let task = match tasks[ix].take() {
                    Some(task) => task,
                    None => {
                        let yields = graph[ix].iter().filter_map(|&j| results[j].as_ref());
                        let lists: Vec<InlineCompletionList> = yields.cloned().collect();
                        let Ok(Some(task)) = this.update(cx, |this, cx| {
                            if stale(this, cx) || this.yielded_list_visible(&lists, position, cx) {
                                return None;
                            }
                            Some(query(this, &providers[ix], cx))
                        }) else {
                            continue;
                        };
                        task
                    }
                };
                match task.await {
                    Ok(list) => results[ix] = Some(list),
                    Err(err) => log::warn!("inline completions failed: {err:#}"),
                }
            }
            let lists: Vec<_> = providers
                .iter()
                .cloned()
                .zip(results)
                .filter_map(|(provider, list)| Some((provider, list?)))
                .collect();
            // Monaco `minShowDelay`.
            if let Some(wait) = min_show.checked_sub(started.elapsed())
                && !wait.is_zero()
            {
                cx.background_executor().timer(wait).await;
            }
            this.update(cx, |this, cx| {
                this.inline.task = None;
                if this.model().read(cx).buffer().version() != version {
                    return;
                }
                this.set_inline_completions(lists, version, position, cx);
            })
            .ok();
        }));
    }

    fn set_inline_completions(
        &mut self,
        lists: Vec<(Arc<dyn InlineCompletionsProvider>, InlineCompletionList)>,
        version: text::Version,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        let rope = self.model().read(cx).buffer().rope().clone();
        let line = rope.line_len_utf16(position.row);
        let word_start = self.word_start_at(position);
        self.inline.suppress_suggestions = false;
        self.inline.commands.clear();
        let include_inline_edits = self.options.inline_suggest.edits.enabled;
        let offset = |point: PointUtf16| {
            rope.point_utf16_to_offset(rope.clip_point_utf16(point, text::Bias::Left))
        };
        let mut items = Vec::new();
        let mut not_shown = Vec::new();
        for (provider, list) in lists {
            if let Some(suppress) = list.suppress_suggestions {
                self.inline.suppress_suggestions |= suppress;
            }
            self.inline.commands.extend(list.commands.iter().cloned());
            for completion in list.items {
                // Monaco `notInlineEditRequested`.
                if completion.is_inline_edit && !include_inline_edits {
                    not_shown.push((provider.clone(), completion));
                    continue;
                }
                // Monaco's default range: the word at the cursor to the end
                // of the line.
                let range = completion.range.clone().unwrap_or_else(|| {
                    PointUtf16::new(position.row, word_start)..PointUtf16::new(position.row, line)
                });
                let start = offset(range.start);
                let end = offset(range.end).max(start);
                let completion = InlineCompletion {
                    insert_text: completion
                        .insert_text
                        .replace("\r\n", "\n")
                        .replace('\r', "\n"),
                    ..completion
                };
                // Monaco `InlineEditItem.create`: the replacements a diff
                // finds.
                let edit = completion.is_inline_edit.then(|| InlineEditItem {
                    edits: diffed_edits(&rope, start..end, &completion.insert_text)
                        .into_iter()
                        .map(|(range, text)| {
                            let replaced = rope.text_for_range(range.clone());
                            UpdatedEdit::new(range, text, &replaced)
                        })
                        .collect(),
                    version,
                    updated_at: self.inline.change_count,
                });
                if edit.as_ref().is_some_and(|edit| edit.edits.is_empty()) {
                    // It changes nothing.
                    not_shown.push((provider.clone(), completion));
                    continue;
                }
                let show_range = completion
                    .show_range
                    .as_ref()
                    .map(|range| offset(range.start)..offset(range.end).max(offset(range.start)));
                self.inline.next_id += 1;
                items.push(Item {
                    provider: provider.clone(),
                    completion,
                    range: start..end,
                    version,
                    original_start: range.start,
                    shown: false,
                    id: self.inline.next_id,
                    commands: list.commands.clone(),
                    show_range,
                    edit,
                });
            }
        }
        // Monaco `createStateWithAppliedResults`: completions win when one
        // is visible at the cursor, else only inline edits are kept.
        let line_text = |row: u32| rope.line(row);
        let prefer_completions = items.iter().any(|item| {
            item.edit.is_none()
                && self.item_shows_at(item, position, cx)
                && self.current_edit(item, cx).is_some_and(|edit| {
                    is_visible(&edit, item.original_start, position, &line_text)
                })
        });
        let (kept, dropped): (Vec<Item>, Vec<Item>) = items
            .into_iter()
            .partition(|item| prefer_completions != item.edit.is_some());
        not_shown.extend(
            dropped
                .into_iter()
                .map(|item| (item.provider, item.completion)),
        );
        for (provider, completion) in not_shown {
            provider.handle_end_of_lifetime(
                &completion,
                &InlineCompletionEndOfLifeReason::Ignored {
                    superseded_by: None,
                    user_typing_disagreed: false,
                },
            );
        }
        // Monaco: replaced items end, superseded by an equal new one, which
        // keeps their identity (and so whether they showed).
        for old in std::mem::replace(&mut self.inline.items, kept) {
            let superseded_by = self
                .inline
                .items
                .iter_mut()
                .find(|new| {
                    new.completion.insert_text == old.completion.insert_text
                        && new.original_start == old.original_start
                        && new.completion.is_inline_edit == old.completion.is_inline_edit
                })
                .map(|new| {
                    new.shown |= old.shown;
                    new.id = old.id;
                    new.completion.clone()
                });
            old.provider.handle_end_of_lifetime(
                &old.completion,
                &InlineCompletionEndOfLifeReason::Ignored {
                    superseded_by,
                    user_typing_disagreed: false,
                },
            );
        }
        cx.notify();
    }

    /// Whether a list from a provider yielded to has a completion visible
    /// at `position`, or one that isn't plain text (Monaco blocks then).
    fn yielded_list_visible(
        &self,
        lists: &[InlineCompletionList],
        position: PointUtf16,
        cx: &gpui::App,
    ) -> bool {
        let rope = self.model().read(cx).buffer().rope().clone();
        let line = rope.line_len_utf16(position.row);
        let word_start = self.word_start_at(position);
        let line_text = |row: u32| self.snapshot.line(row);
        lists.iter().flat_map(|list| &list.items).any(|item| {
            if item.is_snippet || item.is_inline_edit {
                return true;
            }
            let range = item.range.clone().unwrap_or_else(|| {
                PointUtf16::new(position.row, word_start)..PointUtf16::new(position.row, line)
            });
            let edit = TextReplacement {
                range: range.clone(),
                text: item.insert_text.clone(),
            };
            is_visible(&edit, range.start, position, &line_text)
        })
    }

    /// An item's current range and edit.
    fn current_edit(&self, item: &Item, cx: &gpui::App) -> Option<TextReplacement> {
        let buffer = self.model().read(cx).buffer();
        let range = if buffer.version() == item.version {
            item.range.clone()
        } else {
            let patch = buffer.edits_since(item.version);
            let start = patch.transform_offset(item.range.start, text::Bias::Left);
            let end = patch.transform_offset(item.range.end, text::Bias::Right);
            start..end.max(start)
        };
        let rope = buffer.rope();
        Some(TextReplacement {
            range: rope.offset_to_point_utf16(range.start.min(rope.len()))
                ..rope.offset_to_point_utf16(range.end.min(rope.len())),
            text: item.completion.insert_text.clone(),
        })
    }

    /// Whether `position` is in the item's `show_range` (Monaco
    /// `showRange`), which follows edits.
    fn item_shows_at(&self, item: &Item, position: PointUtf16, cx: &gpui::App) -> bool {
        let Some(range) = &item.show_range else {
            return true;
        };
        let buffer = self.model().read(cx).buffer();
        let range = if buffer.version() == item.version {
            range.clone()
        } else {
            let patch = buffer.edits_since(item.version);
            patch.transform_offset(range.start, text::Bias::Left)
                ..patch.transform_offset(range.end, text::Bias::Right)
        };
        let rope = buffer.rope();
        let start = rope.offset_to_point_utf16(range.start.min(rope.len()));
        let end = rope.offset_to_point_utf16(range.end.min(rope.len()));
        start <= position && position <= end
    }

    /// Whether a classic completion is visible at the cursor (Monaco
    /// `isVisible`, which keeps inline edits hidden).
    fn completion_visible(&self, cx: &gpui::App) -> bool {
        let cursor = self.cursors.primary().position;
        let rope = self.model().read(cx).buffer().rope().clone();
        let line_text = |row: u32| rope.line(row);
        self.inline.items.iter().any(|item| {
            item.edit.is_none()
                && self.item_shows_at(item, cursor, cx)
                && self
                    .current_edit(item, cx)
                    .is_some_and(|edit| is_visible(&edit, item.original_start, cursor, &line_text))
        })
    }

    /// The shown inline edit: its item and its replacements (Monaco
    /// `inlineEditState`). It's the last inline edit, shown unless a
    /// completion is visible at the cursor, a peek view is open, or a
    /// snippet is active (with `suppress_in_snippet_mode`).
    pub(crate) fn inline_edit_state(&self, cx: &gpui::App) -> Option<(usize, Replacements)> {
        if !self.options.inline_suggest.enabled || self.is_peek_visible() {
            return None;
        }
        if self.options.inline_suggest.suppress_in_snippet_mode && self.in_snippet_mode() {
            return None;
        }
        let cursor = self.cursors.primary().position;
        let (ix, item) = self
            .inline
            .items
            .iter()
            .enumerate()
            .rev()
            .find(|(_, item)| item.edit.is_some())?;
        if !self.item_shows_at(item, cursor, cx) || self.completion_visible(cx) {
            return None;
        }
        let edit = item.edit.as_ref()?;
        let buffer = self.model().read(cx).buffer();
        if edit.version != buffer.version() {
            return None;
        }
        let rope = buffer.rope();
        let edits = edit
            .edits
            .iter()
            .filter_map(|updated| updated.edit.as_ref())
            .map(|(range, text)| {
                (
                    rope.offset_to_point_utf16(range.start.min(rope.len()))
                        ..rope.offset_to_point_utf16(range.end.min(rope.len())),
                    text.clone(),
                )
            })
            .collect();
        Some((ix, edits))
    }

    /// The range from the inline edit's first change to its last (Monaco
    /// `targetRange`).
    fn inline_edit_target(edits: &[(Range<PointUtf16>, String)]) -> Option<Range<PointUtf16>> {
        Some(edits.first()?.0.start..edits.last()?.0.end)
    }

    /// The shown inline edit's replacements as (replaced text, new text),
    /// for tests.
    #[doc(hidden)]
    pub fn inline_edit_for_tests(&self, cx: &gpui::App) -> Option<Vec<(String, String)>> {
        let (_, edits) = self.inline_edit_state(cx)?;
        let rope = self.model().read(cx).buffer().rope().clone();
        Some(
            edits
                .into_iter()
                .map(|(range, text)| {
                    let start = rope.point_utf16_to_offset(range.start);
                    let end = rope.point_utf16_to_offset(range.end);
                    (rope.text_for_range(start..end), text)
                })
                .collect(),
        )
    }

    /// Whether an inline edit shows (Monaco `inlineEditIsVisible`).
    pub fn inline_edit_visible(&self, cx: &gpui::App) -> bool {
        self.inline_edit_state(cx).is_some()
    }

    /// Whether the cursor is on the inline edit's lines or a line next to
    /// them (Monaco `cursorAtInlineEdit`).
    pub fn cursor_at_inline_edit(&self, cx: &gpui::App) -> bool {
        let Some(target) = self
            .inline_edit_state(cx)
            .and_then(|(_, edits)| Self::inline_edit_target(&edits))
        else {
            return false;
        };
        let row = self.cursors.primary().position.row;
        target.start.row.saturating_sub(1) <= row && row <= target.end.row + 1
    }

    /// Monaco `_tabShouldIndent`: Tab indents an empty line or a selection
    /// covering a line's text.
    fn tab_should_indent(&self, cx: &gpui::App) -> bool {
        if self.inline.in_accept_flow {
            return false;
        }
        let rope = self.model().read(cx).buffer().rope().clone();
        self.cursors.selections().iter().any(|selection| {
            let (start, end) = if selection.anchor <= selection.head {
                (selection.anchor, selection.head)
            } else {
                (selection.head, selection.anchor)
            };
            if start == end {
                return rope.line_len_utf16(start.row) == 0;
            }
            if start.row != end.row {
                return true;
            }
            // Covers the line's text, without its indentation.
            let line = rope.line(start.row);
            let indentation = line.len() - line.trim_start().len();
            let text_start = line[..indentation].encode_utf16().count() as u32;
            let text_end = line.trim_end().encode_utf16().count() as u32;
            start.column <= text_start && end.column >= text_end.max(text_start)
        })
    }

    /// Monaco `showCollapsed`: the inline edit shows as a marker only, with
    /// `inline_suggest.edits.show_collapsed` or once other changes moved
    /// it, until it's jumped to or while accepting.
    pub(crate) fn inline_edit_show_collapsed(&self, cx: &gpui::App) -> bool {
        let Some((ix, _)) = self.inline_edit_state(cx) else {
            return false;
        };
        let item = &self.inline.items[ix];
        let current = item
            .edit
            .as_ref()
            .is_some_and(|edit| edit.updated_at == self.inline.change_count);
        (self.options.inline_suggest.edits.show_collapsed || !current)
            && self.inline.jumped_to != Some(item.id)
            && !self.inline.in_accept_flow
    }

    /// Whether the inline edit's lines are all on screen (Monaco
    /// `isSuggestionInViewport`).
    fn inline_edit_in_viewport(&self, target: &Range<PointUtf16>, cx: &gpui::App) -> bool {
        let visible = self.visible_ranges(cx);
        match (visible.first(), visible.last()) {
            (Some(first), Some(last)) => first.start <= target.start && target.end <= last.end,
            _ => false,
        }
    }

    /// Whether Tab jumps to the inline edit (Monaco
    /// `tabShouldJumpToInlineEdit`): the cursor isn't at it, or it shows
    /// collapsed.
    pub fn tab_should_jump_to_inline_edit(&self, cx: &gpui::App) -> bool {
        if self.tab_should_indent(cx) {
            return false;
        }
        let Some(target) = self
            .inline_edit_state(cx)
            .and_then(|(_, edits)| Self::inline_edit_target(&edits))
        else {
            return false;
        };
        if self.inline_edit_show_collapsed(cx) {
            return true;
        }
        if self.inline.in_accept_flow && self.inline_edit_in_viewport(&target, cx) {
            return false;
        }
        !self.cursor_at_inline_edit(cx)
    }

    /// Whether Tab accepts the inline edit (Monaco
    /// `tabShouldAcceptInlineEdit`): the cursor is at it, it was jumped to,
    /// or it follows an accepted one on screen.
    pub fn tab_should_accept_inline_edit(&self, cx: &gpui::App) -> bool {
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            return false;
        };
        let Some(target) = Self::inline_edit_target(&edits) else {
            return false;
        };
        if self.inline_edit_show_collapsed(cx) || self.tab_should_indent(cx) {
            return false;
        }
        if self.inline.in_accept_flow && self.inline_edit_in_viewport(&target, cx) {
            return true;
        }
        if target.start.row == self.cursors.primary().position.row
            || self.inline.jumped_to == Some(self.inline.items[ix].id)
        {
            return true;
        }
        self.cursor_at_inline_edit(cx)
    }

    /// Jumps to the inline edit (Monaco `editor.action.inlineSuggest.jump`,
    /// Tab while the cursor is away from it): moves the cursor to its start
    /// and reveals it. Tab then accepts it.
    pub fn jump_to_next_inline_edit(&mut self, cx: &mut Context<Self>) {
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            return;
        };
        let Some(target) = Self::inline_edit_target(&edits) else {
            return;
        };
        self.inline.jumped_to = Some(self.inline.items[ix].id);
        self.inline.updating = true;
        self.set_selections(&[Selection::collapsed(target.start)], cx);
        self.inline.updating = false;
        let single_line = target.start.row == target.end.row
            && !edits.iter().any(|(_, text)| text.contains('\n'));
        if single_line {
            self.reveal_position(target.start, cx);
        } else {
            let max_row = self.model().read(cx).buffer().rope().max_point().row;
            self.reveal_range(
                PointUtf16::new(target.start.row.saturating_sub(1), 0)
                    ..PointUtf16::new((target.end.row + 1).min(max_row), 0),
                crate::RevealType::Simple,
                cx,
            );
        }
        cx.notify();
    }

    /// Switches `inline_suggest.edits.show_collapsed` (Monaco
    /// `editor.action.inlineSuggest.toggleShowCollapsed`).
    pub fn toggle_inline_edits_show_collapsed(&mut self, cx: &mut Context<Self>) {
        let mut options = self.options.clone();
        options.inline_suggest.edits.show_collapsed = !options.inline_suggest.edits.show_collapsed;
        self.update_options(options, cx);
    }

    /// Monaco `handleInlineEditShownNextFrame`: reports the inline edit as
    /// shown, once.
    pub(crate) fn inline_edit_did_show(&mut self, ix: usize) {
        let item = &mut self.inline.items[ix];
        item.shown = true;
        if self.inline.shown.insert(format!("edit:{}", item.id)) {
            item.provider.handle_item_did_show(&item.completion);
        }
    }

    /// Accepts the inline edit (Monaco `accept` for an `inlineEdit` state):
    /// applies its replacements and the item's other edits, and puts the
    /// cursor after the last replacement.
    fn accept_inline_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            return false;
        };
        let Some(((first_range, first_text), rest)) = edits.split_first() else {
            return false;
        };
        let item = self.inline.items[ix].clone();
        self.inline.updating = true;
        self.stop_inline_completions(StopReason::Accepted(ix), cx);
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        let cursor = new_ranges(&edits).last().map(|range| range.end);
        let mut others = rest.to_vec();
        others.extend(item.completion.additional_text_edits.iter().cloned());
        self.replace_text(first_range.clone(), first_text, &others, cx);
        if let Some(cursor) = cursor {
            self.set_selections(&[Selection::collapsed(cursor)], cx);
        }
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        self.inline.updating = false;
        // Set after the edit, which resets it.
        self.inline.in_accept_flow = true;
        if let Some(command) = &item.completion.command
            && let Err(err) = crate::commands::execute_command(cx, command)
        {
            log::warn!("{err:#}");
        }
        // Accepting is a trigger.
        self.trigger_inline_completions(cx);
        true
    }

    /// The visible items (deduplicated by their edit), the selected index,
    /// and its ghost text.
    fn inline_state(&self, cx: &gpui::App) -> Option<InlineView> {
        if !self.inline.active
            || !self.options.inline_suggest.enabled
            || self.marked_range.is_some()
        {
            return None;
        }
        if self.options.inline_suggest.suppress_in_snippet_mode && self.in_snippet_mode() {
            return None;
        }
        if self.is_suggest_visible() {
            return None;
        }
        let cursor = self.cursors.primary().position;
        let line_text = |row: u32| self.snapshot.line(row);
        let mut seen = HashSet::new();
        let visible: Vec<(usize, TextReplacement)> = self
            .inline
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.edit.is_none() && self.item_shows_at(item, cursor, cx))
            .filter_map(|(ix, item)| {
                let edit = self.current_edit(item, cx)?;
                is_visible(&edit, item.original_start, cursor, &line_text).then_some((ix, edit))
            })
            .filter(|(_, edit)| seen.insert((edit.text.clone(), edit.range.start)))
            .collect();
        if visible.is_empty() {
            return None;
        }
        let selected = self
            .inline
            .selected
            .as_ref()
            .and_then(|(text, range)| {
                visible
                    .iter()
                    .position(|(_, e)| e.text == *text && e.range.start == range.start)
            })
            .unwrap_or(0);
        let edit = &visible[selected].1;
        let ghost = compute_ghost_text(
            edit,
            &line_text,
            Self::ghost_mode(self.options.inline_suggest.mode),
            Some(cursor),
            0,
        )?;
        if ghost.parts.iter().all(|p| p.text().is_empty()) {
            return None;
        }
        Some((visible, selected, ghost))
    }

    /// Monaco's suggest preview (`suggest.preview`): the suggest widget's
    /// selected item as ghost text, not italic.
    fn suggest_preview(&self) -> Option<GhostText> {
        if !self.options.suggest.preview || !self.is_suggest_visible() {
            return None;
        }
        let (range, text) = self.selected_suggestion_edit()?;
        let line_text = |row: u32| self.snapshot.line(row);
        let length = text.encode_utf16().count();
        compute_ghost_text(
            &TextReplacement { range, text },
            &line_text,
            Self::ghost_mode(self.options.suggest.preview_mode),
            Some(self.cursors.primary().position),
            length,
        )
    }

    /// Whether ghost text shows (Monaco `inlineSuggestionVisible`).
    pub fn inline_suggestion_visible(&self, cx: &gpui::App) -> bool {
        self.inline_state(cx).is_some()
    }

    /// The ghost text shown, as (row, text with lines joined by `\n`), for
    /// tests.
    #[doc(hidden)]
    pub fn ghost_text_for_tests(&self, cx: &gpui::App) -> Option<(u32, Vec<(u32, String)>)> {
        let ghost = self
            .inline_state(cx)
            .map(|(_, _, ghost)| ghost)
            .or_else(|| self.suggest_preview())?;
        Some((
            ghost.line,
            ghost.parts.iter().map(|p| (p.column, p.text())).collect(),
        ))
    }

    /// The ghost text's row and its text for screen readers (Monaco
    /// `renderForScreenReader`).
    pub(crate) fn ghost_text_for_screen_reader(&self, cx: &gpui::App) -> Option<(u32, String)> {
        let (_, _, ghost) = self.inline_state(cx)?;
        let line = self.snapshot.line(ghost.line);
        Some((ghost.line, ghost.render_for_screen_reader(&line)))
    }

    /// Whether ghost text is inserted at `position` (Monaco
    /// `shouldShowHoverAt`).
    pub(crate) fn inline_ghost_text_at(&self, position: PointUtf16, cx: &gpui::App) -> bool {
        self.inline_state(cx).is_some_and(|(_, _, ghost)| {
            ghost.line == position.row && ghost.parts.iter().any(|p| p.column == position.column)
        })
    }

    /// Whether the visible list asks the suggest widget to stay closed
    /// (Monaco `inlineSuggestionSuppressSuggestions`).
    pub(crate) fn inline_suppresses_suggestions(&self, cx: &gpui::App) -> bool {
        (self.inline.suppress_suggestions || self.options.inline_suggest.suppress_suggestions)
            && self.inline_suggestion_visible(cx)
    }

    /// Monaco `syntaxHighlightingEnabled`: each part's first line and the
    /// extra lines, split into pieces colored as code (at 70% opacity), or
    /// one plain ghost-colored piece each. Monaco tokenizes the line with
    /// the ghost text inserted from the line's state; this parses that line
    /// and the extra lines on their own.
    fn ghost_segments(
        &self,
        ghost: &GhostText,
        style: impl Fn(bool) -> TokenStyle,
        cx: &gpui::App,
    ) -> (Vec<GhostSegments>, Vec<GhostSegments>) {
        let plain = |text: &str, preview: bool| vec![(text.to_string(), style(preview))];
        let extra = || {
            ghost
                .parts
                .iter()
                .flat_map(|part| part.lines.iter().skip(1).map(|l| plain(l, false)))
                .collect::<Vec<_>>()
        };
        let firsts = || {
            ghost
                .parts
                .iter()
                .map(|part| plain(&part.lines[0], part.preview))
                .collect()
        };
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let spans = self
            .options
            .inline_suggest
            .syntax_highlighting_enabled
            .then_some(language)
            .flatten()
            .and_then(|language| {
                // The line with the parts inserted, then the extra lines.
                let line = self.snapshot.line(ghost.line);
                let mut text = String::new();
                let mut starts = Vec::new();
                let mut taken = 0;
                for part in &ghost.parts {
                    let at = line
                        .char_indices()
                        .scan(0u32, |units, (ix, ch)| {
                            let here = *units;
                            *units += ch.len_utf16() as u32;
                            Some((ix, here))
                        })
                        .find(|(_, units)| *units >= part.column)
                        .map_or(line.len(), |(ix, _)| ix)
                        .max(taken);
                    text.push_str(&line[taken..at]);
                    taken = at;
                    starts.push(text.len());
                    text.push_str(&part.lines[0]);
                }
                text.push_str(&line[taken..]);
                let mut extra_starts = Vec::new();
                for part in &ghost.parts {
                    for extra in part.lines.iter().skip(1) {
                        text.push('\n');
                        extra_starts.push(text.len());
                        text.push_str(extra);
                    }
                }
                let registry = crate::model::language_registry(cx);
                let spans = crate::colorize::capture_spans(&text, &language, &registry)?;
                Some((spans, starts, extra_starts))
            });
        let Some((spans, starts, extra_starts)) = spans else {
            return (firsts(), extra());
        };
        let colored = |text: &str, start: usize, preview: bool| -> GhostSegments {
            split_by_spans(text, start, &spans)
                .into_iter()
                .map(|(piece, id)| {
                    let token = id.map(|id| self.theme.style_for(id)).unwrap_or_default();
                    (
                        piece,
                        TokenStyle {
                            italic: !preview,
                            opacity: Some(0.7),
                            ..token
                        },
                    )
                })
                .collect()
        };
        let firsts = ghost
            .parts
            .iter()
            .zip(&starts)
            .map(|(part, &start)| colored(&part.lines[0], start, part.preview))
            .collect();
        let extra = ghost
            .parts
            .iter()
            .flat_map(|part| part.lines.iter().skip(1))
            .zip(&extra_starts)
            .map(|(line, &start)| colored(line, start, false))
            .collect();
        (firsts, extra)
    }

    /// Renders the ghost text before painting.
    pub(crate) fn refresh_ghost_text(&mut self, cx: &mut Context<Self>) {
        // The jump resets once the cursor leaves the edit (Monaco
        // `jumpToReset`).
        if self.inline.jumped_to.is_some() && !self.cursor_at_inline_edit(cx) {
            self.inline.jumped_to = None;
        }
        self.refresh_inline_edit(cx);
        let state = self.inline_state(cx);
        self.refresh_inline_toolbar(state.as_ref().map(|(_, _, ghost)| ghost), cx);
        let preview = if state.is_none() {
            self.suggest_preview()
        } else {
            None
        };
        let version = self.model().read(cx).buffer().version();
        let cursor = self.cursors.primary().position;
        let multiline = |ghost: &GhostText| ghost.parts.iter().any(|p| p.lines.len() > 1);
        let key = match (&state, &preview) {
            (Some((visible, selected, ghost)), _) => {
                Some((version, cursor, visible[*selected].0, multiline(ghost)))
            }
            (None, Some(ghost)) => Some((version, cursor, usize::MAX, multiline(ghost))),
            _ => None,
        };
        if key.is_some() && self.inline.rendered.as_ref().map(|r| &r.key) == key.as_ref() {
            return;
        }
        self.clear_ghost_text(cx);
        let Some(key) = key else {
            return;
        };
        let (shown, ghost) = match (state, preview) {
            (Some((visible, selected, ghost)), _) => (Some((visible, selected)), ghost),
            (None, Some(ghost)) => (None, ghost),
            _ => return,
        };
        let colors = &self.theme.colors;
        let visible_color = |c: gpui::Hsla| (c.a > 0.).then_some(c);
        let style = |preview: bool| TokenStyle {
            foreground: Some(colors.ghost_text_foreground),
            italic: !preview,
            ..Default::default()
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let (firsts, extra_lines) = self.ghost_segments(&ghost, style, cx);
        let colors = &self.theme.colors;
        let font_family = self.options.inline_suggest.font_family.clone();
        let mut decorations = Vec::new();
        for (part, segments) in ghost.parts.iter().zip(firsts) {
            let offset = rope.point_utf16_to_offset(PointUtf16::new(ghost.line, part.column));
            // One injected text per piece, in order.
            for (text, style) in segments {
                decorations.push((
                    offset..offset,
                    DecorationOptions {
                        after: Some(InjectedText {
                            content: text.into(),
                            style,
                            background: visible_color(colors.ghost_text_background),
                            swatch: None,
                            font_family: font_family.clone(),
                            ..Default::default()
                        }),
                        border: visible_color(colors.ghost_text_border),
                        ..Default::default()
                    },
                ));
            }
        }
        let owner = Some(cx.entity_id());
        let ids = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &[], decorations, cx)
        });
        let zone = (!extra_lines.is_empty()).then(|| {
            let foreground = self.theme.colors.foreground;
            let view = cx.new(|_| GhostLines {
                lines: extra_lines.clone(),
                foreground,
                font: font_family
                    .clone()
                    .unwrap_or_else(|| self.options.font_family.clone()),
                font_size: self.options.font_size,
                line_height: self.options.resolved_line_height(),
                left: px(0.),
            });
            self.add_view_zone(
                ViewZone {
                    after_line: ghost.line + 1,
                    height: ZoneHeight::Lines(extra_lines.len() as f32),
                    view: view.into(),
                },
                cx,
            )
        });
        // Monaco `handleItemDidShow`, once per item.
        if let Some((visible, selected)) = shown {
            self.inline.items[visible[selected].0].shown = true;
            let item = &self.inline.items[visible[selected].0];
            let identity = format!(
                "{:?}{}",
                visible[selected].1.range, item.completion.insert_text
            );
            if self.inline.shown.insert(identity) {
                item.provider.handle_item_did_show(&item.completion);
            }
        }
        self.inline.rendered = Some(Rendered {
            key,
            decorations: ids,
            zone,
        });
    }

    /// Monaco `InlineCompletionsHintsWidget`: with `show_toolbar: Always`,
    /// the toolbar sits above (else below) the ghost text's start, staying
    /// left of where it first showed on that line, and asks explicitly for
    /// all completions so it can count them.
    fn refresh_inline_toolbar(&mut self, ghost: Option<&GhostText>, cx: &mut Context<Self>) {
        use crate::options::InlineSuggestShowToolbar;
        use crate::widgets::{ContentWidget, ContentWidgetPreference};
        let always = self.options.inline_suggest.show_toolbar == InlineSuggestShowToolbar::Always;
        let position = ghost
            .filter(|ghost| always && !ghost.parts.is_empty())
            .map(|ghost| {
                let column = ghost.parts[0].column;
                match self.inline.toolbar_session {
                    Some(session) if session.row == ghost.line => {
                        PointUtf16::new(ghost.line, column.min(session.column))
                    }
                    _ => PointUtf16::new(ghost.line, column),
                }
            });
        let previous = std::mem::replace(&mut self.inline.toolbar_session, position);
        let Some(position) = position else {
            if let Some((_, id)) = self.inline.toolbar.take() {
                self.remove_widget(id, cx);
            }
            return;
        };
        if previous == Some(position) && self.inline.toolbar.is_some() {
            return;
        }
        if previous.is_none() && !self.inline.explicit {
            self.trigger_inline_suggest(cx);
        }
        let view = match self.inline.toolbar.take() {
            Some((view, id)) => {
                self.remove_widget(id, cx);
                view
            }
            None => {
                let editor = cx.entity();
                let theme = self.theme.clone();
                cx.new(|cx| super::toolbar::InlineToolbar::new(&editor, theme, true, cx))
            }
        };
        let id = self.add_content_widget(
            ContentWidget {
                position,
                preference: vec![
                    ContentWidgetPreference::Above,
                    ContentWidgetPreference::Below,
                ],
                view: view.clone().into(),
            },
            cx,
        );
        self.inline.toolbar = Some((view, id));
    }

    /// Monaco `editor.action.inlineSuggest.toggleAlwaysShowToolbar`: switches
    /// `show_toolbar` between `Always` and `OnHover`.
    pub fn toggle_always_show_inline_toolbar(&mut self, cx: &mut Context<Self>) {
        use crate::options::InlineSuggestShowToolbar;
        let mut options = self.options.clone();
        options.inline_suggest.show_toolbar =
            if options.inline_suggest.show_toolbar == InlineSuggestShowToolbar::Always {
                InlineSuggestShowToolbar::OnHover
            } else {
                InlineSuggestShowToolbar::Always
            };
        self.update_options(options, cx);
    }

    /// The always-shown toolbar's anchor, for tests.
    #[doc(hidden)]
    pub fn inline_toolbar_position_for_tests(&self) -> Option<PointUtf16> {
        self.inline
            .toolbar
            .as_ref()
            .and(self.inline.toolbar_session)
    }

    fn clear_ghost_text(&mut self, cx: &mut Context<Self>) {
        let Some(rendered) = self.inline.rendered.take() else {
            return;
        };
        if !rendered.decorations.is_empty() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &rendered.decorations, Vec::new(), cx)
            });
        }
        if let Some(zone) = rendered.zone {
            self.remove_widget(zone, cx);
        }
    }

    /// Hides the ghost text or inline edit until typing triggers it again,
    /// rejecting the shown one (Monaco `stop('explicitCancel')`, Escape).
    pub fn hide_inline_suggestion(&mut self, cx: &mut Context<Self>) {
        self.stop_inline_completions(StopReason::Explicit, cx);
    }

    /// Monaco `stop`: ends every item's lifetime and hides the ghost text
    /// and inline edit.
    fn stop_inline_completions(&mut self, reason: StopReason, cx: &mut Context<Self>) {
        let shown = match reason {
            StopReason::Explicit => self
                .inline_state(cx)
                .map(|(visible, selected, _)| visible[selected].0)
                .or_else(|| self.inline_edit_state(cx).map(|(ix, _)| ix)),
            _ => None,
        };
        self.inline.jumped_to = None;
        for (ix, item) in std::mem::take(&mut self.inline.items)
            .into_iter()
            .enumerate()
        {
            let reason = if reason == StopReason::Accepted(ix) {
                InlineCompletionEndOfLifeReason::Accepted
            } else if shown == Some(ix) && item.shown {
                // Only a completion that showed can be rejected.
                item.provider.handle_rejection(&item.completion);
                InlineCompletionEndOfLifeReason::Rejected
            } else {
                InlineCompletionEndOfLifeReason::Ignored {
                    superseded_by: None,
                    user_typing_disagreed: false,
                }
            };
            item.provider
                .handle_end_of_lifetime(&item.completion, &reason);
        }
        self.inline.active = false;
        self.inline.selected = None;
        self.inline.task = None;
        self.inline.requested = None;
        self.clear_ghost_text(cx);
        self.clear_inline_edit_view(cx);
        cx.notify();
    }

    /// Selections moved: an explicit move (keys, mouse) or an API change
    /// hides the ghost text, but not an inline edit (Monaco's controller).
    pub(crate) fn inline_selections_changed(
        &mut self,
        change: &SelectionChange,
        cx: &mut Context<Self>,
    ) {
        if self.inline.updating {
            return;
        }
        if self.inline.jumped_to.is_some() && !self.cursor_at_inline_edit(cx) {
            self.inline.jumped_to = None;
        }
        // Monaco stops only ghost text: inline edits stay, even while the
        // cursor is outside their `show_range`.
        if !self.inline.active || self.inline.items.iter().any(|item| item.edit.is_some()) {
            return;
        }
        if change.reason == CursorChangeReason::Explicit || change.source == CursorChangeSource::Api
        {
            self.stop_inline_completions(StopReason::Automatic, cx);
        }
    }

    /// The editor lost focus: hides unless `keep_on_blur`, or an inline
    /// edit asked for explicitly shows (Monaco).
    pub(crate) fn inline_blurred(&mut self, cx: &mut Context<Self>) {
        if self.inline.explicit && self.inline_edit_visible(cx) {
            return;
        }
        if !self.options.inline_suggest.keep_on_blur && self.inline.active {
            self.stop_inline_completions(StopReason::Automatic, cx);
        }
    }

    /// Shows the next (or previous) completion (Monaco `showNext`, Alt+],
    /// and `showPrevious`, Alt+[), asking explicitly for all of them first.
    pub fn show_next_inline_suggestion(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.trigger_inline_suggest(cx);
        let Some((visible, selected, _)) = self.inline_state(cx) else {
            return;
        };
        let count = visible.len();
        let next = if forward {
            (selected + 1) % count
        } else {
            (selected + count - 1) % count
        };
        let edit = &visible[next].1;
        self.inline.selected = Some((edit.text.clone(), edit.range.clone()));
        cx.notify();
    }

    /// The shown completion's index and the count, once an explicit request
    /// asked for all of them (Monaco's "1/3").
    pub fn inline_suggestion_index(&self, cx: &gpui::App) -> Option<(usize, usize)> {
        let (visible, selected, _) = self.inline_state(cx)?;
        self.inline.explicit.then_some((selected, visible.len()))
    }

    /// Whether Tab accepts the ghost text: there's no selection, and it
    /// doesn't start with a tab's worth of indentation inside the line's
    /// indentation (Monaco `inlineSuggestionHasIndentationLessThanTabSize`).
    pub(crate) fn tab_accepts_inline_suggestion(&self, cx: &gpui::App) -> bool {
        let Some((_, _, ghost)) = self.inline_state(cx) else {
            return false;
        };
        if self.cursors.selections().iter().any(|s| s.anchor != s.head) {
            return false;
        }
        let Some(first) = ghost.parts.first() else {
            return false;
        };
        let line = self.snapshot.line(ghost.line);
        let indentation = line.len() - line.trim_start_matches([' ', '\t']).len();
        if (first.column as usize) > indentation {
            return true;
        }
        let tab_size = self.model().read(cx).options().tab_size as usize;
        let leading = &first.lines[0]
            [..first.lines[0].len() - first.lines[0].trim_start_matches([' ', '\t']).len()];
        let width: usize = leading
            .chars()
            .map(|c| if c == '\t' { tab_size } else { 1 })
            .sum();
        width < tab_size
    }

    /// Whether the cursor is right before the ghost text (Monaco
    /// `cursorBeforeGhostText`).
    pub(crate) fn cursor_before_ghost_text(&self, cx: &gpui::App) -> bool {
        let cursor = self.cursors.primary().position;
        self.inline_state(cx).is_some_and(|(_, _, ghost)| {
            ghost.line == cursor.row
                && ghost
                    .parts
                    .first()
                    .is_some_and(|p| p.column == cursor.column)
        })
    }

    /// Accepts the shown completion or inline edit (Monaco
    /// `editor.action.inlineSuggest.commit`, Tab).
    pub fn accept_inline_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        if self.inline_edit_visible(cx) {
            return self.accept_inline_edit(cx);
        }
        let Some((visible, selected, _)) = self.inline_state(cx) else {
            return false;
        };
        let (ix, edit) = visible[selected].clone();
        let item = self.inline.items[ix].clone();
        self.inline.updating = true;
        self.stop_inline_completions(StopReason::Accepted(ix), cx);
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        if item.completion.is_snippet {
            // Monaco: delete the range, apply the other edits, then insert
            // the snippet at its start.
            self.model()
                .update(cx, |model, _| model.start_transaction());
            self.replace_text(
                edit.range.clone(),
                "",
                &item.completion.additional_text_edits,
                cx,
            );
            self.set_selections(&[Selection::collapsed(edit.range.start)], cx);
            self.insert_snippet(
                &item.completion.insert_text,
                crate::snippet_session::SnippetInsertOptions {
                    undo_stop_before: false,
                    undo_stop_after: false,
                    ..Default::default()
                },
                cx,
            );
            self.model().update(cx, |model, _| {
                model.end_transaction();
            });
        } else {
            // The cursor goes after the inserted text, less the suffix it
            // shares with the replaced text.
            let replaced = self.text_range(edit.range.clone());
            let common_suffix = edit
                .text
                .chars()
                .rev()
                .zip(replaced.chars().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let kept: String = {
                let chars: Vec<char> = edit.text.chars().collect();
                chars[..chars.len() - common_suffix.min(chars.len())]
                    .iter()
                    .collect()
            };
            self.replace_text(
                edit.range.clone(),
                &edit.text,
                &item.completion.additional_text_edits,
                cx,
            );
            let cursor = end_after(edit.range.start, &kept);
            self.set_selections(&[Selection::collapsed(cursor)], cx);
        }
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        self.inline.updating = false;
        if let Some(command) = &item.completion.command
            && let Err(err) = crate::commands::execute_command(cx, command)
        {
            log::warn!("{err:#}");
        }
        // Accepting is a trigger.
        self.trigger_inline_completions(cx);
        true
    }

    /// Accepts the next word of the ghost text (Monaco `acceptNextWord`,
    /// Cmd/Ctrl+Right).
    pub fn accept_next_inline_word(&mut self, cx: &mut Context<Self>) -> bool {
        let regex = self.word_regex(cx);
        self.accept_inline_partially(cx, |text| next_word_length(text, &regex))
    }

    /// Accepts the ghost text up to and including the next line break
    /// (Monaco `acceptNextLine`).
    pub fn accept_next_inline_line(&mut self, cx: &mut Context<Self>) -> bool {
        self.accept_inline_partially(cx, |text| {
            text.find('\n').map_or(text.encode_utf16().count(), |ix| {
                text[..=ix].encode_utf16().count()
            })
        })
    }

    fn word_regex(&self, cx: &gpui::App) -> syntax::fancy_regex::Regex {
        let model = self.model().read(cx);
        model
            .language()
            .and_then(|language| language.config.word_pattern.clone())
            .and_then(|pattern| pattern.regex().cloned())
            .unwrap_or_else(|| crate::cursor::in_place_replace::default_word_regex().clone())
    }

    /// Monaco `_acceptNext`.
    fn accept_inline_partially(
        &mut self,
        cx: &mut Context<Self>,
        length: impl FnOnce(&str) -> usize,
    ) -> bool {
        let Some((visible, selected, ghost)) = self.inline_state(cx) else {
            return false;
        };
        let item = self.inline.items[visible[selected].0].clone();
        if item.completion.is_snippet {
            return self.accept_inline_suggestion(cx);
        }
        let Some(first) = ghost.parts.first() else {
            return false;
        };
        let text = first.text();
        let accepted = length(&text);
        let total = text.encode_utf16().count();
        if accepted >= total && ghost.parts.len() == 1 {
            return self.accept_inline_suggestion(cx);
        }
        let partial = utf16_split(&text, accepted).0;
        let cursor = self.cursors.primary().position;
        let ghost_position = PointUtf16::new(ghost.line, first.column);
        // The text between the cursor and the ghost text, then the part.
        let between = self.text_range(cursor..ghost_position);
        let inserted = format!("{between}{partial}");
        self.inline.updating = true;
        self.replace_text(cursor..ghost_position, &inserted, &[], cx);
        let end = end_after(cursor, &inserted);
        self.set_selections(&[Selection::collapsed(end)], cx);
        self.reveal_range(end..end, crate::RevealType::Simple, cx);
        self.inline.updating = false;
        // Monaco counts from the start of the item's range.
        let accepted_length = self
            .current_edit(&item, cx)
            .filter(|edit| edit.range.start.row == end.row)
            .map_or(accepted, |edit| {
                end.column.saturating_sub(edit.range.start.column) as usize
            });
        item.provider
            .handle_partial_accept(&item.completion, accepted_length);
        cx.notify();
        true
    }

    fn text_range(&self, range: Range<PointUtf16>) -> String {
        let start = self.snapshot.point_utf16_to_offset(range.start);
        let end = self.snapshot.point_utf16_to_offset(range.end);
        self.snapshot.rope().text_for_range(start..end.max(start))
    }

    /// One edit (plus `other` edits elsewhere) as an undoable operation.
    fn replace_text(
        &mut self,
        range: Range<PointUtf16>,
        text: &str,
        other: &[(Range<PointUtf16>, String)],
        cx: &mut Context<Self>,
    ) {
        let mut edits = vec![(range, text.to_string())];
        edits.extend(other.iter().cloned());
        edits.sort_by_key(|(range, _)| range.start);
        self.apply(
            crate::cursor::EditOperation {
                edits,
                selections_after: self.cursors.selections(),
                kind: crate::cursor::EditOperationType::Other,
                push_stack_element_before: false,
                push_stack_element_after: false,
                auto_closed: Vec::new(),
                follow_up: None,
                auto_whitespace: Vec::new(),
            },
            cx,
        );
    }
}

/// Where text inserted at `start` ends.
fn end_after(start: PointUtf16, text: &str) -> PointUtf16 {
    match text.rfind('\n') {
        Some(ix) => PointUtf16::new(
            start.row + text.matches('\n').count() as u32,
            text[ix + 1..].encode_utf16().count() as u32,
        ),
        None => PointUtf16::new(start.row, start.column + text.encode_utf16().count() as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &'static str) -> impl Fn(u32) -> String {
        move |row| text.split('\n').nth(row as usize).unwrap_or("").to_string()
    }

    #[test]
    fn visibility_follows_typed_text() {
        let edit = |text: &str| TextReplacement {
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 3),
            text: text.to_string(),
        };
        let start = PointUtf16::new(0, 0);
        // "pri" typed, suggestion "println" over it.
        assert!(is_visible(
            &edit("println"),
            start,
            PointUtf16::new(0, 3),
            &lines("pri")
        ));
        // Typed text that disagrees.
        assert!(!is_visible(
            &edit("println"),
            start,
            PointUtf16::new(0, 3),
            &lines("prx")
        ));
        // Text right of the cursor must appear, in order, in the rest.
        let edit = TextReplacement {
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 4),
            text: "foo(a, b)".into(),
        };
        assert!(is_visible(
            &edit,
            start,
            PointUtf16::new(0, 3),
            &lines("foo)")
        ));
        assert!(!is_visible(
            &edit,
            start,
            PointUtf16::new(0, 3),
            &lines("foo]")
        ));
        // Another line: not shown.
        assert!(!is_visible(
            &edit,
            start,
            PointUtf16::new(1, 0),
            &lines("foo)\n")
        ));
    }

    #[test]
    fn next_word_lengths() {
        let word = crate::cursor::in_place_replace::default_word_regex();
        assert_eq!(next_word_length("hello world", word), 5);
        assert_eq!(next_word_length("(a, b)", word), 1);
        assert_eq!(next_word_length("  foo", word), 2);
        assert_eq!(next_word_length("abc", word), 3);
    }

    #[test]
    fn end_positions() {
        assert_eq!(
            end_after(PointUtf16::new(1, 2), "ab"),
            PointUtf16::new(1, 4)
        );
        assert_eq!(
            end_after(PointUtf16::new(1, 2), "a\nbc"),
            PointUtf16::new(2, 2)
        );
    }
}
