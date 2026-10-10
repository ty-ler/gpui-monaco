//! The inline completions controller (Monaco's `InlineCompletionsModel`,
//! `InlineCompletionsSource` and `InlineCompletionsController`, classic
//! ghost text only): requests completions while typing, keeps them through
//! edits that match them, shows the selected one as ghost text after the
//! cursor (extra lines in a view zone), and accepts all of it, a word or a
//! line.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Context, IntoElement, ParentElement, Render, SharedString, Styled, Task,
    Window, div, px,
};
use text::PointUtf16;

use super::ghost_text::{
    GhostText, GhostTextMode, TextReplacement, compute_ghost_text, single_text_remove_common_prefix,
};
use crate::cursor::Selection;
use crate::decorations::{DecorationId, DecorationOptions, InjectedText};
use crate::editor::CodeEditor;
use crate::events::{CursorChangeReason, CursorChangeSource, SelectionChange};
use crate::options::InlineSuggestMode;
use crate::providers::{
    InlineCompletion, InlineCompletionContext, InlineCompletionTriggerKind,
    InlineCompletionsProvider, inline_completions_providers,
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
struct Item {
    provider: Arc<dyn InlineCompletionsProvider>,
    completion: InlineCompletion,
    /// Its range as byte offsets in the text of `version`.
    range: Range<usize>,
    version: text::Version,
    /// Where it started, to drop it if text before it is deleted.
    original_start: PointUtf16,
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
    items: Vec<Item>,
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
}

/// The additional lines of a ghost text, below its line.
pub struct GhostLines {
    lines: Vec<SharedString>,
    style: TokenStyle,
    font: SharedString,
    font_size: gpui::Pixels,
    line_height: gpui::Pixels,
    left: gpui::Pixels,
}

impl Render for GhostLines {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let color = self.style.foreground.unwrap_or(gpui::transparent_black());
        div()
            .pl(self.left)
            .font_family(self.font.clone())
            .text_size(self.font_size)
            .line_height(self.line_height)
            .text_color(color)
            .italic()
            .children(self.lines.iter().map(|line| {
                div()
                    .h(self.line_height)
                    .whitespace_nowrap()
                    .child(line.clone())
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
        self.fetch_inline_completions(InlineCompletionTriggerKind::Explicit, cx);
    }

    /// The text changed: requests again while active.
    pub(crate) fn inline_completions_after_edit(&mut self, cx: &mut Context<Self>) {
        if self.inline.active && !self.inline.updating {
            self.fetch_inline_completions(InlineCompletionTriggerKind::Automatic, cx);
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
        let language = model.language_id().map(|l| l.to_string());
        let providers = inline_completions_providers(cx, language.as_deref());
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
        self.inline.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok(requests) = this.update(cx, |this, cx| {
                // Stale if the text or cursor moved meanwhile.
                let model = this.model().read(cx);
                if model.buffer().version() != version
                    || this.cursors.primary().position != position
                {
                    return None;
                }
                let context = model.provider_context();
                let selected_suggestion_info = None;
                Some(
                    providers
                        .iter()
                        .map(|provider| {
                            let task = provider.provide_inline_completions(
                                &context,
                                position,
                                InlineCompletionContext {
                                    trigger_kind: kind,
                                    selected_suggestion_info: selected_suggestion_info.clone(),
                                },
                                cx,
                            );
                            (provider.clone(), task)
                        })
                        .collect::<Vec<_>>(),
                )
            }) else {
                return;
            };
            let Some(requests) = requests else {
                this.update(cx, |this, _| this.inline.task = None).ok();
                return;
            };
            let mut lists = Vec::new();
            for (provider, task) in requests {
                match task.await {
                    Ok(list) => lists.push((provider, list)),
                    Err(err) => log::warn!("inline completions failed: {err:#}"),
                }
            }
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
        lists: Vec<(
            Arc<dyn InlineCompletionsProvider>,
            crate::providers::InlineCompletionList,
        )>,
        version: text::Version,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        let rope = self.model().read(cx).buffer().rope().clone();
        let line = rope.line_len_utf16(position.row);
        let word_start = self.word_start_at(position);
        self.inline.suppress_suggestions = false;
        let mut items = Vec::new();
        for (provider, list) in lists {
            if let Some(suppress) = list.suppress_suggestions {
                self.inline.suppress_suggestions |= suppress;
            }
            for completion in list.items {
                // Monaco's default range: the word at the cursor to the end
                // of the line.
                let range = completion.range.clone().unwrap_or_else(|| {
                    PointUtf16::new(position.row, word_start)..PointUtf16::new(position.row, line)
                });
                let start = rope
                    .point_utf16_to_offset(rope.clip_point_utf16(range.start, text::Bias::Left));
                let end =
                    rope.point_utf16_to_offset(rope.clip_point_utf16(range.end, text::Bias::Left));
                items.push(Item {
                    provider: provider.clone(),
                    completion: InlineCompletion {
                        insert_text: completion
                            .insert_text
                            .replace("\r\n", "\n")
                            .replace('\r', "\n"),
                        ..completion
                    },
                    range: start..end.max(start),
                    version,
                    original_start: range.start,
                });
            }
        }
        self.inline.items = items;
        cx.notify();
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

    /// Whether the visible list asks the suggest widget to stay closed
    /// (Monaco `inlineSuggestionSuppressSuggestions`).
    pub(crate) fn inline_suppresses_suggestions(&self, cx: &gpui::App) -> bool {
        (self.inline.suppress_suggestions || self.options.inline_suggest.suppress_suggestions)
            && self.inline_suggestion_visible(cx)
    }

    /// Renders the ghost text before painting.
    pub(crate) fn refresh_ghost_text(&mut self, cx: &mut Context<Self>) {
        let state = self.inline_state(cx);
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
        let mut decorations = Vec::new();
        let mut extra_lines: Vec<String> = Vec::new();
        for part in &ghost.parts {
            let offset = rope.point_utf16_to_offset(PointUtf16::new(ghost.line, part.column));
            decorations.push((
                offset..offset,
                DecorationOptions {
                    after: Some(InjectedText {
                        content: part.lines[0].clone().into(),
                        style: style(part.preview),
                        background: visible_color(colors.ghost_text_background),
                    }),
                    border: visible_color(colors.ghost_text_border),
                    ..Default::default()
                },
            ));
            extra_lines.extend(part.lines.iter().skip(1).cloned());
        }
        let zone_style = style(false);
        let owner = Some(cx.entity_id());
        let ids = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &[], decorations, cx)
        });
        let zone = (!extra_lines.is_empty()).then(|| {
            let view = cx.new(|_| GhostLines {
                lines: extra_lines.iter().map(|l| l.clone().into()).collect(),
                style: zone_style,
                font: self.options.font_family.clone(),
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

    /// Hides the ghost text until typing triggers it again (Monaco `stop`,
    /// Escape).
    pub fn hide_inline_suggestion(&mut self, cx: &mut Context<Self>) {
        self.inline.active = false;
        self.inline.items.clear();
        self.inline.selected = None;
        self.inline.task = None;
        self.inline.requested = None;
        self.clear_ghost_text(cx);
        cx.notify();
    }

    /// Selections moved: an explicit move (keys, mouse) or an API change
    /// hides the ghost text (Monaco's controller).
    pub(crate) fn inline_selections_changed(
        &mut self,
        change: &SelectionChange,
        cx: &mut Context<Self>,
    ) {
        if self.inline.updating || !self.inline.active {
            return;
        }
        if change.reason == CursorChangeReason::Explicit || change.source == CursorChangeSource::Api
        {
            self.hide_inline_suggestion(cx);
        }
    }

    /// The editor lost focus: hides unless `keep_on_blur`.
    pub(crate) fn inline_blurred(&mut self, cx: &mut Context<Self>) {
        if !self.options.inline_suggest.keep_on_blur && self.inline.active {
            self.hide_inline_suggestion(cx);
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

    /// Accepts the shown completion (Monaco
    /// `editor.action.inlineSuggest.commit`, Tab).
    pub fn accept_inline_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((visible, selected, _)) = self.inline_state(cx) else {
            return false;
        };
        let (ix, edit) = visible[selected].clone();
        let item = self.inline.items[ix].clone();
        self.inline.updating = true;
        self.hide_inline_suggestion(cx);
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
