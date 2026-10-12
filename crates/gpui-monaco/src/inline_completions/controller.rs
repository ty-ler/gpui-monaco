//! The inline completions controller (Monaco's `InlineCompletionsModel`,
//! `InlineCompletionsSource` and `InlineCompletionsController`): requests
//! completions while typing, keeps them through edits that match them,
//! shows the selected one as ghost text after the cursor (extra lines in a
//! view zone, with the rest of the line moved to their end as Monaco's
//! `GhostTextView` does), at every other cursor it fits too
//! (`getSecondaryEdits`), and accepts all of it, a word or a line at each.
//! A list asking for forward stability keeps its shown item first while
//! typing agrees with it, and each list is disposed once none of its
//! items is used (Monaco's ref-counted `InlineSuggestionList`).
//!
//! Inline edits (items with `is_inline_edit`, a `uri`, a `jump_to_position`
//! or a `hint`) are diffed into replacements that follow the user's typing
//! (Monaco `InlineEditItem`); the last one shows when no completion is
//! visible at the cursor, through [`super::inline_edits_view`]. Tab jumps to
//! it or accepts it, as Monaco's `tabShouldJumpToInlineEdit` and
//! `tabShouldAcceptInlineEdit` say; Shift+Tab runs its alternative action.
//! A cross-file inline edit (Monaco's `uri`) moves to an editor showing its
//! model when accepted (Monaco `transplantCompletion`).
//!
//! Also Monaco's undo and redo special cases: undoing an accepted inline
//! edit doesn't ask again (`_didUndoInlineEdits`), undoing back to where an
//! inline edit showed asks only for inline edits
//! (`_lastShownInlineCompletionInfo`), and undo and redo keep the shown item
//! (`preserveCurrentCompletion`). Accepting asks again even without a text
//! change (the controller's `noDelay` trigger) and fades the accepted text
//! out (`FadeoutDecoration`).

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use web_time::Instant;

use gpui::{
    AppContext as _, Context, IntoElement, ParentElement, Render, SharedString, Styled, Task,
    Window, div, px,
};
use text::PointUtf16;

use super::ghost_text::{
    GhostLinePiece, GhostText, GhostTextMode, GhostTextViewData, TextReplacement,
    compute_ghost_text, compute_ghost_text_view_data, remove_common_prefix_and_suffix,
    secondary_edits, single_text_edit_augments, single_text_remove_common_prefix,
};
use super::inline_edits::{Changes, Replacements, UpdatedEdit, diffed_edits, new_ranges};
use crate::cursor::Selection;
use crate::decorations::{DecorationId, DecorationOptions, InjectedText};
use crate::editor::CodeEditor;
use crate::events::{CursorChangeCause, CursorChangeReason, CursorChangeSource};
use crate::model::{EditKind, TextModel};
use crate::options::InlineSuggestMode;
use crate::providers::{
    Command, InlineCompletion, InlineCompletionChangeHint, InlineCompletionContext,
    InlineCompletionEndOfLifeReason, InlineCompletionHint, InlineCompletionList,
    InlineCompletionTriggerKind, InlineCompletionsDisposeReason, InlineCompletionsProvider,
    Location, PartialAcceptInfo, PartialAcceptTriggerKind, SelectedSuggestionInfo,
};
use crate::theme::TokenStyle;
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};

/// The editor's debounce when providers don't set one (Monaco configures
/// its debounce service with 50ms).
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(50);

/// The visible items (index, current edit), the selected one, and its
/// ghost text.
pub(crate) type InlineView = (Vec<(usize, TextReplacement)>, usize, GhostText);

/// A provider's list, shared by its items (Monaco's ref-counted
/// `InlineSuggestionList`): the provider's `dispose_inline_completions`
/// runs when the last holder drops it, with the reason last set.
pub(crate) struct ListRef {
    provider: Arc<dyn InlineCompletionsProvider>,
    pub(crate) list: InlineCompletionList,
    reason: std::cell::Cell<InlineCompletionsDisposeReason>,
}

impl ListRef {
    /// A list just returned for a request, disposed with
    /// `TokenCancellation` unless the request gets to use it.
    fn new(provider: Arc<dyn InlineCompletionsProvider>, list: InlineCompletionList) -> Rc<Self> {
        Rc::new(Self {
            provider,
            list,
            reason: std::cell::Cell::new(InlineCompletionsDisposeReason::TokenCancellation),
        })
    }
}

impl Drop for ListRef {
    fn drop(&mut self) {
        self.provider
            .dispose_inline_completions(&self.list, self.reason.get());
    }
}

/// A completion and where it applies, followed through edits.
#[derive(Clone)]
pub(crate) struct Item {
    pub(crate) provider: Arc<dyn InlineCompletionsProvider>,
    /// The list it came in (Monaco `source`).
    list: Rc<ListRef>,
    pub(crate) completion: InlineCompletion,
    /// The text it inserts: `completion.insert_text` with Monaco's
    /// `closeBrackets` when `complete_bracket_pairs` (Monaco's item
    /// `insertText`, while `completion` is the provider's).
    pub(crate) insert_text: String,
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
    /// (In this model even for a cross-file edit, as of the version given.)
    show_range: Option<(Range<usize>, text::Version)>,
    /// An inline edit's replacements (Monaco `InlineEditItem`).
    edit: Option<InlineEditItem>,
    /// It was jumped to (Monaco `identity.jumpedTo`); kept by an equal item
    /// from a newer request.
    pub(crate) jumped: bool,
}

/// An inline edit's replacements, followed through every change.
#[derive(Clone)]
pub(crate) struct InlineEditItem {
    edits: Vec<UpdatedEdit>,
    /// The text version `edits` and `jump_to` are in (of `target`'s buffer
    /// for a cross-file edit).
    version: text::Version,
    /// The change count when it was created or typing last matched it
    /// (Monaco `updatedEditModelVersion`).
    updated_at: u64,
    /// The last change typed part of it (Monaco
    /// `_lastChangePartOfInlineEdit`).
    last_change_updated: bool,
    /// Monaco's `jumpTo` action: where Tab moves the cursor (a byte offset).
    jump_to: Option<usize>,
    /// Monaco `hint`: shown at this byte range of this editor's model (as
    /// of the version given), instead of the diff.
    hint: Option<(Range<usize>, text::Version, InlineCompletionHint)>,
    /// The model edited, when it isn't this editor's (Monaco
    /// `originalTextRef` of a cross-file edit), and its uri.
    pub(crate) target: Option<(gpui::Entity<TextModel>, SharedString)>,
}

impl InlineEditItem {
    /// Monaco `action === undefined`: only a hint, nothing to apply or jump
    /// to.
    pub(crate) fn hint_only(&self) -> bool {
        self.edits.is_empty() && self.jump_to.is_none() && self.target.is_none()
    }
}

/// How a request is made (Monaco `trigger`'s options and the change
/// summary of `_fetchInlineCompletionsPromise`).
#[derive(Clone, Default)]
pub(crate) struct FetchOptions {
    /// Monaco `noDelay`: ask again even if an equal request was made (still
    /// debounced, as in Monaco).
    pub force: bool,
    /// Ask only for inline edits (undoing back to where one showed).
    pub only_inline_edits: bool,
    /// Monaco `preserveCurrentCompletion` (undo and redo): keep this item,
    /// if it still applies, ahead of the new ones.
    pub preserve: Option<u64>,
    /// Ask only this provider (Monaco `_fetchSpecificProviderSignal`, after
    /// the provider's `onDidChangeInlineCompletions`).
    pub provider: Option<Arc<dyn InlineCompletionsProvider>>,
    /// The hint that provider passed (Monaco `changeHint`).
    pub change_hint: Option<InlineCompletionChangeHint>,
    /// Log "No inline suggestion is available." when the request shows
    /// nothing (`inlineSuggest.trigger`'s `showNoResultNotification`; the
    /// standalone notification service logs it to the console).
    pub notify_no_result: bool,
}

/// What standalone Monaco's notification service logs for
/// `showNoResultNotification`.
pub(crate) const NO_INLINE_SUGGESTION: &str = "No inline suggestion is available.";

/// Cross-file inline edits accepted while no editor showed their model, by
/// uri: the next editor to show it takes the edit (Monaco opens the editor
/// and calls `transplantCompletion`).
#[derive(Default)]
struct PendingInlineEdits(std::collections::HashMap<SharedString, Item>);

impl gpui::Global for PendingInlineEdits {}

/// Monaco `FadeoutDecoration`: the accepted text's highlight fades over
/// this long.
pub(crate) const FADE_OUT_DURATION: Duration = Duration::from_millis(1000);

/// Monaco drops an inline edit ignored for this many changes.
const INLINE_EDIT_MAX_IGNORED_CHANGES: u64 = 20;

/// Why inline completions stop (Monaco `stop`'s reason, or accepting).
#[derive(Clone, Copy, PartialEq, Eq)]
enum StopReason {
    /// Escape or `hide`: the shown completion is rejected.
    Explicit,
    /// The cursor moved or the editor lost focus.
    Automatic,
    /// The item at this index was accepted, with its alternative action
    /// or not.
    Accepted(usize, bool),
}

/// What is rendered: the ghost texts (one per cursor that gets the
/// completion), their decorations, the zones with their additional lines
/// and the hidden ends of their lines.
struct Rendered {
    /// The text version, the cursors, the item shown (`usize::MAX` for
    /// the suggest widget's preview) and whether it added lines.
    key: (text::Version, Vec<PointUtf16>, usize, bool),
    decorations: Vec<DecorationId>,
    /// The zones with additional lines, by the row they're below.
    zones: Vec<(u32, WidgetId, gpui::Entity<GhostLines>)>,
    /// Monaco `hiddenRange`: each ghost text's row and the column its line
    /// is hidden from.
    hidden: Vec<(u32, u32)>,
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
    pub(crate) jumped_to: Option<u64>,
    /// The inline edit's view.
    pub(crate) edit_view: super::inline_edits_view::InlineEditViewState,
    /// Monaco `_lastAcceptedInlineCompletionInfo`: the text version right
    /// after accepting, and whether it was an inline edit.
    last_accepted: Option<(text::Version, bool)>,
    /// The text version at the last change seen.
    seen_version: Option<text::Version>,
    /// The text version right after accepting a word or line (Monaco
    /// `_isAcceptingPartially`).
    accepted_partially: Option<text::Version>,
    /// Monaco `_lastShownInlineCompletionInfo`: the model's alternative
    /// version id when an inline edit last started showing.
    last_shown: Option<usize>,
    /// Monaco `FadeoutDecoration`: the accepted text's highlight, fading.
    fade_out: Option<(Vec<DecorationId>, Task<()>)>,
    /// Monaco `suggestWidgetInlineCompletions`: the items of the last
    /// request made with the suggest widget's selection, which extend that
    /// selection's preview; `items` is empty meanwhile.
    pub(crate) suggest_items: Vec<Item>,
    /// The request sequence numbers of `items` and `suggest_items` (Monaco
    /// compares their requests' versions when seeding).
    items_request: u64,
    suggest_items_request: u64,
    next_request: u64,
    /// The suggest widget's selection last seen (Monaco
    /// `SuggestWidgetAdaptor.selectedItem`), and whether its list was
    /// incomplete.
    suggest_item: Option<(SelectedSuggestionInfo, bool)>,
    /// The suggest widget's selection the last request passed.
    requested_selection: Option<SelectedSuggestionInfo>,
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

/// Ghost texts with the edits they show, the primary cursor's first.
type GhostTexts = Vec<(TextReplacement, GhostText)>;

/// A piece of an additional ghost text line and its style.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GhostSegment {
    text: String,
    style: TokenStyle,
    /// Suggested text (Monaco's `ghost-text` class), rather than text of
    /// the line moved below: it's in the inline suggestion font and gets
    /// the warning squiggle.
    ghost: bool,
}

/// The additional lines of a ghost text, below its line (Monaco
/// `AdditionalLinesWidget`).
pub struct GhostLines {
    lines: Vec<Vec<GhostSegment>>,
    font: SharedString,
    /// `inline_suggest.font_family`, for the suggested text.
    ghost_font: Option<SharedString>,
    font_size: gpui::Pixels,
    line_height: gpui::Pixels,
    left: gpui::Pixels,
    /// The editor's text color, for pieces without their own.
    foreground: gpui::Hsla,
    /// Monaco's `warning` class: the suggested text gets the warning
    /// squiggle in this color.
    warning: Option<gpui::Hsla>,
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
                    .children(line.iter().map(|segment| {
                        let style = &segment.style;
                        let mut color = style.foreground.unwrap_or(self.foreground);
                        color.a *= style.opacity.unwrap_or(1.);
                        let mut span = div().text_color(color).child(segment.text.clone());
                        if segment.ghost
                            && let Some(font) = &self.ghost_font
                        {
                            span = span.font_family(font.clone());
                        }
                        if style.italic {
                            span = span.italic();
                        }
                        if style.bold {
                            span = span.font_weight(gpui::FontWeight::BOLD);
                        }
                        // Monaco's `.ghost-text.warning`: every line of the
                        // suggested text gets the squiggle.
                        if segment.ghost
                            && let Some(warning) = self.warning
                        {
                            span = span
                                .text_decoration_1()
                                .text_decoration_wavy()
                                .text_decoration_color(warning);
                        }
                        span
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
        self.fetch_inline_completions(
            InlineCompletionTriggerKind::Automatic,
            FetchOptions::default(),
            cx,
        );
    }

    /// Monaco's controller after `editor.action.inlineSuggest.commit`:
    /// `trigger({ noDelay: true })`, which asks again even when the text
    /// didn't change (accepting a jump or an alternative action).
    fn trigger_inline_completions_after_accept(&mut self, cx: &mut Context<Self>) {
        if !self.options.inline_suggest.enabled || self.options.read_only {
            return;
        }
        self.inline.active = true;
        self.fetch_inline_completions(
            InlineCompletionTriggerKind::Automatic,
            FetchOptions {
                force: true,
                ..Default::default()
            },
            cx,
        );
    }

    /// Asks for inline completions now (Monaco
    /// `editor.action.inlineSuggest.trigger`, Alt+\).
    pub fn trigger_inline_suggest(&mut self, cx: &mut Context<Self>) {
        self.trigger_inline_suggest_with(true, None, false, cx);
    }

    /// Monaco `editor.action.inlineSuggest.trigger` with its arguments:
    /// `explicit` (else as typing asks), `provider_id` (only the provider
    /// it names, see [`InlineCompletionsProvider::provider_id`]; every
    /// provider when none matches) and `show_no_result_notification` (logs
    /// "No inline suggestion is available." when nothing shows, as
    /// standalone Monaco's notification service does).
    pub fn trigger_inline_suggest_with_args(
        &mut self,
        explicit: bool,
        provider_id: Option<&str>,
        show_no_result_notification: bool,
        cx: &mut Context<Self>,
    ) {
        let provider = provider_id.and_then(|id| {
            self.editor_inline_completions_providers(cx)
                .into_iter()
                .find(|provider| {
                    crate::providers::inline_completions_provider_matches(provider.as_ref(), id)
                })
        });
        self.trigger_inline_suggest_with(explicit, provider, show_no_result_notification, cx);
    }

    fn trigger_inline_suggest_with(
        &mut self,
        explicit: bool,
        provider: Option<Arc<dyn InlineCompletionsProvider>>,
        notify_no_result: bool,
        cx: &mut Context<Self>,
    ) {
        if self.options.read_only || self.widget_input {
            return;
        }
        // Typing asks only with inline suggestions on.
        if !explicit && !self.options.inline_suggest.enabled {
            return;
        }
        self.inline.active = true;
        // Monaco `trigger({ explicit: true })` starts an accept flow, and
        // keeps the shown item ahead of the new ones
        // (`_forceUpdateExplicitlySignal` sets `preserveCurrentCompletion`).
        let preserve = if explicit {
            self.inline.in_accept_flow = true;
            self.inline_preserve_candidate(cx)
        } else {
            None
        };
        let kind = if explicit {
            InlineCompletionTriggerKind::Explicit
        } else {
            InlineCompletionTriggerKind::Automatic
        };
        self.fetch_inline_completions(
            kind,
            FetchOptions {
                preserve,
                provider,
                notify_no_result,
                ..Default::default()
            },
            cx,
        );
    }

    /// `showNoResultNotification`: logs when no inline suggestion or edit
    /// shows (Monaco checks the model's state after the request).
    fn notify_if_no_inline_suggestion(&self, cx: &gpui::App) {
        if !self.inline_suggestion_visible(cx) && !self.inline_edit_visible(cx) {
            log::info!("{NO_INLINE_SUGGESTION}");
        }
    }

    /// The text changed: inline edits follow it, and while active it
    /// requests again, with Monaco's undo and redo special cases.
    pub(crate) fn inline_completions_after_edit(&mut self, kind: EditKind, cx: &mut Context<Self>) {
        self.inline.change_count += 1;
        let version = self.model().read(cx).buffer().version();
        let previous = self.inline.seen_version.replace(version);
        // Monaco resets `_inAcceptFlow` on every change; accepting sets it
        // again after its edit, whose event only arrives once accepting is
        // done.
        if self
            .inline
            .last_accepted
            .is_none_or(|(accepted, _)| accepted != version)
        {
            self.inline.in_accept_flow = false;
        }
        // Monaco `preserveCurrentCompletion`: the item showing through an
        // undo, a redo or accepting part of it.
        let accepted_partially = self.inline.accepted_partially == Some(version);
        let preserve = accepted_partially
            .then(|| self.inline_preserve_candidate(cx))
            .flatten();
        self.update_inline_edit_items(cx);
        if !self.inline.active || self.inline.updating {
            return;
        }
        // Monaco `_didUndoInlineEdits`: undoing an inline edit just
        // accepted clears the items and doesn't ask again.
        let undid_inline_edit = kind == EditKind::Undo
            && self
                .inline
                .last_accepted
                .is_some_and(|(at, inline_edit)| inline_edit && Some(at) == previous);
        if undid_inline_edit {
            self.inline.last_accepted = None;
            self.clear_inline_items(cx);
            return;
        }
        // Monaco: undoing back to where an inline edit showed asks for
        // inline edits only, to show one again.
        let alternative_version = self.model().read(cx).alternative_version_id();
        let only_inline_edits = self.inline.last_shown == Some(alternative_version);
        let options = FetchOptions {
            force: false,
            only_inline_edits,
            preserve,
            ..Default::default()
        };
        if matches!(kind, EditKind::Undo | EditKind::Redo) {
            // Undo and redo restore the cursor after the change: ask for
            // the position they leave, keeping what shows there.
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| {
                    if this.inline.active {
                        let options = FetchOptions {
                            preserve: this.inline_preserve_candidate(cx),
                            ..options
                        };
                        this.fetch_inline_completions(
                            InlineCompletionTriggerKind::Automatic,
                            options,
                            cx,
                        );
                    }
                })
                .ok();
            });
        } else {
            self.fetch_inline_completions(InlineCompletionTriggerKind::Automatic, options, cx);
        }
    }

    /// Monaco `itemToPreserveCandidate`: the selected visible completion,
    /// else the inline edit.
    fn inline_preserve_candidate(&self, cx: &gpui::App) -> Option<u64> {
        if let Some((visible, selected, _)) = self.inline_state(cx) {
            return Some(self.inline.items[visible[selected].0].id);
        }
        self.inline
            .items
            .iter()
            .rev()
            .find(|item| item.edit.is_some())
            .map(|item| item.id)
    }

    /// Monaco `InlineCompletionsSource.clear`: drops the items, keeping
    /// inline completions active.
    fn clear_inline_items(&mut self, cx: &mut Context<Self>) {
        for item in std::mem::take(&mut self.inline.items) {
            item.provider.handle_end_of_lifetime(
                &item.completion,
                &InlineCompletionEndOfLifeReason::Ignored {
                    superseded_by: None,
                    user_typing_disagreed: false,
                },
            );
        }
        self.inline.task = None;
        self.inline.requested = None;
        cx.notify();
    }

    /// Monaco `InlineEditItem.withEdit`: moves each inline edit through the
    /// changes since it was last updated, dropping it when a change
    /// conflicts, when it was ignored for 20 changes, or when nothing is
    /// left to change. A jump target moves with the text and goes when the
    /// text around it is replaced; a hint's range moves with the text.
    /// Cross-file edits don't follow this model.
    fn update_inline_edit_items(&mut self, cx: &mut Context<Self>) {
        let model = self.model().clone();
        let buffer = model.read(cx).buffer();
        let count = self.inline.change_count;
        for items in [&mut self.inline.items, &mut self.inline.suggest_items] {
            let mut dropped = Vec::new();
            items.retain_mut(|item| {
                let keep = item
                    .edit
                    .as_mut()
                    .is_none_or(|edit| follow_inline_edit(edit, buffer, count));
                if !keep {
                    dropped.push(item.clone());
                }
                keep
            });
            end_items(dropped);
        }
    }

    /// Monaco `InlineCompletionsSource.fetch`: debounced for automatic
    /// requests; an equal request already made satisfies it, unless
    /// `options.force`. While the suggest widget has a selection (and
    /// `show_on_suggest_conflict` doesn't say otherwise), the request
    /// passes it (Monaco `selectedSuggestionInfo`) and its results become
    /// `suggest_items`.
    fn fetch_inline_completions(
        &mut self,
        kind: InlineCompletionTriggerKind,
        options: FetchOptions,
        cx: &mut Context<Self>,
    ) {
        // Widget inputs are plain inputs in Monaco.
        if self.widget_input {
            return;
        }
        let model = self.model().read(cx);
        let version = model.buffer().version();
        let position = self.cursors.primary().position;
        let selected = self.inline_request_suggest_item(cx);
        if let Some((v, p, k)) = &self.inline.requested
            && !options.force
            && options.provider.is_none()
            && *v == version
            && *p == position
            && self.inline.requested_selection == selected
            && (kind == InlineCompletionTriggerKind::Automatic
                || *k == InlineCompletionTriggerKind::Explicit)
        {
            if options.notify_no_result {
                self.notify_if_no_inline_suggestion(cx);
            }
            return;
        }
        let providers = match &options.provider {
            Some(provider) => vec![provider.clone()],
            None => self.available_inline_completions_providers(cx),
        };
        if providers.is_empty() {
            if options.notify_no_result {
                self.notify_if_no_inline_suggestion(cx);
            }
            return;
        }
        // Monaco `itemToPreserve`: the shown item also stays first when its
        // list asked for forward stability (`enableForwardStability`).
        let options = FetchOptions {
            preserve: options.preserve.or_else(|| {
                self.inline_preserve_candidate(cx).filter(|id| {
                    self.inline
                        .items
                        .iter()
                        .find(|item| item.id == *id)
                        .is_some_and(|item| item.list.list.enable_forward_stability)
                })
            }),
            ..options
        };
        let notify_no_result = options.notify_no_result;
        let updating = self.inline.task.is_some();
        self.inline.requested = Some((version, position, kind));
        self.inline.requested_selection = selected.clone();
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
        // Monaco `earliestShownDateTime`: no wait when explicit or in the
        // accept flow.
        let min_show =
            if kind == InlineCompletionTriggerKind::Explicit || self.inline.in_accept_flow {
                Duration::ZERO
            } else {
                self.options.inline_suggest.min_show_delay
            };
        let started = Instant::now();
        let issued = web_time::SystemTime::now();
        let include_inline_edits = self.options.inline_suggest.edits.enabled;
        let include_inline_completions = !options.only_inline_edits;
        self.inline.next_request += 1;
        let request = self.inline.next_request;
        let context = InlineCompletionContext {
            trigger_kind: kind,
            selected_suggestion_info: selected.clone(),
            include_inline_edits,
            include_inline_completions,
            request_uuid: uuid::Uuid::new_v4().to_string().into(),
            request_issued_date_time: issued,
            earliest_shown_date_time: issued + min_show,
            change_hint: options.change_hint.clone(),
        };
        let for_suggest = selected.is_some();
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
                let provider_context = this.model().read(cx).provider_context();
                provider.provide_inline_completions(
                    &provider_context,
                    position,
                    context.clone(),
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
                this.update(cx, |this, cx| {
                    this.inline.task = None;
                    if notify_no_result {
                        this.notify_if_no_inline_suggestion(cx);
                    }
                })
                .ok();
                return;
            };
            let mut results: Vec<Option<Rc<ListRef>>> = vec![None; providers.len()];
            for ix in yield_order(&graph) {
                let task = match tasks[ix].take() {
                    Some(task) => task,
                    None => {
                        let yields = graph[ix].iter().filter_map(|&j| results[j].as_ref());
                        let lists: Vec<Rc<ListRef>> = yields.cloned().collect();
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
                    Ok(list) => results[ix] = Some(ListRef::new(providers[ix].clone(), list)),
                    Err(err) => log::warn!("inline completions failed: {err:#}"),
                }
            }
            let lists: Vec<Rc<ListRef>> = results.into_iter().flatten().collect();
            // Monaco `minShowDelay`.
            if let Some(wait) = min_show.checked_sub(started.elapsed())
                && !wait.is_zero()
            {
                cx.background_executor().timer(wait).await;
            }
            this.update(cx, |this, cx| {
                this.inline.task = None;
                if this.model().read(cx).buffer().version() == version {
                    this.set_inline_completions(
                        lists,
                        version,
                        position,
                        options,
                        request,
                        for_suggest,
                        cx,
                    );
                }
                if notify_no_result {
                    this.notify_if_no_inline_suggestion(cx);
                }
            })
            .ok();
        }));
    }

    /// Monaco `getAvailableProviders`: the editor's providers, less those
    /// whose group `inline_suggest.experimental.suppress_inline_suggestions`
    /// lists or another provider excludes (`excludesGroupIds`).
    fn available_inline_completions_providers(
        &mut self,
        cx: &gpui::App,
    ) -> Vec<Arc<dyn InlineCompletionsProvider>> {
        let suppressed = self
            .options
            .inline_suggest
            .experimental
            .suppress_inline_suggestions
            .clone();
        let providers: Vec<_> = self
            .editor_inline_completions_providers(cx)
            .into_iter()
            .filter(|p| {
                p.group_id()
                    .is_none_or(|group| !suppressed.contains(&group))
            })
            .collect();
        let excluded: HashSet<SharedString> = providers
            .iter()
            .flat_map(|p| p.excludes_group_ids())
            .collect();
        providers
            .into_iter()
            .filter(|p| p.group_id().is_none_or(|group| !excluded.contains(&group)))
            .collect()
    }

    /// Monaco `createStateWithAppliedResults`: the new items replace the
    /// old ones (or, for a request with the suggest widget's selection,
    /// become `suggest_items` while `items` empties).
    #[allow(clippy::too_many_arguments)]
    fn set_inline_completions(
        &mut self,
        lists: Vec<Rc<ListRef>>,
        version: text::Version,
        position: PointUtf16,
        options: FetchOptions,
        request: u64,
        for_suggest: bool,
        cx: &mut Context<Self>,
    ) {
        let rope = self.model().read(cx).buffer().rope().clone();
        let own_uri = self.model().read(cx).uri().cloned();
        let line = rope.line_len_utf16(position.row);
        let word_start = self.word_start_at(position);
        self.inline.suppress_suggestions = false;
        self.inline.commands.clear();
        let include_inline_edits = self.options.inline_suggest.edits.enabled;
        let include_inline_completions = !options.only_inline_edits;
        let mut items = Vec::new();
        let mut not_shown = Vec::new();
        for list_ref in &lists {
            // Monaco: a list none of whose items are kept ends `notTaken`
            // (`empty` without items) once the request is done with it.
            list_ref.reason.set(if list_ref.list.items.is_empty() {
                InlineCompletionsDisposeReason::Empty
            } else {
                InlineCompletionsDisposeReason::NotTaken
            });
            let provider = &list_ref.provider;
            let list = &list_ref.list;
            if let Some(suppress) = list.suppress_suggestions {
                self.inline.suppress_suggestions |= suppress;
            }
            self.inline.commands.extend(list.commands.iter().cloned());
            for completion in list.items.iter().cloned() {
                // Monaco `notInlineEditRequested` and
                // `notInlineCompletionRequested`.
                let wants_edits = completion.is_inline_edit || completion.show_inline_edit_menu;
                if (wants_edits && !include_inline_edits)
                    || (!wants_edits && !include_inline_completions)
                {
                    not_shown.push((provider.clone(), completion));
                    continue;
                }
                // Monaco opens a cross-file edit's model to diff against it.
                let target = match completion.uri.as_ref() {
                    Some(uri) if Some(uri) != own_uri.as_ref() => {
                        match crate::model::model_for_uri(cx, uri) {
                            Some(model) => Some((model, uri.clone())),
                            None => {
                                not_shown.push((provider.clone(), completion));
                                continue;
                            }
                        }
                    }
                    _ => None,
                };
                let (target_rope, target_version) = match &target {
                    Some((model, _)) => {
                        let buffer = model.read(cx).buffer();
                        (buffer.rope().clone(), buffer.version())
                    }
                    None => (rope.clone(), version),
                };
                let target_offset = |point: PointUtf16| {
                    target_rope.point_utf16_to_offset(
                        target_rope.clip_point_utf16(point, text::Bias::Left),
                    )
                };
                // Monaco `InlineSuggestionItem.create`: anything but a plain
                // edit of this model is an inline edit.
                let is_inline_edit = completion.is_inline_edit
                    || completion.uri.is_some()
                    || completion.jump_to_position.is_some()
                    || completion.hint.is_some();
                // Monaco's default range: the word at the cursor to the end
                // of the line.
                let range = completion.range.clone().unwrap_or_else(|| {
                    if target.is_some() {
                        PointUtf16::default()..PointUtf16::default()
                    } else {
                        PointUtf16::new(position.row, word_start)
                            ..PointUtf16::new(position.row, line)
                    }
                });
                let completion = InlineCompletion {
                    insert_text: completion
                        .insert_text
                        .replace("\r\n", "\n")
                        .replace('\r', "\n"),
                    ..completion
                };
                // Monaco `closeBrackets`, for this model's edits: the range
                // grows or shrinks at its end by what it added or removed.
                let mut range = range;
                let insert_text = if completion.complete_bracket_pairs && target.is_none() {
                    let fixed =
                        self.close_inline_brackets(&completion.insert_text, range.start, cx);
                    let diff = fixed.encode_utf16().count() as i64
                        - completion.insert_text.encode_utf16().count() as i64;
                    if diff != 0 {
                        let column = (range.end.column as i64 + diff).max(0) as u32;
                        range.end = rope.clip_point_utf16(
                            PointUtf16::new(range.end.row, column),
                            text::Bias::Left,
                        );
                        range.end = range.end.max(range.start);
                    }
                    fixed
                } else {
                    completion.insert_text.clone()
                };
                let start = target_offset(range.start);
                let end = target_offset(range.end).max(start);
                let jump_to = completion.jump_to_position.map(target_offset);
                let offset = |point: PointUtf16| {
                    rope.point_utf16_to_offset(rope.clip_point_utf16(point, text::Bias::Left))
                };
                // Monaco `InlineEditItem.create`: the replacements a diff
                // finds (none for a jump), a jump target or a hint.
                let edit = is_inline_edit.then(|| InlineEditItem {
                    edits: if jump_to.is_some() {
                        Vec::new()
                    } else {
                        diffed_edits(&target_rope, start..end, &insert_text)
                            .into_iter()
                            .map(|(range, text)| {
                                let replaced = target_rope.text_for_range(range.clone());
                                UpdatedEdit::new(range, text, &replaced)
                            })
                            .collect()
                    },
                    version: target_version,
                    updated_at: self.inline.change_count,
                    last_change_updated: false,
                    jump_to,
                    hint: completion.hint.as_ref().map(|hint| {
                        let start = offset(hint.range.start);
                        (
                            start..offset(hint.range.end).max(start),
                            version,
                            hint.clone(),
                        )
                    }),
                    target: target.clone(),
                });
                if edit.as_ref().is_some_and(|edit| {
                    edit.edits.is_empty() && edit.jump_to.is_none() && edit.hint.is_none()
                }) {
                    // It changes nothing.
                    not_shown.push((provider.clone(), completion));
                    continue;
                }
                let show_range = completion.show_range.as_ref().map(|range| {
                    let start = offset(range.start);
                    (start..offset(range.end).max(start), version)
                });
                self.inline.next_id += 1;
                items.push(Item {
                    provider: provider.clone(),
                    list: list_ref.clone(),
                    completion,
                    insert_text,
                    range: start..end,
                    version: target_version,
                    original_start: range.start,
                    shown: false,
                    id: self.inline.next_id,
                    commands: list.commands.clone(),
                    show_range,
                    edit,
                    jumped: false,
                });
            }
        }
        // Monaco sorts items asking for the inline edit menu last.
        items.sort_by_key(|item| item.completion.show_inline_edit_menu);
        // Monaco `itemToPreserve` (undo and redo): the item showing before
        // goes first, the equal new one if there is one, while it still
        // applies (Monaco `canBeReused`).
        let preserved = options
            .preserve
            .and_then(|id| self.inline.items.iter().find(|item| item.id == id))
            .filter(|item| self.inline_item_can_be_reused(item, position, cx))
            .cloned();
        let mut kept_old = None;
        if let Some(old) = &preserved {
            match items.iter().position(|new| same_item(new, old)) {
                Some(ix) => {
                    let new = items.remove(ix);
                    items.insert(0, new);
                }
                None => {
                    kept_old = Some(old.id);
                    // Followed to this version, so later changes move it
                    // from here.
                    let mut old = old.clone();
                    if old.edit.is_none()
                        && let Some(edit) = self.current_edit(&old, cx)
                    {
                        old.range = rope.point_utf16_to_offset(edit.range.start)
                            ..rope.point_utf16_to_offset(edit.range.end);
                        old.version = version;
                    }
                    items.insert(0, old);
                }
            }
        }
        // Monaco `createStateWithAppliedResults`: the preserved item's kind
        // wins; else completions win when one is visible at the cursor, else
        // only inline edits are kept.
        let line_text = |row: u32| rope.line(row);
        let prefer_completions = match &preserved {
            Some(item) => item.edit.is_none(),
            None => items.iter().any(|item| {
                item.edit.is_none()
                    && self.item_shows_at(item, position, cx)
                    && self.current_edit(item, cx).is_some_and(|edit| {
                        is_visible(&edit, item.original_start, position, &line_text)
                    })
            }),
        };
        let (kept, dropped): (Vec<Item>, Vec<Item>) = items
            .into_iter()
            .partition(|item| prefer_completions != item.edit.is_some());
        // Monaco: a list with kept items is disposed (`other`) once the
        // last of them goes.
        for item in &kept {
            item.list.reason.set(InlineCompletionsDisposeReason::Other);
        }
        not_shown.extend(
            dropped
                .into_iter()
                .filter(|item| Some(item.id) != kept_old)
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
        // Monaco: a request with the suggest widget's selection empties the
        // items and fills the suggest widget's; any other empties those.
        let other = if for_suggest {
            self.inline.items_request = 0;
            self.inline.suggest_items_request = request;
            std::mem::take(&mut self.inline.items)
        } else {
            self.inline.items_request = request;
            self.inline.suggest_items_request = 0;
            std::mem::take(&mut self.inline.suggest_items)
        };
        end_items(other);
        let target = if for_suggest {
            &mut self.inline.suggest_items
        } else {
            &mut self.inline.items
        };
        // Monaco: replaced items end, superseded by an equal new one, which
        // keeps their identity (and so whether they showed or were jumped
        // to). A preserved item kept as it was lives on.
        for old in std::mem::replace(target, kept) {
            let target = if for_suggest {
                &mut self.inline.suggest_items
            } else {
                &mut self.inline.items
            };
            if Some(old.id) == kept_old {
                if !target.iter().any(|item| item.id == old.id) {
                    old.provider.handle_end_of_lifetime(
                        &old.completion,
                        &InlineCompletionEndOfLifeReason::Ignored {
                            superseded_by: None,
                            user_typing_disagreed: false,
                        },
                    );
                }
                continue;
            }
            let superseded_by = target
                .iter_mut()
                .find(|new| same_item(new, &old))
                .map(|new| {
                    new.shown |= old.shown;
                    new.jumped |= old.jumped;
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

    /// Monaco `canBeReused`: a completion still visible at `position`, its
    /// range around it; an inline edit the last change typed part of.
    fn inline_item_can_be_reused(&self, item: &Item, position: PointUtf16, cx: &gpui::App) -> bool {
        match &item.edit {
            Some(edit) => {
                edit.target.is_none()
                    && edit.last_change_updated
                    && edit.updated_at == self.inline.change_count
            }
            None => {
                let rope = self.model().read(cx).buffer().rope().clone();
                let line_text = |row: u32| rope.line(row);
                self.current_edit(item, cx).is_some_and(|edit| {
                    edit.range.start <= position
                        && position <= edit.range.end
                        && is_visible(&edit, item.original_start, position, &line_text)
                })
            }
        }
    }

    /// Whether a list from a provider yielded to has a completion visible
    /// at `position`, or one that isn't plain text (Monaco blocks then).
    fn yielded_list_visible(
        &self,
        lists: &[Rc<ListRef>],
        position: PointUtf16,
        cx: &gpui::App,
    ) -> bool {
        let rope = self.model().read(cx).buffer().rope().clone();
        let line = rope.line_len_utf16(position.row);
        let word_start = self.word_start_at(position);
        let line_text = |row: u32| self.snapshot.line(row);
        lists.iter().flat_map(|list| &list.list.items).any(|item| {
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
            text: item.insert_text.clone(),
        })
    }

    /// Whether `position` is in the item's `show_range` (Monaco
    /// `showRange`), which follows edits.
    fn item_shows_at(&self, item: &Item, position: PointUtf16, cx: &gpui::App) -> bool {
        let Some((range, version)) = &item.show_range else {
            return true;
        };
        let buffer = self.model().read(cx).buffer();
        let range = if buffer.version() == *version {
            range.clone()
        } else {
            let patch = buffer.edits_since(*version);
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
    /// `inlineEditState`), in its target model's coordinates (this model's
    /// unless it's a cross-file edit). It's the last inline edit, shown
    /// unless a completion is visible at the cursor, a peek view is open, or
    /// a snippet is active (with `suppress_in_snippet_mode`). A jump or a
    /// hint has no replacements.
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
        let model = match &edit.target {
            Some((model, _)) => model.clone(),
            None => self.model().clone(),
        };
        let buffer = model.read(cx).buffer();
        // A cross-file edit doesn't follow its model: it shows as it was.
        if edit.target.is_none() && edit.version != buffer.version() {
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

    /// The inline edit item's parts, if `ix` is one.
    pub(crate) fn inline_edit_item(&self, ix: usize) -> Option<&InlineEditItem> {
        self.inline.items.get(ix)?.edit.as_ref()
    }

    /// Whether the inline edit at `ix` edits another model (Monaco
    /// `!originalTextRef.targets(textModel)`).
    pub(crate) fn inline_edit_is_cross_file(&self, ix: usize) -> bool {
        self.inline_edit_item(ix)
            .is_some_and(|edit| edit.target.is_some())
    }

    /// The inline edit's jump target, in its target model (Monaco's
    /// `jumpTo` action).
    pub(crate) fn inline_edit_jump_to(&self, ix: usize, cx: &gpui::App) -> Option<PointUtf16> {
        let edit = self.inline_edit_item(ix)?;
        let offset = edit.jump_to?;
        let model = match &edit.target {
            Some((model, _)) => model.clone(),
            None => self.model().clone(),
        };
        let rope = model.read(cx).buffer().rope().clone();
        Some(rope.offset_to_point_utf16(offset.min(rope.len())))
    }

    /// The inline edit's hint: its range in this model and the hint (Monaco
    /// `hint`).
    pub(crate) fn inline_edit_hint(
        &self,
        ix: usize,
        cx: &gpui::App,
    ) -> Option<(Range<PointUtf16>, InlineCompletionHint)> {
        let (range, _, hint) = self.inline_edit_item(ix)?.hint.as_ref()?;
        let rope = self.model().read(cx).buffer().rope().clone();
        Some((
            rope.offset_to_point_utf16(range.start.min(rope.len()))
                ..rope.offset_to_point_utf16(range.end.min(rope.len())),
            hint.clone(),
        ))
    }

    /// Monaco `targetRange`: the hint's range, the jump target, or the range
    /// from the first change to the last. `None` for a cross-file edit
    /// without a hint, which has none in this model.
    pub(crate) fn inline_edit_target(
        &self,
        ix: usize,
        edits: &[(Range<PointUtf16>, String)],
        cx: &gpui::App,
    ) -> Option<Range<PointUtf16>> {
        if let Some((range, _)) = self.inline_edit_hint(ix, cx) {
            return Some(range);
        }
        if self.inline_edit_is_cross_file(ix) {
            return None;
        }
        if let Some(at) = self.inline_edit_jump_to(ix, cx) {
            return Some(at..at);
        }
        Some(edits.first()?.0.start..edits.last()?.0.end)
    }

    /// The shown inline edit's target range, for tests.
    #[doc(hidden)]
    pub fn inline_edit_target_for_tests(&self, cx: &gpui::App) -> Option<Range<PointUtf16>> {
        let (ix, edits) = self.inline_edit_state(cx)?;
        self.inline_edit_target(ix, &edits, cx)
    }

    /// The shown inline edit's replacements as (replaced text, new text),
    /// for tests.
    #[doc(hidden)]
    pub fn inline_edit_for_tests(&self, cx: &gpui::App) -> Option<Vec<(String, String)>> {
        let (ix, edits) = self.inline_edit_state(cx)?;
        let model = match self
            .inline_edit_item(ix)
            .and_then(|edit| edit.target.clone())
        {
            Some((model, _)) => model,
            None => self.model().clone(),
        };
        let rope = model.read(cx).buffer().rope().clone();
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
            .and_then(|(ix, edits)| self.inline_edit_target(ix, &edits, cx))
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
    /// it, until it's jumped to or while accepting. Never for a hint or a
    /// jump.
    pub(crate) fn inline_edit_show_collapsed(&self, cx: &gpui::App) -> bool {
        let Some((ix, _)) = self.inline_edit_state(cx) else {
            return false;
        };
        let item = &self.inline.items[ix];
        let Some(edit) = item.edit.as_ref() else {
            return false;
        };
        if edit.hint.is_some() || edit.jump_to.is_some() {
            return false;
        }
        let current = edit.target.is_some() || edit.updated_at == self.inline.change_count;
        (self.options.inline_suggest.edits.show_collapsed || !current)
            && self.inline.jumped_to != Some(item.id)
            && !self.inline.in_accept_flow
    }

    /// Whether the inline edit's lines are all on screen (Monaco
    /// `isSuggestionInViewport`).
    pub(crate) fn inline_edit_in_viewport(
        &self,
        target: &Range<PointUtf16>,
        cx: &gpui::App,
    ) -> bool {
        let visible = self.visible_ranges(cx);
        match (visible.first(), visible.last()) {
            (Some(first), Some(last)) => first.start <= target.start && target.end <= last.end,
            _ => false,
        }
    }

    /// Whether Tab jumps to the inline edit (Monaco
    /// `tabShouldJumpToInlineEdit`): it's a jump, it shows collapsed, or the
    /// cursor isn't at it.
    pub fn tab_should_jump_to_inline_edit(&self, cx: &gpui::App) -> bool {
        if self.tab_should_indent(cx) {
            return false;
        }
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            return false;
        };
        if self.inline_edit_jump_to(ix, cx).is_some() {
            return true;
        }
        if self.inline_edit_show_collapsed(cx) {
            return true;
        }
        if self.inline.in_accept_flow
            && self
                .inline_edit_target(ix, &edits, cx)
                .is_some_and(|target| self.inline_edit_in_viewport(&target, cx))
        {
            return false;
        }
        !self.cursor_at_inline_edit(cx)
    }

    /// Whether Tab accepts the inline edit (Monaco
    /// `tabShouldAcceptInlineEdit`): the cursor is at it, it was jumped to,
    /// or it follows an accepted one on screen. Never for a jump.
    pub fn tab_should_accept_inline_edit(&self, cx: &gpui::App) -> bool {
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            return false;
        };
        if self.inline_edit_jump_to(ix, cx).is_some() {
            return false;
        }
        let Some(target) = self.inline_edit_target(ix, &edits, cx) else {
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
    /// and reveals it. Tab then accepts it. A jump (Monaco's `jumpTo`
    /// action) is accepted by jumping; a cross-file edit is accepted, which
    /// moves it to its model's editor.
    pub fn jump_to_next_inline_edit(&mut self, cx: &mut Context<Self>) {
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            return;
        };
        if self.inline_edit_is_cross_file(ix) {
            self.accept_inline_edit(false, cx);
            return;
        }
        let Some(target) = self.inline_edit_target(ix, &edits, cx) else {
            return;
        };
        let jump = self.inline_edit_jump_to(ix, cx).is_some();
        let has_hint = self.inline_edit_hint(ix, cx).is_some();
        let id = self.inline.items[ix].id;
        if jump {
            // Monaco: stops and reports the jump as accepted.
            self.inline.updating = true;
            self.stop_inline_completions(StopReason::Accepted(ix, false), cx);
            self.inline.updating = false;
        } else {
            self.inline.items[ix].jumped = true;
        }
        self.inline.jumped_to = Some(id);
        self.inline.updating = true;
        self.set_selections(&[Selection::collapsed(target.start)], cx);
        self.inline.updating = false;
        let single_line = target.start.row == target.end.row
            && (has_hint || !edits.iter().any(|(_, text)| text.contains('\n')));
        if single_line || jump {
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
        // Monaco's `dontRefetchSignal`: a request for the old position
        // satisfies the next trigger.
        let version = self.model().read(cx).buffer().version();
        self.inline.requested = Some((
            version,
            target.start,
            InlineCompletionTriggerKind::Automatic,
        ));
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
    /// shown, once. When a new one starts showing, Monaco pushes an undo
    /// stop and remembers the text's alternative version (to ask for an
    /// inline edit again when undoing back to it).
    pub(crate) fn inline_edit_did_show(&mut self, ix: usize, cx: &mut Context<Self>) {
        let item = &mut self.inline.items[ix];
        item.shown = true;
        if self.inline.shown.insert(format!("edit:{}", item.id)) {
            // Monaco passes the edit's text, none for a jump.
            let text = match &item.edit {
                Some(edit) if edit.jump_to.is_some() => "",
                _ => item.insert_text.as_str(),
            };
            item.provider.handle_item_did_show(&item.completion, text);
            run_shown_command(&item.completion, cx);
            let alternative_version = self.model().update(cx, |model, _| {
                model.push_stack_element();
                model.alternative_version_id()
            });
            self.inline.last_shown = Some(alternative_version);
        }
    }

    /// Whether the shown inline edit has an alternative action (Monaco
    /// `inlineSuggestionAlternativeActionVisible`).
    pub fn inline_suggestion_alternative_action_visible(&self, cx: &gpui::App) -> bool {
        self.inline_edit_state(cx).is_some_and(|(ix, _)| {
            self.inline.items[ix]
                .completion
                .alternative_action
                .is_some()
        })
    }

    /// Runs the shown inline edit's alternative action instead of its edit
    /// (Monaco `editor.action.inlineSuggest.commitAlternativeAction`,
    /// Shift+Tab).
    pub fn accept_inline_suggestion_alternative_action(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.inline_suggestion_alternative_action_visible(cx) {
            return false;
        }
        self.accept_inline_edit(true, cx)
    }

    /// Accepts the inline edit (Monaco `accept` for an `inlineEdit` state):
    /// applies its replacements and the item's other edits, puts the cursor
    /// after the last replacement (not for a hint) and fades the new text
    /// out. With `alternative`, runs its alternative action instead. A jump
    /// only ends; a cross-file edit moves to its model's editor.
    fn accept_inline_edit(&mut self, alternative: bool, cx: &mut Context<Self>) -> bool {
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            return false;
        };
        let item = self.inline.items[ix].clone();
        let Some(edit) = item.edit.clone() else {
            return false;
        };
        let alternative = alternative && item.completion.alternative_action.is_some();
        self.inline.updating = true;
        self.stop_inline_completions(StopReason::Accepted(ix, alternative), cx);
        let mut follow_up_trigger = false;
        if let Some((model, uri)) = edit.target.clone() {
            // Monaco opens the target's editor and moves the item there
            // (`transplantCompletion`).
            let range = edits
                .first()
                .map_or(PointUtf16::default()..PointUtf16::default(), |e| {
                    e.0.clone()
                });
            self.transplant_inline_edit(item.clone(), model, uri, range, cx);
        } else if alternative {
            follow_up_trigger = true;
            if let Some(action) = &item.completion.alternative_action
                && let Err(err) = crate::commands::execute_command(cx, &action.command)
            {
                log::warn!("{err:#}");
            }
        } else if let Some(((first_range, first_text), rest)) = edits.split_first() {
            self.model()
                .update(cx, |model, _| model.push_stack_element());
            let new = new_ranges(&edits);
            let cursor = new.last().map(|range| range.end);
            let mut others = rest.to_vec();
            others.extend(item.completion.additional_text_edits.iter().cloned());
            self.replace_text(first_range.clone(), first_text, &others, cx);
            // Monaco leaves the cursor alone for an edit shown elsewhere.
            if let Some(cursor) = cursor
                && edit.hint.is_none()
            {
                self.set_selections(&[Selection::collapsed(cursor)], cx);
            }
            self.model()
                .update(cx, |model, _| model.push_stack_element());
            self.start_inline_edit_fade_out(&new, cx);
        }
        self.inline.updating = false;
        // Set after the edit, which resets it.
        self.inline.in_accept_flow = true;
        self.inline.last_accepted = Some((self.model().read(cx).buffer().version(), true));
        if let Some(command) = &item.completion.command
            && let Err(err) = crate::commands::execute_command(cx, command)
        {
            log::warn!("{err:#}");
        }
        if follow_up_trigger {
            self.trigger_inline_completions(cx);
        }
        true
    }

    /// Monaco `FadeoutDecoration`: highlights `ranges` (the accepted text)
    /// with `inlineEdit.modifiedChangedTextBackground`, fading out with
    /// `easeOutCubic` over a second.
    fn start_inline_edit_fade_out(&mut self, ranges: &[Range<PointUtf16>], cx: &mut Context<Self>) {
        self.clear_inline_edit_fade_out(cx);
        // Monaco skips it when motion is reduced.
        if cx.reduce_motion() {
            return;
        }
        let rope = self.model().read(cx).buffer().rope().clone();
        let color = self
            .theme
            .colors
            .inline_edit_modified_changed_text_background();
        let options = move |opacity: f32| {
            let mut background = color;
            background.a *= opacity;
            DecorationOptions {
                background: Some(background),
                z_index: 1,
                ..Default::default()
            }
        };
        let decorations: Vec<_> = ranges
            .iter()
            .filter(|range| range.start != range.end)
            .map(|range| {
                let start = rope
                    .point_utf16_to_offset(rope.clip_point_utf16(range.start, text::Bias::Left));
                let end =
                    rope.point_utf16_to_offset(rope.clip_point_utf16(range.end, text::Bias::Left));
                (start..end.max(start), options(1.))
            })
            .collect();
        if decorations.is_empty() {
            return;
        }
        let owner = Some(cx.entity_id());
        let ids = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &[], decorations, cx)
        });
        let started = cx.background_executor().now();
        let task = cx.spawn({
            let ids = ids.clone();
            async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let elapsed = cx.background_executor().now() - started;
                    let progress =
                        (elapsed.as_secs_f32() / FADE_OUT_DURATION.as_secs_f32()).min(1.);
                    let done = this
                        .update(cx, |this, cx| {
                            if progress >= 1. {
                                this.clear_inline_edit_fade_out(cx);
                                return true;
                            }
                            let opacity = 1. - crate::view::animation::ease_out_cubic(progress);
                            this.model().update(cx, |model, cx| {
                                for id in &ids {
                                    model.change_decoration_options(*id, options(opacity), cx);
                                }
                            });
                            false
                        })
                        .unwrap_or(true);
                    if done {
                        break;
                    }
                }
            }
        });
        self.inline.fade_out = Some((ids, task));
    }

    fn clear_inline_edit_fade_out(&mut self, cx: &mut Context<Self>) {
        if let Some((ids, _task)) = self.inline.fade_out.take() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &ids, Vec::new(), cx)
            });
        }
    }

    /// The accepted inline edit's fading highlight: its ranges and opacity,
    /// for tests.
    #[doc(hidden)]
    pub fn inline_edit_fade_out_for_tests(&self, cx: &gpui::App) -> Vec<(Range<PointUtf16>, f32)> {
        let Some((ids, _)) = &self.inline.fade_out else {
            return Vec::new();
        };
        let model = self.model().read(cx);
        let rope = model.buffer().rope().clone();
        let full = self
            .theme
            .colors
            .inline_edit_modified_changed_text_background()
            .a;
        ids.iter()
            .filter_map(|id| model.decoration(*id))
            .map(|decoration| {
                let alpha = decoration.options.background.map_or(0., |b| b.a);
                (
                    rope.offset_to_point_utf16(decoration.range.start)
                        ..rope.offset_to_point_utf16(decoration.range.end),
                    if full > 0. { alpha / full } else { 0. },
                )
            })
            .collect()
    }

    /// Monaco's cross-file accept: opens `model`'s editor (emitting
    /// [`CodeEditorEvent::OpenLocation`](crate::CodeEditorEvent::OpenLocation)
    /// for the host) and moves `item` there (`transplantCompletion`). If no
    /// editor shows the model yet, the next one to show it takes the item.
    fn transplant_inline_edit(
        &mut self,
        mut item: Item,
        model: gpui::Entity<TextModel>,
        uri: SharedString,
        range: Range<PointUtf16>,
        cx: &mut Context<Self>,
    ) {
        if let Some(edit) = item.edit.as_mut() {
            edit.target = None;
            // A hint is in this model, not the target's.
            edit.hint = None;
        }
        item.show_range = None;
        let me = cx.entity_id();
        let editor = crate::registry::editors(cx)
            .into_iter()
            .find(|editor| editor.entity_id() != me && editor.read(cx).model() == &model);
        match editor {
            Some(editor) => {
                editor.update(cx, |editor, cx| editor.receive_inline_edit(item, cx));
            }
            None => {
                cx.default_global::<PendingInlineEdits>()
                    .0
                    .insert(uri.clone(), item);
            }
        }
        self.open_location(
            &Location {
                uri: Some(uri),
                range: range.start..range.start,
                ..Default::default()
            },
            false,
            cx,
        );
    }

    /// Monaco `transplantCompletion`: shows an inline edit accepted in
    /// another editor, active and in the accept flow, without asking again.
    fn receive_inline_edit(&mut self, mut item: Item, cx: &mut Context<Self>) {
        let model = self.model().clone();
        let buffer = model.read(cx).buffer();
        if let Some(edit) = item.edit.as_mut()
            && !follow_inline_edit(edit, buffer, self.inline.change_count)
        {
            return;
        }
        let version = buffer.version();
        let target = edits_target_start(&item, buffer);
        for old in std::mem::take(&mut self.inline.items) {
            old.provider.handle_end_of_lifetime(
                &old.completion,
                &InlineCompletionEndOfLifeReason::Ignored {
                    superseded_by: None,
                    user_typing_disagreed: false,
                },
            );
        }
        if let Some(edit) = item.edit.as_mut() {
            edit.updated_at = self.inline.change_count;
        }
        self.inline.items = vec![item];
        self.inline.active = true;
        self.inline.in_accept_flow = true;
        // Monaco `dontRefetchSignal`.
        self.inline.requested = Some((
            version,
            self.cursors.primary().position,
            InlineCompletionTriggerKind::Automatic,
        ));
        if let Some(target) = target {
            self.reveal_range(target..target, crate::RevealType::Center, cx);
        }
        cx.notify();
    }

    /// Takes a cross-file inline edit accepted for this editor's model
    /// while no editor showed it.
    pub(crate) fn take_pending_inline_edit(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<PendingInlineEdits>() {
            return;
        }
        let Some(uri) = self.model().read(cx).uri().cloned() else {
            return;
        };
        let Some(item) = cx.global_mut::<PendingInlineEdits>().0.remove(&uri) else {
            return;
        };
        self.receive_inline_edit(item, cx);
    }

    /// The visible items (deduplicated by their edit), the selected index,
    /// and its ghost text.
    pub(crate) fn inline_state(&self, cx: &gpui::App) -> Option<InlineView> {
        // The suggest widget's preview shows instead, unless Monaco's
        // `showOnSuggestConflict` says otherwise.
        if self.is_suggest_visible() && !self.inline_shows_on_suggest_conflict(cx) {
            return None;
        }
        self.inline_state_unchecked(cx)
    }

    /// [`Self::inline_state`] whether or not the suggest widget shows.
    fn inline_state_unchecked(&self, cx: &gpui::App) -> Option<InlineView> {
        if !self.inline.active
            || !self.options.inline_suggest.enabled
            || self.marked_range.is_some()
        {
            return None;
        }
        if self.options.inline_suggest.suppress_in_snippet_mode && self.in_snippet_mode() {
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
    /// selected item as ghost text, not italic, extended by an inline
    /// completion asked with that selection that starts with it (Monaco
    /// `_computeAugmentation`, which shows even without `suggest.preview`).
    fn suggest_preview(&self, cx: &gpui::App) -> Option<GhostText> {
        self.suggest_preview_edit(cx).map(|(_, ghost, _)| ghost)
    }

    /// [`Self::suggest_preview`] with the edit it shows and the length of
    /// its previewed part.
    fn suggest_preview_edit(&self, cx: &gpui::App) -> Option<(TextReplacement, GhostText, usize)> {
        if !self.is_suggest_visible() || self.inline_shows_on_suggest_conflict(cx) {
            return None;
        }
        let (range, text) = self.selected_suggestion_edit()?;
        let line_text = |row: u32| self.snapshot.line(row);
        let suggest_edit =
            single_text_remove_common_prefix(&TextReplacement { range, text }, &line_text);
        let augmentation = self.inline_suggest_augmentation(&suggest_edit, cx);
        if !self.options.suggest.preview && augmentation.is_none() {
            return None;
        }
        let edit = augmentation.map_or(suggest_edit, |(_, edit)| edit);
        let length = edit.text.encode_utf16().count();
        let ghost = compute_ghost_text(
            &edit,
            &line_text,
            Self::ghost_mode(self.options.suggest.preview_mode),
            Some(self.cursors.primary().position),
            length,
        )?;
        Some((edit, ghost, length))
    }

    /// Whether a request is in flight (Monaco's `status` is `loading`).
    pub(crate) fn inline_completions_loading(&self) -> bool {
        self.inline.task.is_some()
    }

    /// Monaco `stop('automatic')`: hides the ghost text without rejecting
    /// it.
    pub(crate) fn stop_inline_completions_automatically(&mut self, cx: &mut Context<Self>) {
        self.stop_inline_completions(StopReason::Automatic, cx);
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
            .or_else(|| self.suggest_preview(cx))?;
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

    /// Monaco `syntaxHighlightingEnabled` with `computeGhostTextViewData`:
    /// the injected texts and the additional lines of `ghost` (laid out as
    /// `data`), split into pieces colored as code (at 70% opacity), or one
    /// plain ghost-colored piece each. Monaco tokenizes the line with the
    /// injected texts inserted, then the additional lines, from the line's
    /// state; this parses them on their own. Text of the line moved below
    /// keeps the line's colors (Monaco `additionalLinesOriginalSuffix`).
    fn ghost_view_segments(
        &self,
        ghost: &GhostText,
        data: &GhostTextViewData,
        style: impl Fn(bool) -> TokenStyle,
        cx: &gpui::App,
    ) -> (Vec<GhostSegments>, Vec<Vec<GhostSegment>>) {
        let line = self.snapshot.line(ghost.line);
        let byte = |column: u32| super::inline_edits::utf16_to_byte(&line, column);
        let highlight = self.options.inline_suggest.syntax_highlighting_enabled;
        let moves_text = data
            .additional_lines
            .iter()
            .flatten()
            .any(|piece| matches!(piece, GhostLinePiece::Buffer(_)));
        // The line with the injected texts, then the additional lines.
        let mut text = String::new();
        let mut inline_starts = Vec::new();
        let mut taken = 0;
        for (column, inline, _) in &data.inline_texts {
            let at = byte(*column).max(taken);
            text.push_str(&line[taken..at]);
            taken = at;
            inline_starts.push(text.len());
            text.push_str(inline);
        }
        text.push_str(&line[taken..]);
        // The line's text that moves is after every injected text.
        let inserted: usize = data.inline_texts.iter().map(|(_, t, _)| t.len()).sum();
        let mut piece_starts = Vec::new();
        for pieces in &data.additional_lines {
            text.push('\n');
            let mut starts = Vec::new();
            for piece in pieces {
                starts.push(text.len());
                match piece {
                    GhostLinePiece::Ghost(ghost) => text.push_str(ghost),
                    GhostLinePiece::Buffer(columns) => {
                        text.push_str(&line[byte(columns.start)..byte(columns.end)])
                    }
                }
            }
            piece_starts.push(starts);
        }
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let spans = language
            .filter(|_| highlight || moves_text)
            .and_then(|language| {
                let registry = crate::model::language_registry(cx);
                crate::colorize::capture_spans(&text, &language, &registry)
            });
        let colored = |piece: &str, start: usize, ghost: bool, preview: bool| -> GhostSegments {
            let Some(spans) = spans.as_ref().filter(|_| highlight || !ghost) else {
                let style = if ghost {
                    style(preview)
                } else {
                    TokenStyle::default()
                };
                return vec![(piece.to_string(), style)];
            };
            split_by_spans(piece, start, spans)
                .into_iter()
                .map(|(piece, id)| {
                    let token = id.map(|id| self.theme.style_for(id)).unwrap_or_default();
                    let style = if ghost {
                        TokenStyle {
                            italic: !preview,
                            opacity: Some(0.7),
                            ..token
                        }
                    } else {
                        token
                    };
                    (piece, style)
                })
                .collect()
        };
        let inline = data
            .inline_texts
            .iter()
            .zip(&inline_starts)
            .map(|((_, inline, preview), &start)| colored(inline, start, true, *preview))
            .collect();
        let additional = data
            .additional_lines
            .iter()
            .zip(&piece_starts)
            .map(|(pieces, starts)| {
                pieces
                    .iter()
                    .zip(starts)
                    .flat_map(|(piece, &start)| {
                        let (pieces, ghost) = match piece {
                            GhostLinePiece::Ghost(ghost) => {
                                (colored(ghost, start, true, false), true)
                            }
                            GhostLinePiece::Buffer(columns) => {
                                let from = byte(columns.start);
                                (
                                    colored(
                                        &line[from..byte(columns.end)],
                                        from + inserted,
                                        false,
                                        false,
                                    ),
                                    false,
                                )
                            }
                        };
                        pieces.into_iter().map(move |(text, style)| GhostSegment {
                            text,
                            style,
                            ghost,
                        })
                    })
                    .collect()
            })
            .collect();
        (inline, additional)
    }

    /// The ghost texts at the other cursors (Monaco `getSecondaryEdits`
    /// with `computeGhostText` at each cursor): `primary`, the edit at the
    /// primary cursor, repeated where it fits, with its ghost text, for the
    /// cursors where it shows as one.
    fn secondary_ghost_texts(
        &self,
        primary: &TextReplacement,
        mode: GhostTextMode,
        preview_length: usize,
    ) -> Vec<(TextReplacement, GhostText)> {
        let positions: Vec<PointUtf16> = self
            .cursors
            .all()
            .iter()
            .map(|cursor| cursor.position)
            .collect();
        let line_text = |row: u32| self.snapshot.line(row);
        secondary_edits(self.snapshot.rope(), &positions, primary)
            .into_iter()
            .zip(&positions[positions.len().min(1)..])
            .filter_map(|(edit, &position)| {
                let edit = edit?;
                let ghost =
                    compute_ghost_text(&edit, &line_text, mode, Some(position), preview_length)?;
                Some((edit, ghost))
            })
            .collect()
    }

    /// Every ghost text shown, the primary cursor's first, with the edit it
    /// shows (Monaco `ghostTexts` and the state's `edits`): the selected
    /// completion's, else the suggest widget's preview. The item shown is
    /// `None` for the preview without an inline completion extending it.
    fn ghost_texts(&self, cx: &gpui::App) -> Option<(Option<usize>, GhostTexts)> {
        if let Some((visible, selected, ghost)) = self.inline_state(cx) {
            let (ix, edit) = visible[selected].clone();
            let mode = Self::ghost_mode(self.options.inline_suggest.mode);
            let secondary = self.secondary_ghost_texts(&edit, mode, 0);
            let mut ghosts = vec![(edit, ghost)];
            ghosts.extend(secondary);
            return Some((Some(ix), ghosts));
        }
        let (edit, ghost, preview_length) = self.suggest_preview_edit(cx)?;
        let mode = Self::ghost_mode(self.options.suggest.preview_mode);
        let secondary = self.secondary_ghost_texts(&edit, mode, preview_length);
        let mut ghosts = vec![(edit, ghost)];
        ghosts.extend(secondary);
        Some((None, ghosts))
    }

    /// Renders the ghost text before painting.
    pub(crate) fn refresh_ghost_text(&mut self, cx: &mut Context<Self>) {
        self.take_pending_inline_edit(cx);
        self.refresh_inline_suggest_item(cx);
        // The jump resets once the cursor leaves the edit (Monaco
        // `jumpToReset`).
        if self.inline.jumped_to.is_some() && !self.cursor_at_inline_edit(cx) {
            self.inline.jumped_to = None;
        }
        self.refresh_inline_edit(cx);
        let state = self.inline_state(cx);
        self.refresh_inline_toolbar(state.as_ref().map(|(_, _, ghost)| ghost), cx);
        let ghosts = self.ghost_texts(cx);
        let version = self.model().read(cx).buffer().version();
        let cursors: Vec<PointUtf16> = self
            .cursors
            .all()
            .iter()
            .map(|cursor| cursor.position)
            .collect();
        let key = ghosts.as_ref().map(|(shown, ghosts)| {
            let multiline = ghosts
                .iter()
                .any(|(_, ghost)| ghost.parts.iter().any(|p| p.lines.len() > 1));
            (version, cursors, shown.unwrap_or(usize::MAX), multiline)
        });
        if key.is_some() && self.inline.rendered.as_ref().map(|r| &r.key) == key.as_ref() {
            return;
        }
        self.clear_ghost_text(cx);
        let (Some(key), Some((shown, ghosts))) = (key, ghosts) else {
            return;
        };
        let visible_color = |c: gpui::Hsla| (c.a > 0.).then_some(c);
        let ghost_foreground = self.theme.colors.ghost_text_foreground;
        let ghost_background = visible_color(self.theme.colors.ghost_text_background);
        let ghost_border = visible_color(self.theme.colors.ghost_text_border);
        let style = |preview: bool| TokenStyle {
            foreground: Some(ghost_foreground),
            italic: !preview,
            ..Default::default()
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        // Monaco `USE_SQUIGGLES_FOR_WARNING`: ghost text with a warning gets
        // the warning squiggle, on every line.
        let warning_color = shown
            .and_then(|ix| self.inline.items[ix].completion.warning.as_ref())
            .map(|_| {
                self.theme
                    .colors
                    .marker_color(crate::MarkerSeverity::Warning)
            });
        let warning = warning_color
            .map(|color| crate::decorations::DecorationUnderline { color, wavy: true });
        let font_family = self.options.inline_suggest.font_family.clone();
        let mut decorations = Vec::new();
        let mut zones = Vec::new();
        let mut hidden = Vec::new();
        for (_, ghost) in &ghosts {
            let line = self.snapshot.line(ghost.line);
            let data = compute_ghost_text_view_data(ghost, &line);
            let (inline, additional) = self.ghost_view_segments(ghost, &data, style, cx);
            for ((column, _, _), segments) in data.inline_texts.iter().zip(inline) {
                let offset = rope.point_utf16_to_offset(PointUtf16::new(ghost.line, *column));
                // One injected text per piece, in order.
                for (text, style) in segments {
                    decorations.push((
                        offset..offset,
                        DecorationOptions {
                            after: Some(InjectedText {
                                content: text.into(),
                                style,
                                background: ghost_background,
                                swatch: None,
                                font_family: font_family.clone(),
                                underline: warning,
                                // Monaco `GhostTextView`: the cursor stays
                                // before the suggestion.
                                cursor_stops: crate::InjectedTextCursorStops::Left,
                                ..Default::default()
                            }),
                            border: ghost_border,
                            show_if_collapsed: true,
                            ..Default::default()
                        },
                    ));
                }
            }
            if let Some(range) = &data.hidden_range {
                hidden.push((ghost.line, range.start));
            }
            if additional.is_empty() {
                continue;
            }
            let foreground = self.theme.colors.foreground;
            let count = additional.len();
            let view = cx.new(|_| GhostLines {
                lines: additional,
                foreground,
                font: self.options.font_family.clone(),
                ghost_font: font_family.clone(),
                font_size: self.options.font_size,
                line_height: self.options.resolved_line_height(),
                left: px(0.),
                warning: warning_color,
            });
            let id = self.add_view_zone(
                ViewZone::new(
                    ghost.line + 1,
                    ZoneHeight::Lines(count as f32),
                    view.clone().into(),
                ),
                cx,
            );
            zones.push((ghost.line, id, view));
        }
        let owner = Some(cx.entity_id());
        let ids = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &[], decorations, cx)
        });
        // Monaco `handleItemDidShow`, once per item.
        if let Some(ix) = shown {
            self.inline.items[ix].shown = true;
            let item = &self.inline.items[ix];
            // Once per item: an equal item from a newer request keeps its
            // identity (Monaco `_didShow`).
            if self.inline.shown.insert(format!("item:{}", item.id)) {
                item.provider
                    .handle_item_did_show(&item.completion, &item.insert_text);
                run_shown_command(&item.completion, cx);
            }
        }
        self.inline.rendered = Some(Rendered {
            key,
            decorations: ids,
            zones,
            hidden,
        });
    }

    /// Monaco `ghost-text-hidden`: the rows whose rest moved below their
    /// ghost text, with the column each shows up to.
    pub(crate) fn ghost_text_hidden_rows(&self) -> Vec<(u32, u32)> {
        self.inline
            .rendered
            .as_ref()
            .map_or_else(Vec::new, |rendered| rendered.hidden.clone())
    }

    /// Monaco `ghost-text-hidden`: the column `row` shows up to, when ghost
    /// text moved the rest of it below.
    pub(crate) fn ghost_text_hidden_from(&self, row: u32) -> Option<u32> {
        self.inline
            .rendered
            .as_ref()?
            .hidden
            .iter()
            .find(|(hidden_row, _)| *hidden_row == row)
            .map(|(_, column)| *column)
    }

    /// Every ghost text shown, the primary cursor's first, as (row, parts)
    /// like [`Self::ghost_text_for_tests`], for tests.
    #[doc(hidden)]
    pub fn ghost_texts_for_tests(&self, cx: &gpui::App) -> Vec<(u32, Vec<(u32, String)>)> {
        self.ghost_texts(cx)
            .map(|(_, ghosts)| {
                ghosts
                    .into_iter()
                    .map(|(_, ghost)| {
                        (
                            ghost.line,
                            ghost.parts.iter().map(|p| (p.column, p.text())).collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How the ghost texts are drawn, for tests: each one's row, the column
    /// its line is hidden from, and its additional lines as text with
    /// whether each line's suggested text has the warning squiggle.
    #[doc(hidden)]
    #[allow(clippy::type_complexity)]
    pub fn ghost_text_view_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Vec<(u32, Option<u32>, Vec<(String, bool)>)> {
        let Some(rendered) = &self.inline.rendered else {
            return Vec::new();
        };
        let mut rows: Vec<u32> = self
            .ghost_texts(cx)
            .map(|(_, ghosts)| ghosts.iter().map(|(_, ghost)| ghost.line).collect())
            .unwrap_or_default();
        rows.dedup();
        rows.into_iter()
            .map(|row| {
                let hidden = self.ghost_text_hidden_from(row);
                let lines = rendered
                    .zones
                    .iter()
                    .filter(|(zone_row, _, _)| *zone_row == row)
                    .flat_map(|(_, _, view)| {
                        let view = view.read(cx);
                        view.lines
                            .iter()
                            .map(|line| {
                                let text: String = line.iter().map(|s| s.text.as_str()).collect();
                                let squiggled = view.warning.is_some()
                                    && line.iter().any(|segment| segment.ghost);
                                (text, squiggled)
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();
                (row, hidden, lines)
            })
            .collect()
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
                allow_editor_overflow: true,
                ..ContentWidget::new(
                    position,
                    vec![
                        ContentWidgetPreference::Above,
                        ContentWidgetPreference::Below,
                    ],
                    view.clone().into(),
                )
            },
            cx,
        );
        self.widgets.set_layer(id, crate::widgets::layer::WIDGET);
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

    /// The shown ghost text's warning message, and whether the ghost text
    /// is drawn with the warning squiggle, for tests.
    #[doc(hidden)]
    pub fn inline_warning_for_tests(&self, cx: &gpui::App) -> Option<(String, bool)> {
        let warning = self.inline_warning(cx)?;
        let model = self.model().read(cx);
        let squiggled = self.inline.rendered.as_ref().is_some_and(|rendered| {
            rendered.decorations.iter().any(|id| {
                model
                    .decoration(*id)
                    .and_then(|d| d.options.after.as_ref())
                    .and_then(|after| after.underline)
                    .is_some_and(|underline| underline.wavy)
            })
        });
        Some((warning.message.as_str().to_string(), squiggled))
    }

    /// Opens the always-shown toolbar's "⋯" menu, which takes the
    /// keyboard, for tests.
    #[doc(hidden)]
    pub fn open_inline_toolbar_menu_for_tests(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((toolbar, _)) = self.inline.toolbar.clone() {
            toolbar.update(cx, |toolbar, cx| toolbar.open_menu_for_tests(window, cx));
        }
    }

    /// Whether the always-shown toolbar's menu is open, and its selected
    /// row, for tests.
    #[doc(hidden)]
    pub fn inline_toolbar_menu_for_tests(&self, cx: &gpui::App) -> Option<Option<usize>> {
        let (toolbar, _) = self.inline.toolbar.as_ref()?;
        toolbar.read(cx).menu_for_tests()
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
        for (_, zone, _) in rendered.zones {
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
            let reason = if let StopReason::Accepted(accepted, alternative_action) = reason
                && accepted == ix
            {
                InlineCompletionEndOfLifeReason::Accepted { alternative_action }
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
        change: &CursorChangeCause,
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
    /// `editor.action.inlineSuggest.commit`, Tab), then asks again (Monaco's
    /// `noDelay` trigger after the command).
    pub fn accept_inline_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let accepted = self.accept_inline(cx);
        if accepted {
            self.trigger_inline_completions_after_accept(cx);
        }
        accepted
    }

    /// Monaco `model.accept()` without the command, as clicking a view or
    /// the gutter indicator does: nothing asks again.
    pub(crate) fn accept_inline(&mut self, cx: &mut Context<Self>) -> bool {
        if self.inline_edit_visible(cx) {
            return self.accept_inline_edit(false, cx);
        }
        let Some((visible, selected, _)) = self.inline_state(cx) else {
            return false;
        };
        let (ix, edit) = visible[selected].clone();
        let item = self.inline.items[ix].clone();
        // Monaco: the edit at every cursor showing the ghost text, and the
        // cursors after each one's text (less what it keeps,
        // `removeTextReplacementCommonSuffixPrefix`). Cursors without it go.
        let mode = Self::ghost_mode(self.options.inline_suggest.mode);
        let mut edits = vec![edit.clone()];
        edits.extend(
            self.secondary_ghost_texts(&edit, mode, 0)
                .into_iter()
                .map(|(edit, _)| edit),
        );
        let cursors = {
            let line_text = |row: u32| self.snapshot.line(row);
            let minimal: Vec<TextReplacement> = edits
                .iter()
                .map(|edit| remove_common_prefix_and_suffix(edit, &line_text))
                .collect();
            end_positions_after_applying(&minimal)
        };
        self.inline.updating = true;
        self.stop_inline_completions(StopReason::Accepted(ix, false), cx);
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
                &item.insert_text,
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
            let mut others: Vec<(Range<PointUtf16>, String)> = edits[1..]
                .iter()
                .map(|edit| (edit.range.clone(), edit.text.clone()))
                .collect();
            others.extend(item.completion.additional_text_edits.iter().cloned());
            self.replace_text(edit.range.clone(), &edit.text, &others, cx);
            let selections: Vec<Selection> =
                cursors.into_iter().map(Selection::collapsed).collect();
            self.set_selections(&selections, cx);
        }
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        self.inline.updating = false;
        self.inline.in_accept_flow = true;
        self.inline.last_accepted = Some((self.model().read(cx).buffer().version(), false));
        if let Some(command) = &item.completion.command
            && let Err(err) = crate::commands::execute_command(cx, command)
        {
            log::warn!("{err:#}");
        }
        true
    }

    /// Accepts the next word of the ghost text (Monaco `acceptNextWord`,
    /// Cmd/Ctrl+Right).
    pub fn accept_next_inline_word(&mut self, cx: &mut Context<Self>) -> bool {
        let regex = self.word_regex(cx);
        self.accept_inline_partially(PartialAcceptTriggerKind::Word, cx, |text| {
            next_word_length(text, &regex)
        })
    }

    /// Accepts the ghost text up to and including the next line break
    /// (Monaco `acceptNextLine`).
    pub fn accept_next_inline_line(&mut self, cx: &mut Context<Self>) -> bool {
        self.accept_inline_partially(PartialAcceptTriggerKind::Line, cx, |text| {
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
        kind: PartialAcceptTriggerKind,
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
        // Monaco: the same at the other cursors where it fits
        // (`getSecondaryEdits`), and every cursor after its edit.
        let primary = TextReplacement {
            range: cursor..ghost_position,
            text: inserted.clone(),
        };
        let positions: Vec<PointUtf16> = self
            .cursors
            .all()
            .iter()
            .map(|cursor| cursor.position)
            .collect();
        let mut edits = vec![primary.clone()];
        edits.extend(
            secondary_edits(self.snapshot.rope(), &positions, &primary)
                .into_iter()
                .flatten(),
        );
        let cursors = end_positions_after_applying(&edits);
        self.inline.updating = true;
        // Monaco `pushUndoStop`: the accepted word undoes on its own.
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        let others: Vec<(Range<PointUtf16>, String)> = edits[1..]
            .iter()
            .map(|edit| (edit.range.clone(), edit.text.clone()))
            .collect();
        self.replace_text(cursor..ghost_position, &inserted, &others, cx);
        let end = end_after(cursor, &inserted);
        let selections: Vec<Selection> = cursors.into_iter().map(Selection::collapsed).collect();
        self.set_selections(&selections, cx);
        self.reveal_range(end..end, crate::RevealType::Simple, cx);
        self.inline.updating = false;
        self.inline.accepted_partially = Some(self.model().read(cx).buffer().version());
        // Monaco counts from the start of the item's range.
        let accepted_length = self
            .current_edit(&item, cx)
            .filter(|edit| edit.range.start <= end)
            .map_or(accepted, |edit| {
                self.text_range(edit.range.start..end)
                    .encode_utf16()
                    .count()
            });
        item.provider.handle_partial_accept(
            &item.completion,
            PartialAcceptInfo {
                kind,
                accepted_length,
            },
        );
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

/// Monaco's `SuggestWidgetAdaptor` and the parts of `InlineCompletionsModel`
/// that follow the suggest widget, the provider's
/// `onDidChangeInlineCompletions`, and `closeBrackets`.
impl CodeEditor {
    /// The suggest widget's selection (Monaco `SuggestItemInfo`) and whether
    /// its list was incomplete; `None` while the widget is closed or has no
    /// focused suggestion.
    fn current_suggest_item(&self) -> Option<(SelectedSuggestionInfo, bool)> {
        if !self.is_suggest_visible() {
            return None;
        }
        let (range, text) = self.selected_suggestion_edit()?;
        let (_, item) = self.selected_suggestion()?;
        Some((
            SelectedSuggestionInfo {
                range,
                text,
                completion_kind: item.kind,
                is_snippet_text: item.insert_text_is_snippet,
            },
            self.selected_suggestion_incomplete(),
        ))
    }

    /// Monaco `_shouldShowOnSuggestConflict`: with
    /// `inline_suggest.experimental.show_on_suggest_conflict`, a visible
    /// inline completion shows instead of the suggest widget's preview.
    pub(crate) fn inline_shows_on_suggest_conflict(&self, cx: &gpui::App) -> bool {
        use crate::options::ShowOnSuggestConflict;
        let mode = self
            .options
            .inline_suggest
            .experimental
            .show_on_suggest_conflict;
        if mode == ShowOnSuggestConflict::Never || self.inline_state_unchecked(cx).is_none() {
            return false;
        }
        match self.current_suggest_item() {
            None => false,
            Some((_, incomplete)) => mode == ShowOnSuggestConflict::Always || incomplete,
        }
    }

    /// The selection a request passes (Monaco `selectedSuggestionInfo`).
    fn inline_request_suggest_item(&self, cx: &gpui::App) -> Option<SelectedSuggestionInfo> {
        if self.inline_shows_on_suggest_conflict(cx) {
            return None;
        }
        self.current_suggest_item().map(|(info, _)| info)
    }

    /// Before painting: follows the suggest widget's selection (Monaco
    /// `onDidSelectedItemChange`). A new selection asks the providers with
    /// it; once there's none, the items asked with it become the items
    /// (Monaco `seedInlineCompletionsWithSuggestWidget`) and, while active,
    /// the providers are asked again.
    pub(crate) fn refresh_inline_suggest_item(&mut self, cx: &mut Context<Self>) {
        let current = self.current_suggest_item();
        if current == self.inline.suggest_item {
            return;
        }
        self.inline.suggest_item = current.clone();
        let enabled = self.options.inline_suggest.enabled && !self.options.read_only;
        if current.is_some() && !self.inline_shows_on_suggest_conflict(cx) {
            if enabled {
                self.fetch_inline_completions(
                    InlineCompletionTriggerKind::Automatic,
                    FetchOptions::default(),
                    cx,
                );
            }
            return;
        }
        self.seed_inline_completions_with_suggest_items();
        if enabled && self.inline.active {
            self.fetch_inline_completions(
                InlineCompletionTriggerKind::Automatic,
                FetchOptions::default(),
                cx,
            );
        }
        cx.notify();
    }

    /// Monaco `seedInlineCompletionsWithSuggestWidget`: the items asked with
    /// the suggest widget's selection replace older items.
    fn seed_inline_completions_with_suggest_items(&mut self) {
        if self.inline.suggest_items.is_empty() {
            return;
        }
        let suggest_items = std::mem::take(&mut self.inline.suggest_items);
        if self.inline.items.is_empty()
            || self.inline.suggest_items_request > self.inline.items_request
        {
            end_items(std::mem::replace(&mut self.inline.items, suggest_items));
            self.inline.items_request = self.inline.suggest_items_request;
        } else {
            end_items(suggest_items);
        }
        self.inline.suggest_items_request = 0;
    }

    /// Monaco `_computeAugmentation`: the first item asked with the suggest
    /// widget's selection whose edit extends `suggest_edit` (the selection's
    /// edit less the text it keeps), with that edit.
    pub(crate) fn inline_suggest_augmentation(
        &self,
        suggest_edit: &TextReplacement,
        cx: &gpui::App,
    ) -> Option<(usize, TextReplacement)> {
        let line_text = |row: u32| self.snapshot.line(row);
        self.inline
            .suggest_items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.edit.is_none())
            .find_map(|(ix, item)| {
                let edit = self.current_edit(item, cx)?;
                // Monaco intersects the range with the suggestion's start
                // to its end before removing the common prefix.
                let edit = if edit.range.start <= suggest_edit.range.end {
                    let restricted = TextReplacement {
                        range: edit.range.start..edit.range.end.min(suggest_edit.range.end),
                        text: edit.text.clone(),
                    };
                    let trimmed = single_text_remove_common_prefix(&restricted, &line_text);
                    TextReplacement {
                        range: trimmed.range.start..edit.range.end,
                        text: trimmed.text,
                    }
                } else {
                    edit
                };
                single_text_edit_augments(&edit, suggest_edit).then_some((ix, edit))
            })
    }

    /// The selector `SuggestWidgetAdaptor` registers with the suggest
    /// controller (`registerSelector`, priority 100): when the suggest
    /// widget shows or refilters its list, the suggestion the selected
    /// inline completion extends is selected; of several, the one inserting
    /// the most. `None` defers to the selection memory.
    pub(crate) fn inline_suggest_preselect(&self, cx: &gpui::App) -> Option<usize> {
        // Monaco `selectedInlineCompletion`, shown or not (the suggest
        // widget hides it).
        let (visible, selected, _) = self.inline_state_unchecked(cx)?;
        let line_text = |row: u32| self.snapshot.line(row);
        let to_preselect = single_text_remove_common_prefix(&visible[selected].1, &line_text);
        let count = self.suggest.session.as_ref()?.model.filtered().len();
        let mut best: Option<(usize, usize)> = None;
        for index in 0..count {
            let Some((range, text)) = self.suggestion_edit(index) else {
                continue;
            };
            let edit =
                single_text_remove_common_prefix(&TextReplacement { range, text }, &line_text);
            let length = edit.text.encode_utf16().count();
            // Monaco `findFirstMax`: the first of the longest.
            if length > 0
                && single_text_edit_augments(&to_preselect, &edit)
                && best.is_none_or(|(_, best)| length > best)
            {
                best = Some((index, length));
            }
        }
        best.map(|(index, _)| index)
    }

    /// Monaco `handleSuggestAccepted`: the suggestion inserting `text` over
    /// `range` is being accepted; the inline completion extending it gets a
    /// partial accept of kind `Suggest`.
    pub(crate) fn inline_suggest_accepted(
        &mut self,
        range: Range<PointUtf16>,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let line_text = |row: u32| self.snapshot.line(row);
        let item_edit =
            single_text_remove_common_prefix(&TextReplacement { range, text }, &line_text);
        let Some((ix, _)) = self.inline_suggest_augmentation(&item_edit, cx) else {
            return;
        };
        let item = &self.inline.suggest_items[ix];
        let Some(edit) = self.current_edit(item, cx) else {
            return;
        };
        let already = self.text_range(edit.range).encode_utf16().count();
        item.provider.handle_partial_accept(
            &item.completion,
            PartialAcceptInfo {
                kind: PartialAcceptTriggerKind::Suggest,
                accepted_length: already + item_edit.text.encode_utf16().count(),
            },
        );
    }

    /// Monaco's handler of a provider's `onDidChangeInlineCompletions` (see
    /// [`crate::providers::refresh_inline_completions`]): only the focused
    /// editor, if `provider` serves it and nothing from another provider
    /// shows, asks `provider` again with `change_hint`; with
    /// `trigger_command_on_provider_change` it asks every provider for
    /// inline edits instead.
    pub(crate) fn inline_provider_changed(
        &mut self,
        provider: Arc<dyn InlineCompletionsProvider>,
        change_hint: Option<InlineCompletionChangeHint>,
        cx: &mut Context<Self>,
    ) {
        if !self.options.inline_suggest.enabled
            || self.options.read_only
            || self.widget_input
            || !self.has_focus
        {
            return;
        }
        let same = |other: &Arc<dyn InlineCompletionsProvider>| {
            std::ptr::addr_eq(Arc::as_ptr(other), Arc::as_ptr(&provider))
        };
        if !self
            .editor_inline_completions_providers(cx)
            .iter()
            .any(same)
        {
            return;
        }
        if self
            .options
            .inline_suggest
            .trigger_command_on_provider_change
        {
            self.inline.active = true;
            self.fetch_inline_completions(
                InlineCompletionTriggerKind::Automatic,
                FetchOptions {
                    force: true,
                    only_inline_edits: true,
                    ..Default::default()
                },
                cx,
            );
            return;
        }
        // Monaco ignores it while another provider's suggestion shows.
        let shown = self
            .inline_state(cx)
            .map(|(visible, selected, _)| visible[selected].0)
            .or_else(|| self.inline_edit_state(cx).map(|(ix, _)| ix));
        if let Some(ix) = shown
            && !same(&self.inline.items[ix].provider)
        {
            return;
        }
        self.inline.active = true;
        self.fetch_inline_completions(
            InlineCompletionTriggerKind::Automatic,
            FetchOptions {
                force: true,
                provider: Some(provider),
                change_hint,
                ..Default::default()
            },
            cx,
        );
    }

    /// Monaco `closeBrackets`: `text` inserted at `start` (replacing the
    /// rest of the line) with the brackets it opens closed and those it
    /// closes without opening removed, by the language's bracket pairs.
    /// Brackets in strings and comments of that line, tokenized on its
    /// own, don't count.
    fn close_inline_brackets(&self, text: &str, start: PointUtf16, cx: &gpui::App) -> String {
        let model = self.model().read(cx);
        let (Some(language), Some(language_id)) = (model.language(), model.language_id()) else {
            return text.to_string();
        };
        let pairs = language.config.brackets().to_vec();
        if pairs.is_empty() {
            return text.to_string();
        }
        let line = self.snapshot.line(start.row);
        let prefix_len = utf16_split(&line, start.column as usize).0.len();
        let proposed = format!("{}{text}", &line[..prefix_len]);
        let registry = crate::model::language_registry(cx);
        let mut skip: Vec<Range<usize>> =
            crate::colorize::capture_spans(&proposed, language_id, &registry)
                .unwrap_or_default()
                .into_iter()
                .filter(|(range, highlight)| {
                    let name = highlight.name();
                    range.end > prefix_len
                        && (name.starts_with("comment")
                            || name.starts_with("string")
                            || name.starts_with("character")
                            || name.contains("regex"))
                })
                .map(|(range, _)| range.start.saturating_sub(prefix_len)..range.end - prefix_len)
                .collect();
        skip.sort_by_key(|range| range.start);
        fix_brackets(text, &pairs, &skip)
    }

    /// The shown ghost text's warning (Monaco `InlineCompletionsModel.warning`).
    pub(crate) fn inline_warning(
        &self,
        cx: &gpui::App,
    ) -> Option<crate::providers::InlineCompletionWarning> {
        let (visible, selected, _) = self.inline_state(cx)?;
        self.inline.items[visible[selected].0]
            .completion
            .warning
            .clone()
    }
}

/// Monaco `fixBracketsInLine` over `text`: a closing bracket closes the
/// nearest open bracket it closes, closing the unclosed ones inside it
/// first; a closing bracket with nothing to close is dropped, and the
/// brackets still open at the end are closed, innermost first. Brackets in
/// `skip` (sorted byte ranges) don't count.
pub(crate) fn fix_brackets(
    text: &str,
    pairs: &[(String, String)],
    skip: &[Range<usize>],
) -> String {
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .filter(|(open, close)| !open.is_empty() && !close.is_empty())
        .map(|(open, close)| (open.as_str(), close.as_str()))
        .collect();
    let mut out = String::with_capacity(text.len() + 4);
    let mut open: Vec<usize> = Vec::new();
    let mut ix = 0;
    while ix < text.len() {
        let rest = &text[ix..];
        let skipped = skip.iter().any(|range| range.start <= ix && ix < range.end);
        // The longest bracket starting here.
        let found = if skipped {
            None
        } else {
            pairs
                .iter()
                .enumerate()
                .flat_map(|(pair, (o, c))| [(pair, true, *o), (pair, false, *c)])
                .filter(|(_, _, bracket)| rest.starts_with(bracket))
                .max_by_key(|(_, _, bracket)| bracket.len())
        };
        match found {
            Some((pair, true, bracket)) => {
                open.push(pair);
                out.push_str(bracket);
                ix += bracket.len();
            }
            Some((_, false, bracket)) => {
                if let Some(at) = open.iter().rposition(|&pair| pairs[pair].1 == bracket) {
                    for pair in open.drain(at + 1..).rev() {
                        out.push_str(pairs[pair].1);
                    }
                    open.pop();
                    out.push_str(bracket);
                }
                ix += bracket.len();
            }
            None => {
                let ch = rest.chars().next().unwrap_or_default();
                out.push(ch);
                ix += ch.len_utf8().max(1);
            }
        }
    }
    for pair in open.into_iter().rev() {
        out.push_str(pairs[pair].1);
    }
    out
}

/// Where `item`'s first change (or jump) starts in `buffer`, its model.
fn edits_target_start(item: &Item, buffer: &text::Buffer) -> Option<PointUtf16> {
    let edit = item.edit.as_ref()?;
    let offset = edit.jump_to.or_else(|| {
        edit.edits
            .iter()
            .find_map(|updated| updated.edit.as_ref().map(|(range, _)| range.start))
    })?;
    let rope = buffer.rope();
    Some(rope.offset_to_point_utf16(offset.min(rope.len())))
}

/// Runs `completion`'s `shown_command` (Monaco `reportInlineEditShown`).
fn run_shown_command(completion: &InlineCompletion, cx: &mut gpui::App) {
    if let Some(command) = &completion.shown_command
        && let Err(err) = crate::commands::execute_command(cx, command)
    {
        log::warn!("{err:#}");
    }
}

/// Ends `items`' lifetimes as ignored (Monaco disposing a state).
fn end_items(items: Vec<Item>) {
    for item in items {
        item.provider.handle_end_of_lifetime(
            &item.completion,
            &InlineCompletionEndOfLifeReason::Ignored {
                superseded_by: None,
                user_typing_disagreed: false,
            },
        );
    }
}

/// Monaco's item hash: the same edit from a newer request.
fn same_item(new: &Item, old: &Item) -> bool {
    new.insert_text == old.insert_text
        && new.original_start == old.original_start
        && new.completion.is_inline_edit == old.completion.is_inline_edit
        && new.completion.jump_to_position == old.completion.jump_to_position
        && new.completion.uri == old.completion.uri
}

/// Moves `edit` (of this model) through the changes to `buffer` since its
/// version (Monaco `InlineEditItem._applyTextModelChanges`); `false` drops
/// it. `count` is the change count now.
fn follow_inline_edit(edit: &mut InlineEditItem, buffer: &text::Buffer, count: u64) -> bool {
    let version = buffer.version();
    // The hint is in this model, even for a cross-file edit.
    if let Some((range, hint_version, _)) = edit.hint.as_mut()
        && *hint_version != version
    {
        let patch = buffer.edits_since(*hint_version);
        let start = patch.transform_offset(range.start, text::Bias::Left);
        let end = patch.transform_offset(range.end, text::Bias::Right);
        *range = start..end.max(start);
        *hint_version = version;
    }
    // Monaco: a change to another model leaves a cross-file edit alone.
    if edit.target.is_some() || edit.version == version {
        return true;
    }
    let rope = buffer.rope();
    let patch = buffer.edits_since(edit.version);
    let changes: Changes = patch
        .edits()
        .iter()
        .map(|e| (e.old.clone(), rope.text_for_range(e.new.clone())))
        .collect();
    edit.version = version;
    if let Some(offset) = edit.jump_to {
        // Monaco `applyToOffsetOrUndefined`: gone once replaced.
        if changes
            .iter()
            .any(|(range, _)| range.start < offset && offset < range.end)
        {
            return false;
        }
        edit.jump_to = Some(patch.transform_offset(offset, text::Bias::Right));
        return true;
    }
    if edit.edits.is_empty() {
        // Only a hint.
        return true;
    }
    for updated in &mut edit.edits {
        updated.apply_changes(&changes);
    }
    if edit.edits.iter().any(|updated| updated.edit.is_none()) {
        return false;
    }
    edit.last_change_updated = edit
        .edits
        .iter()
        .any(|updated| updated.last_change_updated_edit);
    if edit.last_change_updated {
        edit.updated_at = count;
    }
    if edit.updated_at + INLINE_EDIT_MAX_IGNORED_CHANGES < count {
        return false;
    }
    edit.edits.retain(|updated| {
        updated
            .edit
            .as_ref()
            .is_some_and(|(range, text)| !range.is_empty() || !text.is_empty())
    });
    // Empty once the user typed it.
    !edit.edits.is_empty()
}

/// Monaco `getEndPositionsAfterApplying`: where each of `edits` (disjoint,
/// in any order) ends once they're all applied, in their order.
fn end_positions_after_applying(edits: &[TextReplacement]) -> Vec<PointUtf16> {
    let mut order: Vec<usize> = (0..edits.len()).collect();
    order.sort_by_key(|&ix| edits[ix].range.start);
    let sorted: Vec<(Range<PointUtf16>, String)> = order
        .iter()
        .map(|&ix| (edits[ix].range.clone(), edits[ix].text.clone()))
        .collect();
    let mut ends = vec![PointUtf16::default(); edits.len()];
    for (range, &ix) in new_ranges(&sorted).into_iter().zip(&order) {
        ends[ix] = range.end;
    }
    ends
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

    /// Monaco `fixBracketsInLine`, as `completeBracketPairs` uses it.
    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn fix_brackets_closes_and_drops() {
        let pairs: Vec<(String, String)> = [("(", ")"), ("[", "]"), ("{", "}")]
            .iter()
            .map(|(o, c)| (o.to_string(), c.to_string()))
            .collect();
        let fix = |text: &str| fix_brackets(text, &pairs, &[]);
        assert_eq!(fix("foo(bar"), "foo(bar)");
        assert_eq!(fix("a[b{c"), "a[b{c}]");
        // Closing an outer bracket closes the ones open inside it first.
        assert_eq!(fix("(a[b)"), "(a[b])");
        // A closing bracket with nothing to close goes.
        assert_eq!(fix("x)]"), "x");
        assert_eq!(fix("f(a))"), "f(a)");
        assert_eq!(fix("ok()"), "ok()");
        // Brackets in strings and comments don't count.
        assert_eq!(fix_brackets("f(\"(\"", &pairs, &[2..5]), "f(\"(\")");
        assert_eq!(fix_brackets("x // )", &pairs, &[2..6]), "x // )");
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
