//! The suggest controller (Monaco's `contrib/suggest` `SuggestController`
//! and `SuggestModel`): when to ask completion providers, live filtering
//! while typing, selection memories, resolving items, keyboard navigation
//! and accepting items in insert or replace mode.

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, SharedString, Task};
use text::PointUtf16;

use crate::cursor::{EditOperation, EditOperationType, Selection, StandardTokenType};
use crate::editor::CodeEditor;
use crate::options::{AcceptSuggestionOnEnter, SuggestInsertMode, WordBasedSuggestions};
use crate::providers::{
    CompletionItem, CompletionItemKind, CompletionItemTag, CompletionList, CompletionProvider,
    CompletionTrigger, completion_provider_groups, completion_providers,
};
use crate::suggest::memory::{Candidate, MemoryContext, SuggestMemories};
use crate::suggest::{CompletionModel, CompletionModelOptions, LineContext};
use crate::suggest_widget::{
    LOADING_MESSAGE, SuggestUi, SuggestWidget, SuggestWidgetMetrics, SuggestWidgetState,
};
use crate::widgets::{ContentWidgetPreference, WidgetId};

/// Monaco stops collecting document words past this many.
const MAX_WORDS: usize = 10_000;

/// Selection memories (Monaco's `SuggestMemoryService` storage): one set
/// shared by every window (Monaco's profile scope, with
/// `suggest.shareSuggestSelections`) and one per window (Monaco's
/// workspace scope, the default).
#[derive(Default)]
struct Memories {
    shared: SuggestMemories,
    windows: std::collections::HashMap<gpui::WindowId, SuggestMemories>,
}

impl gpui::Global for Memories {}

/// Which selection memories an editor uses (see
/// `suggest.share_suggest_selections`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryScope {
    /// Shared by every window.
    Shared,
    /// The window's own.
    Window(gpui::WindowId),
}

impl Memories {
    fn get(&mut self, scope: MemoryScope) -> &mut SuggestMemories {
        match scope {
            MemoryScope::Shared => &mut self.shared,
            MemoryScope::Window(id) => self.windows.entry(id).or_default(),
        }
    }
}

/// The item the selection memories pick (`suggest_selection`) among
/// `candidates` (Monaco `ISuggestMemoryService.select`).
pub(crate) fn memory_select(
    scope: MemoryScope,
    mode: crate::options::SuggestSelection,
    context: &MemoryContext,
    candidates: &[Candidate],
    cx: &mut gpui::App,
) -> usize {
    cx.default_global::<Memories>()
        .get(scope)
        .select(mode, context, candidates)
}

/// One provider's items in a request or session (an entry of Monaco's
/// `CompletionModel.getItemsByProvider`).
#[derive(Clone)]
pub(crate) struct ProviderItems {
    /// `None` for word-based suggestions.
    pub(crate) provider: Option<Arc<dyn CompletionProvider>>,
    pub(crate) items: Vec<CompletionItem>,
    /// The provider's list was incomplete.
    pub(crate) incomplete: bool,
}

/// Monaco's `snippetChoiceCompletions` provider: the active snippet
/// choice's options (see `CodeEditor::snippet_choice_completions`).
struct SnippetChoiceCompletions(Vec<CompletionItem>);

impl CompletionProvider for SnippetChoiceCompletions {
    fn provide_completions(
        &self,
        _: &crate::providers::ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut gpui::App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: self.0.clone(),
            incomplete: false,
        }))
    }

    fn debug_display_name(&self) -> SharedString {
        "snippetChoiceCompletions".into()
    }
}

/// Monaco `CompletionOptions.providerFilter` and `providerItemsToReuse`.
#[derive(Default)]
pub(crate) struct SuggestRequestOptions {
    /// Only these providers are asked; all when `None`.
    pub(crate) filter: Option<Vec<Arc<dyn CompletionProvider>>>,
    /// Items kept from the open session instead of asking their providers
    /// again.
    pub(crate) reuse: Vec<ProviderItems>,
}

fn same_provider(a: &Arc<dyn CompletionProvider>, b: &Arc<dyn CompletionProvider>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// Monaco `provideSuggestionItems`: asks the provider `groups` (equal
/// scores, best first; see
/// [`completion_provider_groups`](crate::providers::completion_provider_groups))
/// one group at a time, and stops after the first group that yields items
/// `keep` accepts. Providers in `options.reuse` contribute their kept items
/// instead of being asked, and with `options.filter` only those providers
/// are asked.
pub(crate) fn provide_suggestion_items(
    groups: Vec<Vec<Arc<dyn CompletionProvider>>>,
    options: SuggestRequestOptions,
    context: crate::providers::ProviderContext,
    position: PointUtf16,
    trigger: CompletionTrigger,
    keep: impl Fn(&CompletionItem) -> bool + 'static,
    cx: &mut gpui::App,
) -> Task<Vec<ProviderItems>> {
    cx.spawn(async move |cx| {
        let mut result = Vec::new();
        for group in groups {
            let mut did_add_result = false;
            let mut tasks = Vec::new();
            for provider in group {
                let reused = options.reuse.iter().find(|reused| {
                    reused
                        .provider
                        .as_ref()
                        .is_some_and(|p| same_provider(p, &provider))
                });
                if let Some(reused) = reused {
                    did_add_result |= !reused.items.is_empty();
                    result.push(reused.clone());
                    continue;
                }
                if options
                    .filter
                    .as_ref()
                    .is_some_and(|filter| !filter.iter().any(|p| same_provider(p, &provider)))
                {
                    continue;
                }
                let task = cx.update(|cx| {
                    provider.provide_completions(&context, position, trigger.clone(), cx)
                });
                tasks.push((provider, task));
            }
            for (provider, task) in tasks {
                match task.await {
                    Ok(list) => {
                        let items: Vec<_> = list.items.into_iter().filter(|i| keep(i)).collect();
                        did_add_result |= !items.is_empty();
                        result.push(ProviderItems {
                            provider: Some(provider),
                            items,
                            incomplete: list.incomplete,
                        });
                    }
                    Err(err) => log::warn!("completions failed: {err:#}"),
                }
            }
            if did_add_result {
                break;
            }
        }
        result
    })
}

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
    /// Identifies the request's completion model.
    pub(crate) id: u64,
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
    /// Whether `selected` is focused (Monaco's list focus): off when
    /// `suggest.selection_mode` opened the widget without a focused item,
    /// until one is focused.
    pub(crate) focused: bool,
    /// Whether each of `providers` gave an incomplete list with items
    /// (Monaco `getIncompleteProvider`): typing more asks those again.
    provider_incomplete: Vec<bool>,
    /// Opened by itself (typing, a trigger character), not Ctrl+Space
    /// (Monaco's `State.Auto`).
    auto: bool,
    /// Requested because a trigger character was typed (Monaco's
    /// `CompletionTriggerKind.TriggerCharacter`).
    trigger_character: bool,
    /// The provider of each item; `None` for word-based suggestions.
    item_providers: Vec<Option<usize>>,
    providers: Vec<Arc<dyn CompletionProvider>>,
    /// Items resolved.
    pub(crate) resolved: Vec<bool>,
    /// Items whose provider resolves them (Monaco's
    /// `provider.resolveCompletionItem`), known once one was asked to.
    resolvable: Vec<bool>,
    /// The item being resolved.
    resolving: Option<usize>,
    resolve_task: Option<Task<()>>,
    /// Monaco `_onListFocus`: the details panel shows "Loading..." once
    /// the focused item has been resolving for 250ms.
    details_loading: bool,
    details_loading_task: Option<Task<()>>,
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
    /// Monaco's explain mode (`toggleExplainMode`): the details panel
    /// shows how the focused item was scored. Off once the widget hides.
    explain_mode: bool,
    /// Monaco's `State.Details`: the details panel has the focus
    /// (`toggleSuggestionFocus`), so the keys scroll it. Off once the
    /// widget hides.
    pub(crate) details_focused: bool,
    /// This editor's completion items as ghost text (`quick_suggestions`
    /// `Inline`).
    pub(crate) inline_provider:
        Option<Arc<crate::suggest_inline_completions::SuggestInlineCompletions>>,
    /// Monaco `_waitForInlineCompletions` (`OffWhenInlineCompletions`).
    inline_wait: Option<InlineWait>,
    /// Sessions started, for their ids.
    sessions: u64,
    /// Monaco `OvertypingCapturer`: each selection's text (and whether it
    /// spans lines) when typing last replaced selections, for `$SELECTION`
    /// in the suggestion inserted next.
    overtyped: Vec<(String, bool)>,
    /// What `$SELECTION` falls back to while a suggestion is inserted.
    pub(crate) snippet_overtyped: Option<Vec<(String, bool)>>,
    /// The widget's message screen readers last read, so it isn't read
    /// again on every keystroke.
    announced: Option<&'static str>,
    /// Whether the widget's focused row is the editor's active descendant.
    active_descendant: bool,
    /// Monaco `LineSuffix`, set on each request: what follows the cursor
    /// on its line, for the `suffixDelta` accepting adds.
    line_suffix: Option<LineSuffix>,
}

/// Monaco `LineSuffix`: the request's row and how many UTF-16 units of
/// its line followed the cursor. Monaco tracks a decoration on the
/// character after the request (or the line end), which typing at the
/// cursor pushes along; the same text after it stays this long.
#[derive(Clone, Copy)]
struct LineSuffix {
    row: u32,
    len: u32,
}

impl LineSuffix {
    /// Monaco `LineSuffix.delta`: the marker's column minus `position`'s
    /// (negative once the cursor moved right without typing), 0 on another
    /// line.
    fn delta(&self, line_len: u32, position: PointUtf16) -> i64 {
        if position.row != self.row {
            return 0;
        }
        i64::from(line_len.saturating_sub(self.len)) - i64::from(position.column)
    }
}

/// A quick suggestion waiting for inline completions to settle before
/// deciding whether to open (Monaco `_waitForInlineCompletionsAndTrigger`).
struct InlineWait {
    /// The text version and cursor when it started; the widget opens only
    /// if neither changed.
    version: text::Version,
    position: PointUtf16,
    /// Checks again whenever the editor changes.
    _observer: gpui::Subscription,
    /// Monaco gives up waiting after 750ms.
    _timeout: Task<()>,
}

/// Monaco `_onListFocus`: how long an item resolves before its details
/// show "Loading...".
const DETAILS_LOADING_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// Monaco's wait for inline completions before a quick suggestion.
const INLINE_COMPLETIONS_WAIT: std::time::Duration = std::time::Duration::from_millis(750);

impl Session {
    fn insert_context(&self) -> InsertContext {
        InsertContext {
            position: self.position,
            word_start: self.word_start,
            word_end: self.word_end,
        }
    }

    /// The focused item's index in the session's items.
    fn focused_index(&self) -> Option<usize> {
        if !self.focused {
            return None;
        }
        self.model.filtered().get(self.selected).map(|s| s.index)
    }

    /// Whether the focused item is resolving, and whether its details show
    /// "Loading..." meanwhile.
    pub(crate) fn focused_resolving(&self) -> (bool, bool) {
        let resolving = self
            .focused_index()
            .is_some_and(|index| self.resolving == Some(index));
        (resolving, resolving && self.details_loading)
    }

    /// Monaco's `suggestionCanResolve` for the focused item: its provider
    /// resolves items, or it has docs, or a detail other than its label (no
    /// detail counts, as Monaco compares `undefined` with the label).
    pub(crate) fn focused_can_resolve(&self) -> bool {
        let Some(index) = self.focused_index() else {
            return false;
        };
        let item = &self.model.items()[index];
        self.resolvable[index]
            || item.documentation.is_some()
            || crate::suggest_widget::has_label_object(item)
            || item.detail.as_ref() != Some(&item.label)
    }

    /// Monaco's `suggestionHasInsertAndReplaceRange` for the focused item:
    /// its insert and replace ranges end in different places. Items without
    /// ranges get the session's (Monaco fills them in when collecting),
    /// which do when the cursor is inside a word.
    pub(crate) fn focused_has_insert_and_replace(&self) -> bool {
        let Some(index) = self.focused_index() else {
            return false;
        };
        let item = &self.model.items()[index];
        let insert_end = item.range.as_ref().map_or(self.position, |r| r.end);
        let replace_end = item
            .replace_range
            .as_ref()
            .or(item.range.as_ref())
            .map_or(PointUtf16::new(self.position.row, self.word_end), |r| r.end);
        insert_end != replace_end
    }

    /// Monaco `CompletionModel.getItemsByProvider`: the session's items by
    /// provider (word-based ones under `None`). Items without a range get
    /// the session's default ranges, as Monaco fills them in when
    /// collecting, so they still insert there when reused by a later
    /// request.
    fn items_by_provider(&self) -> Vec<ProviderItems> {
        let row = self.position.row;
        let insert = PointUtf16::new(row, self.word_start)..self.position;
        let replace = PointUtf16::new(row, self.word_start)..PointUtf16::new(row, self.word_end);
        let mut groups: Vec<(Option<usize>, ProviderItems)> = Vec::new();
        for (item, provider) in self.model.items().iter().zip(&self.item_providers) {
            let mut item = item.clone();
            if item.range.is_none() {
                item.range = Some(insert.clone());
                item.replace_range = item.replace_range.or_else(|| Some(replace.clone()));
            }
            match groups.iter_mut().find(|(p, _)| p == provider) {
                Some((_, group)) => group.items.push(item),
                None => groups.push((
                    *provider,
                    ProviderItems {
                        provider: provider.map(|ix| self.providers[ix].clone()),
                        items: vec![item],
                        incomplete: provider.is_some_and(|ix| self.provider_incomplete[ix]),
                    },
                )),
            }
        }
        groups.into_iter().map(|(_, group)| group).collect()
    }
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Whether `row`'s leading whitespace is `expected`, reading only as much
/// of the row as that takes (not a whole long line).
fn row_has_leading_whitespace(rope: &text::Rope, row: u32, expected: &str) -> bool {
    let start = rope.point_to_offset(text::Point::new(row, 0));
    let mut chars = rope.chars_at(start);
    expected.chars().all(|c| chars.next() == Some(c)) && !matches!(chars.next(), Some(' ' | '\t'))
}

impl CodeEditor {
    /// Whether the suggest widget is showing.
    pub fn is_suggest_visible(&self) -> bool {
        self.suggest.widget.is_some()
    }

    /// How many suggestions the open widget lists.
    pub(crate) fn suggest_item_count(&self) -> usize {
        match (&self.suggest.widget, &self.suggest.session) {
            (Some(_), Some(session)) => session.model.filtered().len(),
            _ => 0,
        }
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

    /// The details panel's detail and docs (markdown), when it shows.
    #[doc(hidden)]
    pub fn suggest_details_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(Option<String>, Option<String>)> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        let widget = widget.read(cx);
        if !widget.details_shown(cx) {
            return None;
        }
        let (detail, docs) = widget.details_content()?;
        Some((
            detail.map(|d| d.to_string()),
            docs.map(|d| d.as_str().to_string()),
        ))
    }

    /// The suggest list's first visible row and how many rows show, for
    /// tests.
    #[doc(hidden)]
    pub fn suggest_rows_shown_for_tests(&self, cx: &gpui::App) -> Option<(usize, usize)> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        let widget = widget.read(cx);
        Some((widget.first_visible_row(), widget.visible_rows(cx)))
    }

    /// The suggest list's side of the cursor line and its size (border box
    /// width, rows and status bar height), for tests.
    #[doc(hidden)]
    pub fn suggest_list_layout_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(ContentWidgetPreference, gpui::Size<gpui::Pixels>)> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        let widget = widget.read(cx);
        Some((widget.list_side(cx), widget.list_size(cx)))
    }

    /// Where the details panel was last placed (`"east"`, `"west"`,
    /// `"south"` or `"north"`) and its border box relative to the list's,
    /// for tests.
    #[doc(hidden)]
    pub fn suggest_details_placement_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(&'static str, gpui::Bounds<gpui::Pixels>)> {
        use crate::suggest_widget::DetailsSide;
        let (widget, _) = self.suggest.widget.as_ref()?;
        let (side, bounds) = widget.read(cx).details_placement()?;
        let side = match side {
            DetailsSide::East => "east",
            DetailsSide::West => "west",
            DetailsSide::South => "south",
            DetailsSide::North => "north",
        };
        Some((side, bounds))
    }

    /// Where the suggest list's rows and the details panel's content were
    /// last painted, in window coordinates (inside their borders), for
    /// tests.
    #[doc(hidden)]
    pub fn suggest_painted_bounds_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(
        gpui::Bounds<gpui::Pixels>,
        Option<gpui::Bounds<gpui::Pixels>>,
    )> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        widget.read(cx).painted_bounds(cx)
    }

    /// The suggest status bar's left and right actions as shown, for tests.
    #[doc(hidden)]
    pub fn suggest_status_bar_for_tests(&self, cx: &gpui::App) -> Option<(String, String)> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        widget.read(cx).status_bar_text(cx)
    }

    /// Whether the suggest list's scrollbar shows without the mouse over
    /// it (for a while after scrolling), for tests.
    #[doc(hidden)]
    pub fn suggest_scrollbars_revealed_for_tests(&self, cx: &gpui::App) -> bool {
        self.suggest
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).scrollbars_revealed(cx))
    }

    /// The suggest list's and details panel's scrollbar opacity as last
    /// rendered, and whether either was fading, for tests.
    #[doc(hidden)]
    pub fn suggest_scrollbar_opacities_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<([f32; 2], bool)> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        Some(widget.read(cx).scrollbar_opacities())
    }

    /// The suggest list's scroll position in pixels, for tests.
    #[doc(hidden)]
    pub fn suggest_list_scroll_top_for_tests(&self, cx: &gpui::App) -> Option<gpui::Pixels> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        Some(widget.read(cx).list_scroll_top())
    }

    /// The height of the suggest details panel's content as last painted,
    /// for tests.
    #[doc(hidden)]
    pub fn suggest_details_content_height_for_tests(&self, cx: &gpui::App) -> Option<gpui::Pixels> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        widget.read(cx).details_content_height()
    }

    /// The size the suggest widget was resized to, if any.
    #[doc(hidden)]
    pub fn suggest_size_for_tests(cx: &gpui::App) -> Option<gpui::Size<gpui::Pixels>> {
        cx.try_global::<SuggestUi>().and_then(|ui| ui.size)
    }

    /// Stores a resized size for the suggest widget, as dragging its edges
    /// does.
    #[doc(hidden)]
    pub fn set_suggest_size_for_tests(size: gpui::Size<gpui::Pixels>, cx: &mut gpui::App) {
        cx.default_global::<SuggestUi>().size = Some(size);
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
        let session = self.suggest.session.as_ref().filter(|s| s.focused)?;
        self.suggestion_edit(session.selected)
    }

    /// The edit of the suggestion at `index` of the list (Monaco
    /// `SuggestItemInfo.getSingleTextEdit`), as for
    /// [`Self::selected_suggestion_edit`].
    pub(crate) fn suggestion_edit(
        &self,
        index: usize,
    ) -> Option<(std::ops::Range<PointUtf16>, String)> {
        let session = self.suggest.session.as_ref()?;
        let scored = session.model.filtered().get(index)?;
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

    /// The selection memories this editor uses: its window's, or with
    /// `suggest.share_suggest_selections` the ones every window shares.
    pub(crate) fn memory_scope(&self) -> MemoryScope {
        match self.window_id {
            Some(id) if !self.options.suggest.share_suggest_selections => MemoryScope::Window(id),
            _ => MemoryScope::Shared,
        }
    }

    /// Whether a suggestion is focused (Monaco
    /// `suggestWidgetHasFocusedSuggestion`).
    pub(crate) fn has_focused_suggestion(&self) -> bool {
        self.suggest
            .session
            .as_ref()
            .is_some_and(|s| s.focused && s.selected < s.model.filtered().len())
    }

    /// Whether the focused suggestion's provider gave an incomplete list
    /// (Monaco `SuggestItemInfo.listIncomplete`).
    pub(crate) fn selected_suggestion_incomplete(&self) -> bool {
        let Some(session) = self.suggest.session.as_ref().filter(|s| s.focused) else {
            return false;
        };
        session
            .model
            .filtered()
            .get(session.selected)
            .and_then(|scored| session.item_providers[scored.index])
            .is_some_and(|provider| session.provider_incomplete[provider])
    }

    /// The focused suggestion and its index in the list; `None` while
    /// none is focused (see `suggest.selection_mode`).
    pub fn selected_suggestion(&self) -> Option<(usize, CompletionItem)> {
        let session = self.suggest.session.as_ref().filter(|s| s.focused)?;
        let scored = session.model.filtered().get(session.selected)?;
        Some((
            session.selected,
            session.model.items()[scored.index].clone(),
        ))
    }

    /// The word at `position` by the language's `wordPattern` (Monaco
    /// `getWordAtPosition`), as start and end columns; `position` itself
    /// when there's none.
    fn word_columns_at(&self, position: PointUtf16, cx: &gpui::App) -> (u32, u32) {
        self.model()
            .read(cx)
            .word_at_position(position)
            .filter(|word| word.range.start.row == position.row)
            .map_or((position.column, position.column), |word| {
                (
                    word.range.start.column.min(position.column),
                    word.range.end.column.max(position.column),
                )
            })
    }

    /// Start column of the word ending at the cursor (Monaco
    /// `getWordUntilPosition`), by the language's `wordPattern`.
    pub(crate) fn word_start_at(&self, position: PointUtf16, cx: &gpui::App) -> u32 {
        self.word_columns_at(position, cx).0
    }

    fn line_context_at(&self, position: PointUtf16, session_column: u32) -> LineContext {
        // The row up to `position`, a char split by it included (not the
        // rest of a long line).
        let rope = self.snapshot.rope();
        let start = rope.point_to_offset(text::Point::new(position.row, 0));
        let end = rope.point_utf16_to_offset(rope.clip_point_utf16(position, text::Bias::Right));
        LineContext {
            leading_line_content: rope.text_for_range(start..end.max(start)),
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
        self.suggest.explain_mode = false;
        self.suggest.details_focused = false;
        self.suggest.announced = None;
        self.suggest.active_descendant = false;
        if let Some((_, id)) = self.suggest.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Monaco `SuggestModel._updateActiveSuggestSession`, when completion
    /// providers are registered or removed: an open session asks again, or
    /// closes when no provider is left (Monaco registers word-based
    /// suggestions as a `*` provider, so they count while they're on).
    pub(crate) fn update_active_suggest_session(&mut self, cx: &mut Context<Self>) {
        let Some(auto) = self.suggest.session.as_ref().map(|session| session.auto) else {
            return;
        };
        let has_provider = self.options.word_based_suggestions != WordBasedSuggestions::Off
            || !completion_provider_groups(cx, self.provider_target(cx)).is_empty();
        if has_provider {
            self.request_suggestions(CompletionTrigger::Invoke, auto, cx);
        } else {
            self.hide_suggest(cx);
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
        self.request_suggestions_with(trigger, auto, SuggestRequestOptions::default(), cx);
    }

    /// Monaco `SuggestModel.trigger` with `completionOptions`: asks the
    /// providers (see [`provide_suggestion_items`]), keeping the items
    /// `options` reuses.
    fn request_suggestions_with(
        &mut self,
        trigger: CompletionTrigger,
        auto: bool,
        mut options: SuggestRequestOptions,
        cx: &mut Context<Self>,
    ) {
        // Widget inputs are plain inputs in Monaco.
        if self.options.read_only || self.widget_input {
            return;
        }
        let position = self.cursors.primary().position;
        // Monaco `onDidTrigger` sets the `LineSuffix`.
        self.suggest.line_suffix = Some(LineSuffix {
            row: position.row,
            len: self
                .snapshot
                .rope()
                .line_len_utf16(position.row)
                .saturating_sub(position.column),
        });
        let context = self.model().read(cx).provider_context();
        // Monaco's snippet controller registers its choice provider as
        // exclusive while a choice is active, so it's the only one asked,
        // word-based suggestions included.
        let choice = self.snippet_choice_completions(cx);
        let groups = match &choice {
            Some((items, _)) => {
                options = SuggestRequestOptions::default();
                vec![vec![Arc::new(SnippetChoiceCompletions(items.clone()))
                    as Arc<dyn CompletionProvider>]]
            }
            None => completion_provider_groups(cx, self.provider_target(cx)),
        };
        // Word-based suggestions are the lowest group here (Monaco
        // registers them as a `*` provider): reused ones only show when
        // no provider has items.
        let reused_words = options
            .reuse
            .iter()
            .position(|reused| reused.provider.is_none())
            .map(|ix| options.reuse.remove(ix).items);
        let has_providers = !groups.is_empty();
        let (word_start, word_end) = self.word_columns_at(position, cx);
        // Monaco `offWithInlineSuggestions`: off while an inline
        // completions provider serves this language and they're enabled.
        let words_enabled = choice.is_none()
            && match self.options.word_based_suggestions {
                WordBasedSuggestions::Off => false,
                WordBasedSuggestions::OffWithInlineSuggestions => {
                    !self.options.inline_suggest.enabled
                        || crate::providers::inline_completions_providers(
                            cx,
                            self.provider_target(cx),
                        )
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
        if !has_providers && words.is_none() && reused_words.is_none() {
            return;
        }
        let request = provide_suggestion_items(
            groups,
            options,
            context,
            position,
            trigger.clone(),
            self.suggestion_filter(),
            cx,
        );
        let trigger_character = matches!(trigger, CompletionTrigger::TriggerCharacter(_));
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
                            position,
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
            let mut providers = Vec::new();
            let mut provider_incomplete = Vec::new();
            for list in request.await {
                let Some(provider) = list.provider else {
                    continue;
                };
                let ix = providers.len();
                providers.push(provider);
                // Monaco `getIncompleteProvider` counts providers with items.
                provider_incomplete.push(list.incomplete && !list.items.is_empty());
                item_providers.extend(std::iter::repeat_n(Some(ix), list.items.len()));
                items.extend(list.items);
            }
            if items.is_empty()
                && let Some(reused) = reused_words
            {
                item_providers = vec![None; reused.len()];
                items = reused;
            }
            // Monaco: word-based suggestions are a lower-ranked provider
            // group, used only when the others have nothing; collected off
            // the main thread.
            else if items.is_empty()
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
                    (providers, provider_incomplete),
                    (position, word_start, word_end),
                    (auto, trigger_character),
                    cx,
                )
            })
            .ok();
        }));
    }

    /// Whether an item is kept: not of a hidden kind, deprecated
    /// (`suggest.show_deprecated`) or a snippet (`snippet_suggestions:
    /// None`), as Monaco filters when collecting.
    fn suggestion_filter(&self) -> impl Fn(&CompletionItem) -> bool + 'static {
        let hidden_kinds = self.options.suggest.hidden_kinds.clone();
        let show_deprecated = self.options.suggest.show_deprecated;
        let snippets = self.options.snippet_suggestions.is_some();
        move |item| {
            !hidden_kinds.contains(&item.kind)
                && (show_deprecated || !item.tags.contains(&CompletionItemTag::Deprecated))
                && (snippets || item.kind != CompletionItemKind::Snippet)
        }
    }

    /// Drops the items [`Self::suggestion_filter`] doesn't keep.
    fn filter_suggestion_kinds(
        &self,
        items: Vec<CompletionItem>,
        item_providers: Vec<Option<usize>>,
    ) -> (Vec<CompletionItem>, Vec<Option<usize>>) {
        let keep = self.suggestion_filter();
        items
            .into_iter()
            .zip(item_providers)
            .filter(|(item, _)| keep(item))
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
        (providers, provider_incomplete): (Vec<Arc<dyn CompletionProvider>>, Vec<bool>),
        (position, word_start, word_end): (PointUtf16, u32, u32),
        (auto, trigger_character): (bool, bool),
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
        let resolvable = vec![false; items.len()];
        let mut model =
            CompletionModel::new_with_options(items, position, word_start, line_context, options);
        // Monaco: distances only count while the cursor is where the request
        // was made.
        if head == position {
            model.set_distances(distances);
        }
        let leading_whitespace = leading_whitespace(&self.snapshot.line(position.row)).to_string();
        self.suggest.sessions += 1;
        self.suggest.session = Some(Session {
            id: self.suggest.sessions,
            position,
            word_start,
            word_end,
            leading_whitespace,
            frozen: false,
            model,
            selected: 0,
            focused: !self.suggest_no_focus(auto, trigger_character, false),
            provider_incomplete,
            auto,
            trigger_character,
            item_providers,
            providers,
            resolved,
            resolvable,
            resolving: None,
            resolve_task: None,
            details_loading: false,
            details_loading_task: None,
        });
        self.select_by_memory(cx);
        if self.suggest.accept_best.is_some() {
            self.finish_accept_best(cx);
            return;
        }
        self.resolve_selected_suggestion(cx);
        self.refresh_suggest_widget(cx);
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
            (Vec::new(), Vec::new()),
            (head, range.start.column, range.end.column),
            (false, false),
            cx,
        );
    }

    /// Tab completion (Monaco `insertBestCompletion`): at the end of a word,
    /// asks for completions without showing them and inserts the one
    /// `suggest_selection` picks, or types a tab if none changes the text.
    /// Returns false if it didn't apply (no word ends at the cursor).
    pub(crate) fn insert_best_completion(&mut self, cx: &mut Context<Self>) -> bool {
        let head = self.cursors.primary().position;
        if self.options.read_only || self.is_suggest_visible() || {
            let (start, end) = self.word_columns_at(head, cx);
            start == head.column || end != head.column
        } {
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

    /// Monaco `OvertypingCapturer` on `onWillType`: remembers the text of
    /// the selections typing is about to replace (for `$SELECTION` in a
    /// suggestion inserted next), or forgets it when typing replaces none.
    /// It's locked from a suggest request until the widget closes (Monaco's
    /// `onDidTrigger` to `onDidCancel`), so typing that filters a session
    /// keeps what the first keystroke replaced.
    pub(crate) fn capture_overtyped(&mut self) {
        /// Monaco `OvertypingCapturer._maxSelectionLength`.
        const MAX_SELECTION_LENGTH: usize = 51_200;
        if self.suggest.session.is_some() || self.suggest.task.is_some() {
            return;
        }
        let selections = self.cursors.selections();
        if selections.iter().all(Selection::is_empty) {
            self.suggest.overtyped.clear();
            return;
        }
        self.suggest.overtyped.clear();
        for selection in selections {
            let start = self.snapshot.point_utf16_to_offset(selection.start());
            let end = self.snapshot.point_utf16_to_offset(selection.end());
            let text = self.snapshot.rope().text_for_range(start..end);
            // Monaco counts UTF-16 units, and stops at the first that's too
            // long.
            if text.encode_utf16().count() > MAX_SELECTION_LENGTH {
                return;
            }
            let multiline = selection.start().row != selection.end().row;
            self.suggest.overtyped.push((text, multiline));
        }
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
        if ranges.is_empty() {
            return Vec::new();
        }
        let (_, regex) = self.word_sources(cx);
        // The word being typed doesn't count.
        let word_start = self.word_start_at(position, cx);
        let current: String = self
            .snapshot
            .line(position.row)
            .encode_utf16()
            .skip(word_start as usize)
            .take((position.column - word_start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        let Some(distance) = crate::suggest::word_distance::WordDistance::new(
            self.snapshot.rope(),
            ranges,
            &regex,
            &current,
        ) else {
            return Vec::new();
        };
        // Monaco's completion model asks for each item at its request
        // position.
        items
            .iter()
            .map(|item| distance.distance(position, item))
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
        let word_start = self.word_start_at(head, cx);
        let word_until: String = line
            .encode_utf16()
            .skip(word_start as usize)
            .take(head.column.saturating_sub(word_start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        (language, line, head.column, word_until)
    }

    /// Selects the item `suggest_selection` picks (Monaco's memory
    /// `select`).
    fn select_by_memory(&mut self, cx: &mut Context<Self>) {
        // Monaco's selectors by priority: the inline completion's (100,
        // `SuggestWidgetAdaptor`) before the memory's (0).
        if let Some(index) = self.inline_suggest_preselect(cx)
            && let Some(session) = self.suggest.session.as_mut()
        {
            session.selected = index;
            return;
        }
        let (language, line, column, word_until) = self.memory_context(cx);
        let mode = self.options.suggest_selection;
        let scope = self.memory_scope();
        let memories = cx.default_global::<Memories>().get(scope);
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
        session.selected = memories.select(mode, &context, &candidates);
    }

    fn refresh_suggest_widget(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.suggest.session else {
            return;
        };
        if session.model.filtered().is_empty() && session.auto {
            self.hide_suggest(cx);
            return;
        }
        // Monaco `showSuggestions`: at the cursor, on every refilter.
        let anchor = self.cursors.primary().position;
        let mut state = SuggestWidgetState::from_session(session);
        if self.suggest.explain_mode {
            state.set_explanation(self.suggest_explanation());
        }
        state.set_details_focused(self.suggest.details_focused);
        self.show_suggest_widget(state, anchor, cx);
        // The focus goes back to the list when the details panel closes.
        if self.suggest.details_focused && !self.is_suggestion_details_visible(cx) {
            self.suggest.details_focused = false;
            self.refresh_suggest_widget(cx);
        }
    }

    /// Moves the focus from the suggest list to its details panel or back
    /// (Monaco `toggleSuggestionFocus`, Ctrl+Alt+Space): while the details
    /// panel has it, Up/Down and Page Up/Page Down scroll the panel and
    /// Cmd/Ctrl+C copies its text. Opens the details panel if it's closed.
    /// Returns whether the focus moved.
    pub fn toggle_suggestion_focus(&mut self, cx: &mut Context<Self>) -> bool {
        if self.suggest.details_focused {
            self.suggest.details_focused = false;
            self.refresh_suggest_widget(cx);
            return true;
        }
        // Monaco's `State.Open`: a focused item in a list that isn't frozen.
        let open = self.is_suggest_visible()
            && self
                .suggest
                .session
                .as_ref()
                .is_some_and(|s| s.focused && !s.frozen && !s.model.filtered().is_empty());
        if !open {
            return false;
        }
        if !self.is_suggestion_details_visible(cx) {
            // Monaco `toggleDetails(true)`: shows them if the item has any
            // (remembered for later sessions).
            let expanded = crate::suggest_widget::details_expanded(cx);
            cx.default_global::<SuggestUi>().details_expanded = true;
            self.refresh_suggest_widget(cx);
            if !self.is_suggestion_details_visible(cx) {
                cx.default_global::<SuggestUi>().details_expanded = expanded;
                return false;
            }
        }
        self.suggest.details_focused = true;
        self.refresh_suggest_widget(cx);
        true
    }

    /// Whether the suggest widget's details panel has the focus (Monaco
    /// `suggestWidgetDetailsFocused`).
    pub fn is_suggestion_details_focused(&self) -> bool {
        self.suggest.details_focused
    }

    /// Scrolls the focused details panel; false if it isn't focused.
    pub(crate) fn scroll_suggestion_details(
        &mut self,
        scroll: crate::suggest_widget::DetailsScroll,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.suggest.details_focused {
            return false;
        }
        let Some((widget, _)) = &self.suggest.widget else {
            return false;
        };
        widget.update(cx, |widget, cx| {
            widget.scroll_details(scroll);
            cx.notify();
        });
        true
    }

    /// The details panel's scroll position, for tests.
    #[doc(hidden)]
    pub fn suggest_details_scroll_for_tests(&self, cx: &gpui::App) -> Option<gpui::Pixels> {
        let (widget, _) = self.suggest.widget.as_ref()?;
        Some(widget.read(cx).details_scroll_top())
    }

    /// Copies the focused details panel's text to the clipboard (Monaco
    /// `suggestWidgetCopy`, Cmd/Ctrl+C while the details panel has the
    /// focus). Monaco copies the text selected in the panel; there's no
    /// selection here, so this copies the detail and the docs' markdown.
    /// Returns whether it copied.
    pub fn copy_suggestion_details(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.suggest.details_focused {
            return false;
        }
        let Some(text) = self
            .suggest
            .widget
            .as_ref()
            .and_then(|(widget, _)| widget.read(cx).details_text())
        else {
            return false;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        true
    }

    /// The suggest widget's row height (`suggest.line_height`, or the
    /// editor's, clamped to 8–1000 as in Monaco).
    pub(crate) fn suggest_item_height(&self) -> gpui::Pixels {
        let line_height = self.options.suggest.line_height;
        let height = if line_height > 0. {
            gpui::px(line_height)
        } else {
            self.line_height()
        };
        height.clamp(gpui::px(8.), gpui::px(1000.))
    }

    /// The height of the line showing `point` (the editor's while it isn't
    /// laid out).
    fn line_height_at(&self, point: PointUtf16) -> gpui::Pixels {
        self.layout
            .as_ref()
            .and_then(|layout| {
                let line = layout.line_for_point(point)?;
                Some(layout.line_height_of(line.view_line))
            })
            .unwrap_or_else(|| self.line_height())
    }

    fn suggest_metrics(&self, anchor: PointUtf16) -> SuggestWidgetMetrics {
        let font_size = if self.options.suggest.font_size > 0. {
            gpui::px(self.options.suggest.font_size)
        } else {
            self.options.font_size
        };
        SuggestWidgetMetrics {
            item_height: self.suggest_item_height(),
            font_size,
            line_height: self.line_height_at(anchor),
            editor_width: self.layout_size().map(|size| size.width),
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
                let style = self.markdown_style(cx);
                let options = self.options.suggest.clone();
                let widget = cx.new(|cx| {
                    SuggestWidget::new(editor, state, theme, style, options, metrics, cx)
                });
                let (id, room) = self.add_overflow_content_widget(
                    anchor,
                    vec![
                        ContentWidgetPreference::Below,
                        ContentWidgetPreference::Above,
                    ],
                    widget.clone().into(),
                    crate::widgets::layer::SUGGEST,
                    cx,
                );
                widget.update(cx, |widget, _| widget.room = room);
                self.suggest.widget = Some((widget, id));
            }
        }
        self.announce_suggest_widget(cx);
    }

    /// Has screen readers read the suggest widget's "Loading..." or "No
    /// suggestions." message when it shows (Monaco `status`), and notes
    /// whether its focused row is the editor's active descendant: the
    /// focused suggestion is read that way, not announced.
    fn announce_suggest_widget(&mut self, cx: &mut Context<Self>) {
        let Some((widget, _)) = &self.suggest.widget else {
            return;
        };
        let widget = widget.read(cx);
        let text = widget.announcement();
        self.suggest.active_descendant = widget.has_active_descendant();
        if let Some(text) = text
            && self.suggest.announced != Some(text)
        {
            self.announce_status(text, cx);
        }
        self.suggest.announced = text;
    }

    /// Whether the suggest widget's focused row is the editor's active
    /// descendant (Monaco's suggest widget `setAriaOptions({
    /// activeDescendant })`), which makes the editor's `aria-autocomplete`
    /// "list".
    pub(crate) fn suggest_has_active_descendant(&self) -> bool {
        self.suggest.active_descendant
    }

    /// Shows or hides the details panel (Monaco `toggleSuggestionDetails`,
    /// Ctrl+Space while suggestions show); remembered for later sessions.
    /// Like Monaco's `toggleDetails`, it only shows them when the focused
    /// item has docs or a detail (or explain mode is on), so it does
    /// nothing on an item without.
    pub fn toggle_suggestion_details(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.suggest.session else {
            return;
        };
        if crate::suggest_widget::details_expanded(cx) {
            cx.default_global::<SuggestUi>().details_expanded = false;
        } else {
            let can_expand = session.focused_index().is_some_and(|index| {
                crate::suggest_widget::can_expand(&session.model.items()[index])
            });
            if !self.is_suggest_visible() || !(can_expand || self.suggest.explain_mode) {
                return;
            }
            cx.default_global::<SuggestUi>().details_expanded = true;
        }
        self.refresh_suggest_widget(cx);
    }

    /// Rows the list pages by (Page Up / Page Down).
    pub(crate) fn suggest_page_size(&self, cx: &gpui::App) -> usize {
        self.suggest.widget.as_ref().map_or(11, |(widget, _)| {
            widget.read(cx).visible_rows(cx).saturating_sub(1).max(1)
        })
    }

    /// Resolves the selected item if its provider can (Monaco resolves on
    /// focus): its docs, detail and edits replace the original's. Call it
    /// before showing the focus, so the widget knows the item is resolving.
    /// While it resolves, the details panel keeps what it showed, and
    /// shows "Loading..." after 250ms (Monaco `_onListFocus`).
    fn resolve_selected_suggestion(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.suggest.session.as_mut().filter(|s| s.focused) else {
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
        // Already resolved, or still resolving (Monaco caches the item's
        // resolve promise).
        if session.resolved[index] || session.resolving == Some(index) {
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
        session.resolvable[index] = true;
        session.resolving = Some(index);
        session.details_loading = false;
        session.details_loading_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DETAILS_LOADING_DELAY).await;
            this.update(cx, |this, cx| {
                let Some(session) = this.suggest.session.as_mut() else {
                    return;
                };
                session.details_loading_task = None;
                if session.resolving == Some(index) {
                    session.details_loading = true;
                    this.refresh_suggest_widget(cx);
                }
            })
            .ok();
        }));
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
                session.resolving = None;
                session.details_loading = false;
                session.details_loading_task = None;
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
        // Typing goes to the text, so the focus is back in the list.
        if self.suggest.details_focused {
            self.suggest.details_focused = false;
            self.refresh_suggest_widget(cx);
        }
        // Typing triggers inline completions too.
        self.trigger_inline_completions(cx);
        let providers = completion_providers(cx, self.provider_target(cx));
        // Monaco `suppressSuggestions`: ghost text keeps the widget closed.
        if self.inline_suppresses_suggestions(cx) && self.suggest.session.is_none() {
            return;
        }
        // Monaco `_updateTriggerCharacters`: only the providers declaring
        // the character are asked; the open session's items from the others
        // are kept (`providerFilter`, `providerItemsToReuse`).
        let supports: Vec<_> = providers
            .into_iter()
            .filter(|p| p.trigger_characters().contains(&ch))
            .collect();
        // Monaco `checkTriggerCharacter`: not when it's a case for quick
        // suggestions (a word character that's also a trigger).
        if self.options.suggest_on_trigger_characters
            && !supports.is_empty()
            && !self.should_auto_trigger(cx)
        {
            let reuse = self
                .suggest
                .session
                .as_ref()
                .map(|session| {
                    session
                        .items_by_provider()
                        .into_iter()
                        .filter(|reused| {
                            !reused
                                .provider
                                .as_ref()
                                .is_some_and(|p| supports.iter().any(|s| same_provider(s, p)))
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.request_suggestions_with(
                CompletionTrigger::TriggerCharacter(ch),
                true,
                SuggestRequestOptions {
                    filter: Some(supports),
                    reuse,
                },
                cx,
            );
            return;
        }
        if self.suggest.session.is_some() {
            self.refilter_suggest(cx);
            return;
        }
        if self.suggest.task.is_some() {
            // Still computing: the results are filtered by what's typed
            // then, unless the word (by the language's `wordPattern`)
            // ended.
            let head = self.cursors.primary().position;
            if self.word_start_at(head, cx) == head.column {
                self.hide_suggest(cx);
            }
            return;
        }
        // Monaco quick suggestions, after any typing: after
        // `quick_suggestions_delay`, if the cursor ends a word by the
        // language's `wordPattern` (or its first character) and the token
        // type allows it (`quick_suggest`).
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
    fn should_auto_trigger(&self, cx: &gpui::App) -> bool {
        let head = self.cursors.primary().position;
        let (start, end) = self.word_columns_at(head, cx);
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
        use crate::options::QuickSuggestionsValue;
        // Cancel any wait for inline completions from a previous cycle.
        self.suggest.inline_wait = None;
        if self.suggest.session.is_some() || self.suggest.task.is_some() {
            return;
        }
        let quick = self.options.quick_suggestions;
        if quick.is_all_off() {
            return;
        }
        if self.options.suggest.snippets_prevent_quick_suggestions && self.in_snippet_mode() {
            return;
        }
        if !self.should_auto_trigger(cx) {
            return;
        }
        let mut wait_for_inline_completions = false;
        if !quick.is_all_on() {
            // The type of the token that triggered this.
            let head = self.cursors.primary().position;
            let offset = self.snapshot.point_utf16_to_offset(head).saturating_sub(1);
            let token: StandardTokenType = self.model().read(cx).token_type_at(offset);
            match quick.value_for(token) {
                // `Inline` shows as ghost text instead
                // (`suggest_inline_completions`).
                QuickSuggestionsValue::Off | QuickSuggestionsValue::Inline => return,
                QuickSuggestionsValue::OffWhenInlineCompletions => {
                    wait_for_inline_completions = self.options.inline_suggest.enabled
                        && !self.editor_inline_completions_providers(cx).is_empty();
                }
                QuickSuggestionsValue::On => {}
            }
        }
        // Monaco `canShowQuickSuggest`: not while ghost text asks to keep
        // the widget closed.
        if self.inline_suppresses_suggestions(cx) {
            return;
        }
        if wait_for_inline_completions {
            self.quick_suggest_after_inline_completions(cx);
        } else {
            self.request_suggestions(CompletionTrigger::Invoke, true, cx);
        }
    }

    /// Monaco `_waitForInlineCompletionsAndTrigger`: opens the suggest
    /// widget once inline completions are done loading, unless one shows;
    /// after 750ms it opens anyway and stops the inline completions.
    fn quick_suggest_after_inline_completions(&mut self, cx: &mut Context<Self>) {
        if self.inline_suggestion_visible(cx) {
            // Inline completions are already showing.
            return;
        }
        if !self.inline_completions_loading() {
            self.request_suggestions(CompletionTrigger::Invoke, true, cx);
            return;
        }
        let observer = cx.observe_self(|this, cx| {
            if this.suggest.inline_wait.is_none() || this.inline_completions_loading() {
                return;
            }
            let visible = this.inline_suggestion_visible(cx);
            this.finish_inline_wait(!visible, cx);
        });
        let timeout = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(INLINE_COMPLETIONS_WAIT)
                .await;
            this.update(cx, |this, cx| {
                if this.finish_inline_wait(true, cx) {
                    this.stop_inline_completions_automatically(cx);
                }
            })
            .ok();
        });
        self.suggest.inline_wait = Some(InlineWait {
            version: self.model().read(cx).buffer().version(),
            position: self.cursors.primary().position,
            _observer: observer,
            _timeout: timeout,
        });
    }

    /// Ends the wait for inline completions, opening the suggest widget if
    /// `trigger` and nothing changed meanwhile. Returns whether it was
    /// waiting.
    fn finish_inline_wait(&mut self, trigger: bool, cx: &mut Context<Self>) -> bool {
        let Some(wait) = self.suggest.inline_wait.take() else {
            return false;
        };
        if !trigger || self.suggest.session.is_some() || self.suggest.task.is_some() {
            return true;
        }
        if self.model().read(cx).buffer().version() == wait.version
            && self.cursors.primary().position == wait.position
        {
            self.request_suggestions(CompletionTrigger::Invoke, true, cx);
        }
        true
    }

    /// Monaco `SuggestModel._onNewContext`, after typing or deleting:
    /// closes, re-requests, or re-filters the open session.
    pub(crate) fn refilter_suggest(&mut self, cx: &mut Context<Self>) {
        let head = self.cursors.primary().position;
        let Some(session) = &self.suggest.session else {
            return;
        };
        if head.row != session.position.row
            || !row_has_leading_whitespace(
                self.snapshot.rope(),
                head.row,
                &session.leading_whitespace,
            )
        {
            // Another line (Enter), or the word was outdented.
            self.hide_suggest(cx);
            return;
        }
        let (auto, trigger_character) = (session.auto, session.trigger_character);
        let word_start = self.word_start_at(head, cx);
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
            // itself, keeping the items the providers gave (Monaco reuses
            // `getItemsByProvider`, so only providers without items are
            // asked again).
            if self.should_auto_trigger(cx) {
                let reuse = session.items_by_provider();
                self.request_suggestions_with(
                    CompletionTrigger::Invoke,
                    auto,
                    SuggestRequestOptions {
                        filter: None,
                        reuse,
                    },
                    cx,
                );
            }
            return;
        }
        if head.column > session.position.column
            && session
                .provider_incomplete
                .iter()
                .any(|incomplete| *incomplete)
            && on_word
        {
            // Monaco: only the providers with incomplete lists are asked
            // again; the others' items are kept.
            let (incomplete, complete): (Vec<_>, Vec<_>) = session
                .items_by_provider()
                .into_iter()
                .partition(|items| items.incomplete);
            let filter = incomplete
                .into_iter()
                .filter_map(|items| items.provider)
                .collect();
            self.request_suggestions_with(
                CompletionTrigger::Incomplete,
                auto,
                SuggestRequestOptions {
                    filter: Some(filter),
                    reuse: complete,
                },
                cx,
            );
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
        let no_focus = self.suggest_no_focus(auto, trigger_character, true);
        if let Some(session) = self.suggest.session.as_mut() {
            session.focused = !no_focus;
        }
        self.select_by_memory(cx);
        self.resolve_selected_suggestion(cx);
        self.refresh_suggest_widget(cx);
    }

    /// Monaco `SuggestController`'s `noFocus` for `suggest.selectionMode`:
    /// whether a session that opened by itself shows without a focused
    /// item. `refilter` is set when the list is filtered again after
    /// typing.
    fn suggest_no_focus(&self, auto: bool, trigger_character: bool, refilter: bool) -> bool {
        use crate::options::SuggestSelectionMode;
        if !auto {
            return false;
        }
        match self.options.suggest.selection_mode {
            SuggestSelectionMode::Always => false,
            SuggestSelectionMode::Never => true,
            SuggestSelectionMode::WhenTriggerCharacter => !trigger_character,
            SuggestSelectionMode::WhenQuickSuggestion => trigger_character && !refilter,
        }
    }

    /// Focuses the first suggestion if none is focused (Monaco
    /// `focusSuggestion`, Ctrl+Space while suggestions show). Returns
    /// whether it did.
    pub fn focus_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.suggest.session.as_ref() else {
            return false;
        };
        if session.focused || !self.is_suggest_visible() || session.model.filtered().is_empty() {
            return false;
        }
        self.select_suggestion(0, cx);
        true
    }

    /// Whether the suggest widget shows without a focused suggestion
    /// (see `suggest.selection_mode`).
    pub fn is_suggestion_unfocused(&self) -> bool {
        self.is_suggest_visible()
            && self
                .suggest
                .session
                .as_ref()
                .is_some_and(|session| !session.focused)
    }

    /// Toggles explain mode (Monaco `toggleExplainMode`, Cmd/Ctrl+/ while
    /// suggestions show): the details panel shows how the focused
    /// suggestion was scored and sorted. Returns whether a suggestion was
    /// focused.
    pub fn toggle_suggest_explain_mode(&mut self, cx: &mut Context<Self>) -> bool {
        if self.selected_suggestion().is_none() || !self.is_suggest_visible() {
            return false;
        }
        self.suggest.explain_mode = !self.suggest.explain_mode;
        // Monaco opens the details panel if it's closed.
        if !self.is_suggestion_details_visible(cx) {
            cx.default_global::<SuggestUi>().details_expanded = true;
        }
        self.refresh_suggest_widget(cx);
        true
    }

    /// Whether explain mode is on (see
    /// [`CodeEditor::toggle_suggest_explain_mode`]).
    pub fn is_suggest_explain_mode(&self) -> bool {
        self.suggest.explain_mode
    }

    /// Forgets the size the suggest widget was resized to (Monaco
    /// `editor.action.resetSuggestSize`).
    pub fn reset_suggest_size(&mut self, cx: &mut Context<Self>) {
        if cx
            .try_global::<SuggestUi>()
            .is_some_and(|ui| ui.size.is_some())
        {
            cx.global_mut::<SuggestUi>().size = None;
        }
        if self.is_suggest_visible() {
            self.refresh_suggest_widget(cx);
        }
    }

    /// Monaco `SuggestDetailsWidget.renderItem` in explain mode: the
    /// provider as the detail, and the focused item's scoring as a code
    /// block.
    fn suggest_explanation(&self) -> Option<(SharedString, String)> {
        let session = self.suggest.session.as_ref().filter(|s| s.focused)?;
        let scored = session.model.filtered().get(session.selected)?;
        let item = &session.model.items()[scored.index];
        let head = self.cursors.primary().position;
        let start = item
            .range
            .as_ref()
            .filter(|r| r.start.row == head.row)
            .map_or(session.word_start, |r| r.start.column);
        let prefix: String = self
            .snapshot
            .line(head.row)
            .encode_utf16()
            .skip(start as usize)
            .take(head.column.saturating_sub(start) as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or('\u{fffd}'))
            .collect();
        let prefix = if prefix.is_empty() {
            "(no prefix)".to_string()
        } else {
            prefix
        };
        let word = match &item.filter_text {
            Some(filter) => format!("{filter} (filterText)"),
            None => item.label.to_string(),
        };
        let based_on = match &item.sort_text {
            Some(sort) => format!("sortText: \"{sort}\""),
            None => "label".to_string(),
        };
        let commit: String = item.commit_characters.iter().collect();
        let markdown = format!(
            "```empty\nscore: {}\nprefix: {prefix}\nword: {word}\ndistance: {} (localityBonus-setting)\nindex: {}, based on {based_on}\ncommit_chars: {commit}\n```",
            scored.score.score,
            session.model.distance(scored.index),
            scored.index,
        );
        let provider = match session.item_providers[scored.index] {
            Some(ix) => session.providers[ix].debug_display_name(),
            None => "wordbasedCompletions".into(),
        };
        Some((format!("Provider: {provider}").into(), markdown))
    }

    /// Moves the selection by `delta` rows (Up/Down/Page). Returns whether
    /// the widget handled it.
    pub(crate) fn suggest_navigate(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        // Monaco's `State.Details`: the keys scroll the focused details.
        if self.suggest.details_focused {
            use crate::suggest_widget::DetailsScroll;
            let scroll = if delta < 0 {
                DetailsScroll::Up
            } else {
                DetailsScroll::Down
            };
            return self.scroll_suggestion_details(scroll, cx);
        }
        // "Loading..." (only shown for explicit requests) and "No
        // suggestions." keep `suggestWidgetVisible` set with nothing
        // focused, so Monaco's select commands run and do nothing
        // (`SuggestWidget.selectNext` in the `Loading` and `Empty` states):
        // the keys don't move the cursor.
        if self
            .suggest
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).message().is_some())
        {
            return true;
        }
        let Some(session) = self.suggest.session.as_mut() else {
            return false;
        };
        let count = session.model.filtered().len();
        if count == 0 || self.suggest.widget.is_none() {
            return false;
        }
        // Monaco's `selectNextSuggestion` and the others need several
        // suggestions or none focused (`suggestWidgetMultipleSuggestions ||
        // !suggestWidgetHasFocusedSuggestion`); otherwise the keys move the
        // cursor, which closes the widget.
        if count == 1 && session.focused {
            return false;
        }
        // Monaco's list moves from no focus as if from before the first
        // row (forward) or from the first (back).
        let current = if session.focused {
            session.selected as isize
        } else if delta > 0 {
            -1
        } else {
            0
        };
        session.focused = true;
        let selected = current + delta;
        session.selected = if delta.abs() == 1 {
            selected.rem_euclid(count as isize) as usize
        } else {
            selected.clamp(0, count as isize - 1) as usize
        };
        self.resolve_selected_suggestion(cx);
        self.refresh_suggest_widget(cx);
        true
    }

    /// Moves the selection a page down or up (Monaco
    /// `selectNextPageSuggestion`, `selectPrevPageSuggestion`); with the
    /// details panel focused, pages it instead. Returns whether the widget
    /// handled it.
    pub(crate) fn suggest_navigate_page(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        if self.suggest.details_focused {
            use crate::suggest_widget::DetailsScroll;
            let scroll = if down {
                DetailsScroll::PageDown
            } else {
                DetailsScroll::PageUp
            };
            return self.scroll_suggestion_details(scroll, cx);
        }
        let page = self.suggest_page_size(cx) as isize;
        self.suggest_navigate(if down { page } else { -page }, cx)
    }

    pub(crate) fn select_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(session) = self.suggest.session.as_mut() {
            session.selected = index;
            session.focused = true;
        }
        self.resolve_selected_suggestion(cx);
        self.refresh_suggest_widget(cx);
    }

    /// Selects the first suggestion (Monaco `selectFirstSuggestion`); with
    /// the details panel focused, scrolls it to the top.
    pub fn select_first_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        if self.scroll_suggestion_details(crate::suggest_widget::DetailsScroll::Top, cx) {
            return true;
        }
        if !self.is_suggest_visible() {
            return false;
        }
        self.select_suggestion(0, cx);
        true
    }

    /// Selects the last suggestion (Monaco `selectLastSuggestion`); with
    /// the details panel focused, scrolls it to the bottom.
    pub fn select_last_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        if self.scroll_suggestion_details(crate::suggest_widget::DetailsScroll::Bottom, cx) {
            return true;
        }
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
    /// was accepted. With `tab_completion` on, Tab and Shift+Tab then
    /// replace it with the other suggestions (Monaco's
    /// `KeepAlternativeSuggestions`).
    pub fn accept_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        self.accept_suggestion_with(false, true, cx)
    }

    /// Accepts the selected suggestion in the other insert mode (Monaco
    /// `acceptAlternativeSelectedSuggestion`, Shift+Enter or Shift+Tab).
    /// Unlike [`Self::accept_suggestion`], Tab doesn't cycle through the
    /// other suggestions afterwards.
    pub fn accept_alternative_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        self.accept_suggestion_with(true, false, cx)
    }

    /// Accepts the selected suggestion without keeping the others for Tab,
    /// as Monaco does for clicks (`onDidSelect`) and commit characters.
    pub(crate) fn accept_suggestion_only(&mut self, cx: &mut Context<Self>) -> bool {
        self.accept_suggestion_with(false, false, cx)
    }

    /// Enter with the widget open: accepts unless
    /// `accept_suggestion_on_enter` says not to (then Enter types a new
    /// line).
    pub(crate) fn accept_suggestion_on_enter(&mut self, cx: &mut Context<Self>) -> bool {
        // Monaco's accept commands need a focused suggestion.
        if self.selected_suggestion().is_none() {
            return false;
        }
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

    /// Monaco's `suggestionMakesTextEdit` context key: with `smart` Enter,
    /// [`Self::suggestion_makes_text_edit`] for the focused suggestion;
    /// otherwise (and with none focused) true.
    pub(crate) fn suggestion_makes_text_edit_key(&self) -> bool {
        self.options.accept_suggestion_on_enter != AcceptSuggestionOnEnter::Smart
            || self.selected_suggestion().is_none()
            || self.suggestion_makes_text_edit()
    }

    /// Monaco's `SuggestContext.AcceptSuggestionsOnEnter &&
    /// SuggestContext.MakesTextEdit`: whether Enter accepts the focused
    /// suggestion (the status bar's "Insert" shows Enter's key, else Tab's).
    pub(crate) fn suggestion_accepts_on_enter(&self) -> bool {
        self.options.accept_suggestion_on_enter != AcceptSuggestionOnEnter::Off
            && self.suggestion_makes_text_edit_key()
    }

    /// Monaco `SuggestController.forceRenderingAbove` (with `force`) and
    /// `stopForceRenderingAbove`.
    pub(crate) fn force_suggest_rendering_above(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some((widget, _)) = &self.suggest.widget else {
            return;
        };
        if widget.read(cx).forced_above() == force {
            return;
        }
        widget.update(cx, |widget, cx| {
            if force {
                widget.force_rendering_above(cx);
            } else {
                widget.stop_force_rendering_above();
            }
        });
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

    fn accept_suggestion_with(
        &mut self,
        toggle: bool,
        keep_alternatives: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        // Monaco's accept commands need a focused suggestion.
        if self.suggest.session.as_ref().is_some_and(|s| !s.focused) {
            return false;
        }
        // Monaco's `onWillInsertSuggestItem` for inline completions.
        let suggest_edit = self.selected_suggestion_edit();
        let Some(session) = self.suggest.session.take() else {
            return false;
        };
        let Some(scored) = session.model.filtered().get(session.selected) else {
            self.hide_suggest(cx);
            return false;
        };
        if let Some((range, text)) = suggest_edit {
            self.inline_suggest_accepted(range, text, cx);
        }
        let item: CompletionItem = session.model.items()[scored.index].clone();
        let selected = session.selected;
        let provider = session.item_providers[scored.index].map(|p| session.providers[p].clone());
        let resolved = session.resolved[scored.index];
        // Monaco memorizes before inserting.
        let (language, line, column, word_until) = self.memory_context(cx);
        let mode = self.options.suggest_selection;
        let scope = self.memory_scope();
        cx.default_global::<Memories>().get(scope).memorize(
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
        // Monaco keeps the other items for Tab and Shift+Tab, after Tab or
        // Enter (`KeepAlternativeSuggestions`).
        if keep_alternatives && self.options.tab_completion == crate::options::TabCompletion::On {
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
            && let Some(inserted) = self.model().read(cx).last_transaction_id()
        {
            self.suggest.late_edits = Some(cx.spawn(async move |this, cx| {
                let Ok(resolved) = task.await else {
                    return;
                };
                this.update(cx, |this, cx| {
                    this.apply_late_edits(resolved, version, inserted, cx)
                })
                .ok();
            }));
        }
        // Monaco's inline completions controller asks again after
        // `acceptSelectedSuggestion`.
        self.trigger_inline_completions(cx);
        // Monaco runs the item's command after inserting.
        if let Some(command) = &item.command {
            if command.id == crate::snippet_session::NEXT_PLACEHOLDER_COMMAND {
                self.next_snippet_placeholder(cx);
            } else if command.id == "editor.action.triggerSuggest" {
                self.request_suggestions(CompletionTrigger::Invoke, true, cx);
            } else {
                self.run_provider_command(command.clone(), cx);
            }
        }
        // Monaco `_alertCompletionItem`.
        if !item.additional_text_edits.is_empty() {
            self.announce(
                format!(
                    "Accepting '{}' made {} additional edits",
                    item.label,
                    item.additional_text_edits.len()
                ),
                cx,
            );
        }
        true
    }

    /// Applies additional edits resolved after inserting, unless one starts
    /// after the earliest change since `version` (from before inserting):
    /// Monaco drops them then, since only text before every change still
    /// has its old positions. They join the insertion's undo step
    /// (`inserted`) unless another edit came between.
    fn apply_late_edits(
        &mut self,
        item: CompletionItem,
        version: text::Version,
        inserted: text::TransactionId,
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
            // Monaco inserts an unresolved item without an undo stop after
            // it, so the resolved edits join the insertion's undo step
            // unless the user typed in between (which pushes the stop).
            if model.last_transaction_id() == Some(inserted) {
                model.pop_stack_element();
            } else {
                model.push_stack_element();
            }
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
        // Monaco adds the `LineSuffix` delta: after moving right without
        // typing, the text the cursor passed isn't overwritten again.
        let suffix_delta = self.suggest.line_suffix.map_or(0, |suffix| {
            suffix.delta(self.snapshot.rope().line_len_utf16(head.row), head)
        });
        let overwrite_after = (i64::from(end) - i64::from(position.column) + suffix_delta)
            .clamp(0, i64::from(u32::MAX)) as u32;
        // Monaco inserts every item as a snippet (plain text escaped), after
        // its additional edits, as one undo step.
        self.model().update(cx, |model, _| {
            model.push_stack_element();
            model.start_transaction();
        });
        if !item.additional_text_edits.is_empty() {
            // Monaco shifts an edit on the request's line that starts after
            // the request position by the text typed since the request.
            let delta = head.column as i64 - position.column as i64;
            let shift = |column: u32| (column as i64 + delta).max(0) as u32;
            let edits: Vec<_> = item
                .additional_text_edits
                .iter()
                .map(|(range, text)| {
                    let mut range = range.clone();
                    if range.start.row == position.row && range.start.column > position.column {
                        let multi_line = range.end.row != range.start.row;
                        range.start.column = shift(range.start.column);
                        if !multi_line {
                            range.end.column = shift(range.end.column);
                        }
                    }
                    (range, text.clone())
                })
                .collect();
            self.apply_additional_text_edits(&edits, cx);
        }
        let template = if item.insert_text_is_snippet {
            item.insert_text().to_string()
        } else {
            crate::snippet::parser::SnippetParser::escape(item.insert_text())
        };
        // Monaco passes its `OvertypingCapturer` to the snippet controller.
        self.suggest.snippet_overtyped = Some(self.suggest.overtyped.clone());
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
        self.suggest.snippet_overtyped = None;
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
        // Monaco accepts only while the widget isn't frozen.
        let Some(session) = self
            .suggest
            .session
            .as_ref()
            .filter(|s| s.focused && !s.frozen)
        else {
            return false;
        };
        let Some(scored) = session.model.filtered().get(session.selected) else {
            return false;
        };
        if session.model.items()[scored.index]
            .commit_characters
            .contains(&ch)
        {
            self.accept_suggestion_only(cx)
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
