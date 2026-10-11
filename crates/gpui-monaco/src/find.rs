//! Find and replace (Monaco's `contrib/find`): the find state, the find
//! model bound to the text while the widget shows, and its decorations.
//!
//! Ports `findModel.ts` (`FindModelBoundToEditorModel`), `findDecorations.ts`
//! and the navigation parts of `findState.ts` (MIT). Rows are 0-based and
//! columns UTF-16, where Monaco's are 1-based.

use std::ops::Range;
use std::time::Duration;

use gpui::{AppContext as _, Context, Entity, Task, Window};
use text::PointUtf16;

use crate::cursor::{self, Selection};
use crate::decorations::{DecorationId, DecorationOptions, OverviewRulerLane};
use crate::editor::CodeEditor;
use crate::events::CursorChangeReason;
use crate::find_widget::FindWidget;
use crate::replace_pattern::{ReplacePattern, parse_replace_string};
use crate::search::{self, SearchData, SearchParams};
use crate::widgets::{OverlayPosition, OverlayWidget, WidgetId};

/// Monaco highlights at most this many matches.
pub const MATCHES_LIMIT: usize = 19_999;

/// Monaco's `_updateDecorationsScheduler` delay after an edit.
const RESEARCH_AFTER_EDIT: Duration = Duration::from_millis(100);

/// The find widget's inputs and toggles (Monaco `FindReplaceState`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    /// The search string.
    pub query: String,
    /// The replacement. In regex mode `$1`, `$&`, `$$` and `\u` `\U` `\l`
    /// `\L` (case changes) work as in Monaco.
    pub replace: String,
    /// Treat `query` as a regular expression.
    pub is_regex: bool,
    /// Match case exactly.
    pub match_case: bool,
    /// Only match whole words, as delimited by
    /// [`EditorOptions::word_separators`](crate::EditorOptions::word_separators).
    pub whole_word: bool,
    /// Search only within the selections captured when this was turned on.
    pub in_selection: bool,
    /// Keep the case of what's replaced (Monaco `preserveCase`): `abc` →
    /// `def`, `Abc` → `Def`, `ABC` → `DEF`.
    pub preserve_case: bool,
}

/// Monaco `FindDecorations` plus what `FindModelBoundToEditorModel` keeps:
/// exists while the widget shows.
#[derive(Default)]
struct FindModel {
    /// One per match, in order (at most [`MATCHES_LIMIT`]).
    matches: Vec<DecorationId>,
    /// Merged overview ruler marks when there are over 1000 matches.
    overview: Vec<DecorationId>,
    scopes: Vec<DecorationId>,
    range_highlight: Option<DecorationId>,
    highlighted: Option<DecorationId>,
    /// Where incremental search goes from.
    start_position: PointUtf16,
    /// The text version the model's own last edit produced, whose change
    /// event it ignores (Monaco `_ignoreModelContentChanged`), and whether
    /// to re-search then (the editor sees the new text only at that event).
    own_edit: Option<(text::Version, bool)>,
    research_task: Option<Task<()>>,
}

#[derive(Default)]
pub(crate) struct FindState {
    pub options: FindOptions,
    pub visible: bool,
    pub replace_visible: bool,
    /// Monaco `searchScope`: the selections find in selection searches.
    search_scope: Option<Vec<Range<PointUtf16>>>,
    /// The current match's 1-based position, 0 when unknown.
    pub matches_position: usize,
    pub matches_count: usize,
    model: Option<FindModel>,
    widget: Option<(Entity<FindWidget>, WidgetId)>,
    /// Monaco's `FindWidgetViewZone` (`find.addExtraSpaceOnTop`) and its
    /// height.
    zone: Option<(WidgetId, gpui::Pixels)>,
    /// The toggles' own values while a Cmd/Ctrl+D session started from a
    /// word overrides them (Monaco's `FindOptionOverride`s, whose
    /// `actualIsRegex`, `actualWholeWord` and `actualMatchCase` these are):
    /// `options` holds the overridden values until the next change.
    overridden: Option<FindToggles>,
    /// The last status read to screen readers (Monaco's find widget alerts
    /// each new matches count).
    last_alert: Option<String>,
    /// The widget's find and replace inputs, whose heights make its own.
    inputs: Option<(Entity<CodeEditor>, Entity<CodeEditor>)>,
    /// The widget's height as last painted, with the height it was laid
    /// out for.
    measured: Option<(gpui::Pixels, gpui::Pixels)>,
}

/// How `find_start` seeds the search string (Monaco
/// `seedSearchStringFromSelection` in `IFindStartOptions`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SeedMode {
    #[default]
    None,
    Single,
    Multiple,
}

/// Monaco `FindStartFocusAction`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StartFocus {
    #[default]
    NoChange,
    FindInput,
    ReplaceInput,
}

/// Monaco `IFindStartOptions`.
#[derive(Clone, Copy, Debug, Default)]
struct FindStart {
    force_reveal_replace: bool,
    seed: SeedMode,
    seed_non_empty_only: bool,
    /// Monaco `seedSearchStringFromGlobalClipboard`: without a seed from
    /// the selection, seed from the find clipboard (with
    /// `find.global_find_clipboard`).
    seed_from_global_clipboard: bool,
    focus: StartFocus,
    update_search_scope: bool,
}

/// The shared find string where there's no system find pasteboard (Monaco's
/// browser clipboard service keeps its `findText` in memory).
#[derive(Default)]
struct FindClipboard(String);

impl gpui::Global for FindClipboard {}

/// Monaco `IClipboardService.readFindText`: macOS's find pasteboard, else
/// the app's find string.
fn read_find_text(cx: &gpui::App) -> String {
    #[cfg(target_os = "macos")]
    if let Some(item) = cx.read_from_find_pasteboard() {
        return item.text().unwrap_or_default();
    }
    cx.try_global::<FindClipboard>()
        .map(|clipboard| clipboard.0.clone())
        .unwrap_or_default()
}

/// Monaco `IClipboardService.writeFindText`.
fn write_find_text(text: &str, cx: &mut gpui::App) {
    #[cfg(target_os = "macos")]
    cx.write_to_find_pasteboard(gpui::ClipboardItem::new_string(text.to_string()));
    cx.default_global::<FindClipboard>().0 = text.to_string();
}

/// The search and replace strings entered in any editor's find widget
/// (Monaco's `FindWidgetSearchHistory` and `ReplaceWidgetHistory`), oldest
/// first, without repeats.
#[derive(Default)]
pub(crate) struct FindHistory {
    pub find: Vec<String>,
    pub replace: Vec<String>,
}

impl gpui::Global for FindHistory {}

impl FindHistory {
    /// Adds `value` as the newest entry.
    pub fn add(list: &mut Vec<String>, value: &str) {
        if value.is_empty() {
            return;
        }
        list.retain(|v| v != value);
        list.push(value.to_string());
    }
}

/// The toggles find remembers across editors (Monaco stores
/// `editor.isRegex`, `editor.matchCase`, `editor.wholeWord` and
/// `editor.preserveCase` in workspace storage).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FindToggles {
    is_regex: bool,
    match_case: bool,
    whole_word: bool,
    preserve_case: bool,
}

impl gpui::Global for FindToggles {}

impl FindToggles {
    fn of(options: &FindOptions) -> Self {
        Self {
            is_regex: options.is_regex,
            match_case: options.match_case,
            whole_word: options.whole_word,
            preserve_case: options.preserve_case,
        }
    }

    /// Monaco `loadQueryState`.
    pub(crate) fn load(options: &mut FindOptions, cx: &gpui::App) {
        let toggles = cx.try_global::<Self>().copied().unwrap_or_default();
        options.is_regex = toggles.is_regex;
        options.match_case = toggles.match_case;
        options.whole_word = toggles.whole_word;
        options.preserve_case = toggles.preserve_case;
    }
}

/// An empty view for the find widget's view zone.
struct FindZoneSpacer;

impl gpui::Render for FindZoneSpacer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

impl FindState {
    /// With the remembered toggles (Monaco loads them when the controller
    /// is created).
    pub(crate) fn new(cx: &gpui::App) -> Self {
        let mut state = Self::default();
        FindToggles::load(&mut state.options, cx);
        state
    }

    /// "3 of 15", "No results", or `None` when there's no query (Monaco's
    /// matches count label).
    pub fn status(&self) -> Option<String> {
        if self.options.query.is_empty() {
            return None;
        }
        if self.matches_count == 0 {
            return Some("No results".into());
        }
        let total = if self.matches_count >= MATCHES_LIMIT {
            format!("{MATCHES_LIMIT}+")
        } else {
            self.matches_count.to_string()
        };
        Some(match self.matches_position {
            0 => format!("? of {total}"),
            position => format!("{position} of {total}"),
        })
    }

    /// Monaco `canNavigateBack`.
    pub(crate) fn can_navigate_back(&self, loop_: bool) -> bool {
        self.can_navigate_in_loop(loop_) || self.matches_position != 1
    }

    /// Monaco `canNavigateForward`.
    pub(crate) fn can_navigate_forward(&self, loop_: bool) -> bool {
        self.can_navigate_in_loop(loop_) || self.matches_position < self.matches_count
    }

    fn can_navigate_in_loop(&self, loop_: bool) -> bool {
        loop_ || self.matches_count >= MATCHES_LIMIT
    }
}

fn match_options(colors: &crate::theme::EditorColors, overview: bool) -> DecorationOptions {
    DecorationOptions {
        background: Some(colors.find_match_highlight_background),
        // High contrast outlines matches; `findMatch*Foreground` recolors
        // their text.
        border: colors.find_match_highlight_border,
        text: colors
            .find_match_highlight_foreground
            .map(|foreground| crate::theme::TokenStyle {
                foreground: Some(foreground),
                ..Default::default()
            }),
        overview_ruler: overview.then_some((colors.find_match_overview, OverviewRulerLane::Center)),
        // Monaco's `_FIND_MATCH_NO_OVERVIEW_DECORATION` has no minimap mark.
        minimap: overview.then(|| find_match_minimap(colors)),
        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
        z_index: 10,
        ..Default::default()
    }
}

fn current_match_options(colors: &crate::theme::EditorColors) -> DecorationOptions {
    DecorationOptions {
        background: Some(colors.find_match_background),
        border: colors.find_match_border,
        text: colors
            .find_match_foreground
            .map(|foreground| crate::theme::TokenStyle {
                foreground: Some(foreground),
                ..Default::default()
            }),
        overview_ruler: Some((colors.find_match_overview, OverviewRulerLane::Center)),
        minimap: Some(find_match_minimap(colors)),
        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
        z_index: 13,
        ..Default::default()
    }
}

/// Monaco's find match minimap mark: `minimap.findMatchHighlight`, inline.
fn find_match_minimap(colors: &crate::theme::EditorColors) -> crate::MinimapDecoration {
    crate::MinimapDecoration {
        color: colors.minimap_find_match_highlight,
        position: crate::MinimapPosition::Inline,
        ..Default::default()
    }
}

/// Monaco `Range.compareRangesUsingStarts`.
fn compare_using_starts(a: &Range<PointUtf16>, b: &Range<PointUtf16>) -> std::cmp::Ordering {
    a.start.cmp(&b.start).then(a.end.cmp(&b.end))
}

fn contains_range(outer: &Range<PointUtf16>, inner: &Range<PointUtf16>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

impl CodeEditor {
    /// The find widget's current query and toggles.
    pub fn find_options(&self) -> &FindOptions {
        &self.find.options
    }

    /// Whether the find widget is open.
    pub fn is_find_visible(&self) -> bool {
        self.find.visible
    }

    /// The find state the widget shows (matches, navigation).
    pub(crate) fn find_state(&self) -> &FindState {
        &self.find
    }

    fn find_widget(&self) -> Option<&Entity<FindWidget>> {
        self.find.widget.as_ref().map(|(widget, _)| widget)
    }

    /// The find widget's layout in the last frame: (width, input width,
    /// reduced, narrow, collapsed), for tests.
    #[doc(hidden)]
    pub fn find_widget_layout_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(gpui::Pixels, gpui::Pixels, bool, bool, bool)> {
        Some(self.find_widget()?.read(cx).layout_for_tests())
    }

    /// The title of the find widget control `id` (`"match-case"`,
    /// `"previous"`, `"close"`, …), for tests.
    #[doc(hidden)]
    pub fn find_widget_title_for_tests(
        &self,
        id: &str,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<String> {
        self.find_widget()?.read(cx).title_for_tests(id, window, cx)
    }

    /// Whether the find widget control `id` is shown and enabled, for
    /// tests.
    #[doc(hidden)]
    pub fn find_widget_control_enabled_for_tests(&self, id: &str, cx: &gpui::App) -> bool {
        self.find_widget()
            .is_some_and(|w| w.read(cx).is_enabled_for_tests(id, cx))
    }

    /// Whether the find input, the replace input and the widget have the
    /// focus, for tests.
    #[doc(hidden)]
    pub fn find_widget_focus_for_tests(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> (bool, bool, bool) {
        self.find_widget_focus(window, cx)
    }

    /// The focused find widget control's id, for tests.
    #[doc(hidden)]
    pub fn find_widget_focused_control_for_tests(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<&'static str> {
        self.find_widget()?
            .read(cx)
            .focused_control_for_tests(window)
    }

    /// Monaco `findInputFocussed`, `replaceInputFocussed` and
    /// `findWidgetFocused`.
    pub(crate) fn find_widget_focus(&self, window: &Window, cx: &gpui::App) -> (bool, bool, bool) {
        match &self.find.widget {
            Some((widget, _)) if self.find.visible => widget.read(cx).focus_state(window, cx),
            _ => (false, false, false),
        }
    }

    /// The find widget's match counter text.
    pub fn find_status(&self) -> Option<String> {
        self.find.status()
    }

    /// The highlighted matches (at most [`MATCHES_LIMIT`]), in byte offsets.
    pub fn find_matches(&self, cx: &gpui::App) -> Vec<Range<usize>> {
        let Some(model) = &self.find.model else {
            return Vec::new();
        };
        let text = self.model().read(cx);
        model
            .matches
            .iter()
            .filter_map(|id| Some(text.decoration(*id)?.range.clone()))
            .collect()
    }

    fn search_params(&self) -> SearchParams {
        let options = &self.find.options;
        SearchParams {
            query: options.query.clone(),
            is_regex: options.is_regex,
            match_case: options.match_case,
            word_separators: options
                .whole_word
                .then(|| self.options.word_separators.to_string()),
        }
    }

    fn search_data(&self) -> Option<SearchData> {
        self.search_params().parse()
    }

    /// Updates the query and toggles, moving to the first match from where
    /// the search started (Monaco's `state.change(…, moveCursor: true)`).
    pub fn set_find_options(&mut self, options: FindOptions, cx: &mut Context<Self>) {
        self.change_find_options(options, true, cx);
    }

    /// Updates the query and toggles (Monaco `FindReplaceState.change`);
    /// `move_cursor` moves to the first match from where the search started.
    pub fn change_find_options(
        &mut self,
        options: FindOptions,
        move_cursor: bool,
        cx: &mut Context<Self>,
    ) {
        let mut options = options;
        // Monaco resets the overrides on any change: the toggles the change
        // leaves alone get their own values back.
        if let Some(actual) = self.find.overridden.take() {
            let current = &self.find.options;
            if options.is_regex == current.is_regex {
                options.is_regex = actual.is_regex;
            }
            if options.whole_word == current.whole_word {
                options.whole_word = actual.whole_word;
            }
            if options.match_case == current.match_case {
                options.match_case = actual.match_case;
            }
        }
        self.apply_find_options(options, move_cursor, cx);
    }

    /// Monaco's multicursor `_beginSessionIfNeeded`: Cmd/Ctrl+D and
    /// Cmd/Ctrl+Shift+L make `search` the search string, so the find
    /// widget and F3 see it. A session started from a word also overrides
    /// whole word and match case on and regex off until the next change
    /// (Monaco `wholeWordOverride`, `matchCaseOverride`, `isRegexOverride`).
    pub(crate) fn seed_find_from_multi_cursor(
        &mut self,
        search: String,
        from_word: bool,
        cx: &mut Context<Self>,
    ) {
        let actual = self
            .find
            .overridden
            .take()
            .unwrap_or_else(|| FindToggles::of(&self.find.options));
        let mut options = self.find.options.clone();
        options.query = search;
        options.is_regex = actual.is_regex;
        options.whole_word = actual.whole_word;
        options.match_case = actual.match_case;
        if from_word {
            options.is_regex = false;
            options.whole_word = true;
            options.match_case = true;
            self.find.overridden = Some(actual);
        }
        self.apply_find_options(options.clone(), false, cx);
        let replace = self.find.replace_visible;
        // Deferred: the command may come from the widget itself.
        if let Some((widget, _)) = &self.find.widget {
            let widget = widget.downgrade();
            cx.defer(move |cx| {
                widget
                    .update(cx, |widget, cx| widget.set_inputs(&options, replace, cx))
                    .ok();
            });
        }
    }

    /// Monaco's multicursor `_endSession`: the toggles a session overrode
    /// get their own values back.
    pub(crate) fn clear_find_overrides(&mut self, cx: &mut Context<Self>) {
        if self.find.overridden.is_some() {
            let options = self.find.options.clone();
            self.change_find_options(options, false, cx);
        }
    }

    fn apply_find_options(
        &mut self,
        options: FindOptions,
        move_cursor: bool,
        cx: &mut Context<Self>,
    ) {
        // Monaco's controller: turning on find in selection captures the
        // selections (whole lines when they span lines).
        let mut options = options;
        let mut scope_changed = false;
        if options.in_selection != self.find.options.in_selection {
            let scope = options.in_selection.then(|| self.selection_scopes(cx));
            match scope {
                // Monaco `toggleSearchScope` needs a non-empty selection.
                Some(scope) if scope.is_empty() => options.in_selection = false,
                scope => {
                    self.find.search_scope = scope;
                    scope_changed = true;
                }
            }
        }
        // Monaco `saveQueryState`: the toggles' own values, not overrides.
        let actual = |options: &FindOptions, overridden: Option<FindToggles>| {
            let mut toggles = FindToggles::of(options);
            if let Some(o) = overridden {
                (toggles.is_regex, toggles.whole_word, toggles.match_case) =
                    (o.is_regex, o.whole_word, o.match_case);
            }
            toggles
        };
        let saved = actual(&options, self.find.overridden);
        if saved != actual(&self.find.options, self.find.overridden) {
            cx.set_global(saved);
        }
        let old = std::mem::replace(&mut self.find.options, options);
        let new = &self.find.options;
        let query_changed = new.query != old.query;
        let replace_changed = new.replace != old.replace;
        let research = query_changed
            || new.is_regex != old.is_regex
            || new.match_case != old.match_case
            || new.whole_word != old.whole_word;
        if research || scope_changed {
            let scope = scope_changed.then(|| self.find.search_scope.clone());
            self.find_research(move_cursor, scope, cx);
        }
        // Monaco's controller `_onStateChanged`.
        if query_changed {
            let query = self.find.options.query.clone();
            self.set_global_find_term(&query, cx);
        }
        // Multi-line inputs make the widget taller (Monaco
        // `_tryUpdateHeight`).
        if self.find.visible && (query_changed || replace_changed) {
            self.update_find_view_zone(cx);
        }
        cx.notify();
    }

    /// Monaco `getGlobalBufferTerm`: the find clipboard's text with
    /// `find.global_find_clipboard`, else empty.
    fn global_find_term(&self, cx: &gpui::App) -> String {
        if self.options.find.global_find_clipboard {
            read_find_text(cx)
        } else {
            String::new()
        }
    }

    /// Monaco `setGlobalBufferTerm`: writes `text` to the find clipboard
    /// with `find.global_find_clipboard`.
    fn set_global_find_term(&self, text: &str, cx: &mut gpui::App) {
        if self.options.find.global_find_clipboard {
            write_find_text(text, cx);
        }
    }

    /// The editor (or a widget in it) got the focus: with
    /// `find.global_find_clipboard`, the open find widget takes a different
    /// search string from the find clipboard and selects it (Monaco's find
    /// widget on `onDidFocusEditorWidget`).
    pub(crate) fn find_widget_focus_in(&mut self, cx: &mut Context<Self>) {
        if !self.find.visible {
            return;
        }
        let term = self.global_find_term(cx);
        if term.is_empty() || term == self.find.options.query {
            return;
        }
        let mut options = self.find.options.clone();
        options.query = term;
        self.change_find_options(options.clone(), false, cx);
        let replace = self.find.replace_visible;
        if let Some((widget, _)) = &self.find.widget {
            widget.update(cx, |widget, cx| {
                widget.set_inputs(&options, replace, cx);
                widget.select_find_input(cx);
            });
        }
    }

    /// Sets the find scope (Monaco `searchScope`): search only these ranges,
    /// or everywhere with `None`; `move_cursor` moves to the first match from
    /// where the search started.
    pub fn set_find_search_scope(
        &mut self,
        scope: Option<Vec<Range<PointUtf16>>>,
        move_cursor: bool,
        cx: &mut Context<Self>,
    ) {
        self.find.options.in_selection = scope.is_some();
        self.find.search_scope = scope.clone();
        self.find_research(move_cursor, Some(scope), cx);
        cx.notify();
    }

    /// The non-empty selections as find scopes (Monaco's controller: a
    /// selection ending at a line start ends on the line before).
    fn selection_scopes(&self, cx: &gpui::App) -> Vec<Range<PointUtf16>> {
        let rope = self.model().read(cx).buffer().rope().clone();
        self.cursors
            .selections()
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| {
                let (start, mut end) = (s.start(), s.end());
                if end.row > start.row && end.column == 0 {
                    end = PointUtf16::new(end.row - 1, rope.line_len_utf16(end.row - 1));
                }
                start..end
            })
            .collect()
    }

    // ----- the find model ---------------------------------------------------------

    /// Binds the find model to the text (Monaco creates it when the widget
    /// shows), searching right away.
    pub(crate) fn start_find_model(&mut self, cx: &mut Context<Self>) {
        if self.find.model.is_some() {
            return;
        }
        self.find.model = Some(FindModel {
            start_position: self.cursors.primary().position,
            ..Default::default()
        });
        let scope = self.find.search_scope.clone();
        self.find_research(false, Some(scope), cx);
    }

    /// Unbinds the find model, removing its decorations.
    fn stop_find_model(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.find.model.take() else {
            return;
        };
        let mut old = model.matches;
        old.extend(model.overview);
        old.extend(model.scopes);
        old.extend(model.range_highlight);
        let owner = Some(cx.entity_id());
        self.model().update(cx, |text, cx| {
            text.delta_decorations(owner, &old, Vec::new(), cx);
        });
        self.find.matches_position = 0;
        self.find.matches_count = 0;
    }

    fn find_model(&self) -> Option<&FindModel> {
        self.find.model.as_ref()
    }

    fn match_ranges(&self, cx: &gpui::App) -> Vec<Range<PointUtf16>> {
        self.find_model().map_or(Vec::new(), |model| {
            model
                .matches
                .iter()
                .filter_map(|id| self.decoration_point_range(*id, cx))
                .collect()
        })
    }

    /// Monaco `FindDecorations.getFindScopes`.
    fn find_scopes(&self, cx: &gpui::App) -> Option<Vec<Range<PointUtf16>>> {
        let model = self.find_model()?;
        let scopes: Vec<_> = model
            .scopes
            .iter()
            .filter_map(|id| self.decoration_point_range(*id, cx))
            .collect();
        (!scopes.is_empty()).then_some(scopes)
    }

    fn full_range(&self) -> Range<PointUtf16> {
        PointUtf16::zero()..self.snapshot.max_point_utf16()
    }

    /// Monaco `FindDecorations.setStartPosition`.
    fn set_find_start_position(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        if let Some(model) = self.find.model.as_mut() {
            model.start_position = position;
        }
        self.set_current_find_match(None, cx);
    }

    /// Monaco's `onDidChangeCursorPosition`: explicit moves, undo and redo
    /// restart incremental search from the cursor.
    pub(crate) fn find_cursor_changed(
        &mut self,
        reason: CursorChangeReason,
        cx: &mut Context<Self>,
    ) {
        if self.find.model.is_some()
            && matches!(
                reason,
                CursorChangeReason::Explicit | CursorChangeReason::Undo | CursorChangeReason::Redo
            )
        {
            let position = self.cursors.primary().position;
            self.set_find_start_position(position, cx);
        }
    }

    /// Monaco's `onDidChangeModelContent`: re-searches shortly after edits
    /// that aren't the find model's own.
    pub(crate) fn find_content_changed(&mut self, flush: bool, cx: &mut Context<Self>) {
        let Some(model) = self.find.model.as_mut() else {
            return;
        };
        let version = self.snapshot.version();
        if let Some((own, research)) = model.own_edit.take()
            && own == version
        {
            if research {
                self.find_research(false, None, cx);
            }
            return;
        }
        if flush {
            // `set_value` dropped the decorations' ranges.
            let mut old = std::mem::take(&mut model.matches);
            old.append(&mut model.overview);
            old.append(&mut model.scopes);
            old.extend(model.range_highlight.take());
            model.highlighted = None;
            let owner = Some(cx.entity_id());
            self.model().update(cx, |text, cx| {
                text.delta_decorations(owner, &old, Vec::new(), cx);
            });
        }
        let position = self.cursors.primary().position;
        self.set_find_start_position(position, cx);
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RESEARCH_AFTER_EDIT).await;
            this.update(cx, |this, cx| this.find_research(false, None, cx))
                .ok();
        });
        if let Some(model) = self.find.model.as_mut() {
            model.research_task = Some(task);
        }
    }

    /// Monaco `research`: finds the matches in the scopes (`new_scope`, or
    /// the current ones), updates the decorations and match info, and with
    /// `move_cursor` moves to the first match from the start position.
    fn find_research(
        &mut self,
        move_cursor: bool,
        new_scope: Option<Option<Vec<Range<PointUtf16>>>>,
        cx: &mut Context<Self>,
    ) {
        if self.find.model.is_none() {
            return;
        }
        let scopes = match new_scope {
            Some(scope) => scope,
            None => self.find_scopes(cx),
        };
        let rope = self.snapshot.rope().clone();
        // Scopes over several lines cover whole lines.
        let scopes = scopes.map(|scopes| {
            scopes
                .into_iter()
                .map(|scope| {
                    if scope.start.row == scope.end.row {
                        return scope;
                    }
                    let mut end_row = scope.end.row;
                    if scope.end.column == 0 {
                        end_row -= 1;
                    }
                    PointUtf16::new(scope.start.row, 0)
                        ..PointUtf16::new(end_row, rope.line_len_utf16(end_row))
                })
                .collect::<Vec<_>>()
        });
        let matches = self.find_matches_in(scopes.as_deref(), MATCHES_LIMIT);
        self.set_find_decorations(&matches, scopes.as_deref(), cx);
        let selection = self.cursors.primary().selection();
        let selection = selection.start()..selection.end();
        let mut position = self.current_matches_position(&selection, cx);
        if position == 0 && !matches.is_empty() {
            // Not on a match: the nearest one after it.
            let after = matches
                .iter()
                .position(|m| compare_using_starts(m, &selection).is_ge())
                .unwrap_or(matches.len());
            if after > 0 {
                position = after;
            }
        }
        self.find.matches_position = position;
        self.find.matches_count = self.find_model().map_or(0, |m| m.matches.len());
        if move_cursor && self.options.find.cursor_move_on_type {
            let start = self
                .find_model()
                .map_or(PointUtf16::zero(), |m| m.start_position);
            self.move_to_next_match_after(start, cx);
        }
        self.alert_find_status(cx);
        cx.notify();
    }

    /// Monaco's find widget `_updateMatchesCount` and `_getAriaLabel`: reads
    /// a changed matches count to screen readers, with the current match's
    /// line and position.
    fn alert_find_status(&mut self, cx: &mut Context<Self>) {
        if !self.find.visible {
            return;
        }
        let query = &self.find.options.query;
        let label = match self.find.status() {
            Some(status) => status,
            None => "No results".to_string(),
        };
        let current = self
            .find_model()
            .and_then(|m| m.highlighted)
            .and_then(|id| self.decoration_point_range(id, cx));
        let message = if self.find.matches_count == 0 {
            if query.is_empty() {
                format!("{label} found")
            } else {
                format!("{label} found for '{query}'")
            }
        } else if let Some(current) = current {
            let line = self.snapshot.rope().line(current.start.row);
            format!(
                "{line}, {label} found for '{query}', at {}:{}",
                current.start.row + 1,
                current.start.column + 1
            )
        } else {
            format!("{label} found for '{query}'")
        };
        if self.find.last_alert.as_ref() != Some(&message) {
            self.find.last_alert = Some(message.clone());
            self.announce(message, cx);
        }
    }

    /// Monaco `_findMatches`: the matches in each scope (or the whole
    /// text), up to `limit`.
    fn find_matches_in(
        &self,
        scopes: Option<&[Range<PointUtf16>]>,
        limit: usize,
    ) -> Vec<Range<PointUtf16>> {
        let Some(data) = self.search_data() else {
            return Vec::new();
        };
        let rope = self.snapshot.rope();
        let ranges: Vec<Range<PointUtf16>> = match scopes {
            Some(scopes) => {
                // Monaco `findMatches`: sorted, overlapping ones merged.
                let mut sorted = scopes.to_vec();
                sorted.sort_by_key(|r| r.start);
                let mut merged: Vec<Range<PointUtf16>> = Vec::new();
                for range in sorted {
                    match merged.last_mut() {
                        Some(last) if range.start < last.end => last.end = last.end.max(range.end),
                        _ => merged.push(range),
                    }
                }
                merged
            }
            None => vec![self.full_range()],
        };
        let mut result = Vec::new();
        for range in ranges {
            if result.len() >= limit {
                break;
            }
            let offsets =
                rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end);
            result.extend(
                search::find_matches(rope, &data, offsets, limit - result.len())
                    .into_iter()
                    .map(|m| {
                        rope.offset_to_point_utf16(m.start)..rope.offset_to_point_utf16(m.end)
                    }),
            );
        }
        result
    }

    /// Monaco `FindDecorations.set`.
    fn set_find_decorations(
        &mut self,
        matches: &[Range<PointUtf16>],
        scopes: Option<&[Range<PointUtf16>]>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut model) = self.find.model.take() else {
            return;
        };
        let colors = self.theme.colors.clone();
        let rope = self.snapshot.rope().clone();
        let offsets = |r: &Range<PointUtf16>| {
            rope.point_utf16_to_offset(r.start)..rope.point_utf16_to_offset(r.end)
        };
        let mut overview = Vec::new();
        let options = if matches.len() > 1000 {
            // Merged overview ruler marks, so painting it stays cheap.
            let line_count = rope.line_count().max(1) as f32;
            let height = self
                .layout
                .as_ref()
                .map_or(1000., |l| f32::from(l.bounds.size.height));
            let per_line = height / line_count;
            let merge = (3. / per_line).ceil().max(2.) as u32;
            let (mut start, mut end) = (matches[0].start.row, matches[0].end.row);
            for range in &matches[1..] {
                if end + merge >= range.start.row {
                    end = end.max(range.end.row);
                } else {
                    overview.push((start, end));
                    (start, end) = (range.start.row, range.end.row);
                }
            }
            overview.push((start, end));
            match_options(&colors, false)
        } else {
            match_options(&colors, true)
        };
        let new_matches = matches
            .iter()
            .map(|m| (offsets(m), options.clone()))
            .collect();
        let new_overview = overview
            .into_iter()
            .map(|(start, end)| {
                (
                    rope.point_to_offset(text::Point::new(start, 0))
                        ..rope.point_to_offset(text::Point::new(end, 0)),
                    DecorationOptions {
                        overview_ruler: Some((
                            colors.find_match_overview,
                            OverviewRulerLane::Center,
                        )),
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let new_scopes = scopes
            .unwrap_or_default()
            .iter()
            .map(|scope| {
                (
                    offsets(scope),
                    DecorationOptions {
                        background: Some(colors.find_range_highlight_background),
                        border: colors.find_range_highlight_border,
                        is_whole_line: true,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let mut old_singles: Vec<DecorationId> = Vec::new();
        old_singles.extend(model.range_highlight.take());
        old_singles.append(&mut model.scopes);
        let owner = Some(cx.entity_id());
        self.model().update(cx, |text, cx| {
            model.matches = text.delta_decorations(owner, &model.matches, new_matches, cx);
            model.overview = text.delta_decorations(owner, &model.overview, new_overview, cx);
            text.delta_decorations(owner, &old_singles, Vec::new(), cx);
            model.scopes = text.delta_decorations(owner, &[], new_scopes, cx);
        });
        model.highlighted = None;
        self.find.model = Some(model);
    }

    /// Monaco `getCurrentMatchesPosition`: the 1-based position of a match
    /// touching `range`, or 0.
    fn current_matches_position(&self, range: &Range<PointUtf16>, cx: &gpui::App) -> usize {
        let Some(model) = self.find_model() else {
            return 0;
        };
        model
            .matches
            .iter()
            .position(|id| {
                self.decoration_point_range(*id, cx)
                    .is_some_and(|m| m.start <= range.end && range.start <= m.end)
            })
            .map_or(0, |ix| ix + 1)
    }

    /// Monaco `FindDecorations.setCurrentFindMatch`: highlights `range` if
    /// it's a match and returns its 1-based position (0 otherwise).
    fn set_current_find_match(
        &mut self,
        range: Option<&Range<PointUtf16>>,
        cx: &mut Context<Self>,
    ) -> usize {
        let Some(model) = self.find.model.as_ref() else {
            return 0;
        };
        let mut new_current = None;
        let mut position = 0;
        if let Some(range) = range {
            for (ix, id) in model.matches.iter().enumerate() {
                if self.decoration_point_range(*id, cx).as_ref() == Some(range) {
                    new_current = Some(*id);
                    position = ix + 1;
                    break;
                }
            }
        }
        if model.highlighted.is_none() && new_current.is_none() {
            return position;
        }
        let colors = self.theme.colors.clone();
        let overview = model.overview.is_empty();
        let old_highlighted = model.highlighted;
        let old_range_highlight = model.range_highlight;
        let rope = self.snapshot.rope().clone();
        let range_highlight = new_current
            .and_then(|id| self.decoration_point_range(id, cx))
            .map(|mut r| {
                // A match ending at a line start highlights up to the line
                // before.
                if r.start.row != r.end.row && r.end.column == 0 {
                    let row = r.end.row - 1;
                    r.end = PointUtf16::new(row, rope.line_len_utf16(row));
                }
                rope.point_utf16_to_offset(r.start)..rope.point_utf16_to_offset(r.end)
            });
        let owner = Some(cx.entity_id());
        let new_range_highlight = self.model().update(cx, |text, cx| {
            if let Some(id) = old_highlighted {
                text.change_decoration_options(id, match_options(&colors, overview), cx);
            }
            if let Some(id) = new_current {
                text.change_decoration_options(id, current_match_options(&colors), cx);
            }
            let old: Vec<_> = old_range_highlight.into_iter().collect();
            let new = range_highlight
                .map(|r| {
                    vec![(
                        r,
                        DecorationOptions {
                            background: Some(colors.range_highlight_background),
                            border: colors.range_highlight_border,
                            is_whole_line: true,
                            stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                            ..Default::default()
                        },
                    )]
                })
                .unwrap_or_default();
            text.delta_decorations(owner, &old, new, cx)
                .first()
                .copied()
        });
        if let Some(model) = self.find.model.as_mut() {
            model.highlighted = new_current;
            model.range_highlight = new_range_highlight;
        }
        position
    }

    /// Monaco `matchBeforePosition`.
    fn match_before_position(
        &self,
        position: PointUtf16,
        cx: &gpui::App,
    ) -> Option<Range<PointUtf16>> {
        let ranges = self.match_ranges(cx);
        for r in ranges.iter().rev() {
            if r.end.row > position.row {
                continue;
            }
            if r.end.row < position.row {
                return Some(r.clone());
            }
            if r.end.column > position.column {
                continue;
            }
            return Some(r.clone());
        }
        ranges.last().cloned()
    }

    /// Monaco `matchAfterPosition`.
    fn match_after_position(
        &self,
        position: PointUtf16,
        cx: &gpui::App,
    ) -> Option<Range<PointUtf16>> {
        let ranges = self.match_ranges(cx);
        for r in &ranges {
            if r.start.row < position.row {
                continue;
            }
            if r.start.row > position.row {
                return Some(r.clone());
            }
            if r.start.column < position.column {
                continue;
            }
            return Some(r.clone());
        }
        ranges.first().cloned()
    }

    /// Monaco `_setCurrentFindMatch`: highlights and selects `range`.
    fn select_find_match(&mut self, range: Range<PointUtf16>, cx: &mut Context<Self>) {
        let position = self.set_current_find_match(Some(&range), cx);
        self.find.matches_position = position;
        self.find.matches_count = self.find_model().map_or(0, |m| m.matches.len());
        self.alert_find_status(cx);
        self.set_selections(
            &[Selection {
                anchor: range.start,
                head: range.end,
            }],
            cx,
        );
        self.reveal_range(
            range.clone(),
            crate::view::reveal::RevealType::CenterIfOutsideViewport,
            cx,
        );
    }

    fn uses_line_stops(&self) -> bool {
        let options = &self.find.options;
        options.is_regex && (options.query.contains('^') || options.query.contains('$'))
    }

    /// Monaco `_prevSearchPosition`.
    fn prev_search_position(&self, before: PointUtf16) -> PointUtf16 {
        let rope = self.snapshot.rope();
        let PointUtf16 {
            mut row,
            mut column,
        } = before;
        if self.uses_line_stops() || column == 0 {
            row = if row == 0 {
                rope.max_point().row
            } else {
                row - 1
            };
            column = rope.line_len_utf16(row);
        } else {
            column -= 1;
        }
        PointUtf16::new(row, column)
    }

    /// Monaco `_nextSearchPosition`.
    fn next_search_position(&self, after: PointUtf16) -> PointUtf16 {
        let rope = self.snapshot.rope();
        let PointUtf16 {
            mut row,
            mut column,
        } = after;
        if self.uses_line_stops() || column == rope.line_len_utf16(row) {
            row = if row == rope.max_point().row {
                0
            } else {
                row + 1
            };
            column = 0;
        } else {
            column += 1;
        }
        PointUtf16::new(row, column)
    }

    fn cannot_find(&mut self, cx: &mut Context<Self>) -> bool {
        if self.find.matches_count > 0 {
            return false;
        }
        // Remind that find in selection is on.
        if let Some(scope) = self.find_scopes(cx).and_then(|s| s.into_iter().next()) {
            self.reveal_range(
                scope,
                crate::view::reveal::RevealType::CenterIfOutsideViewport,
                cx,
            );
        }
        true
    }

    fn search_range(&self, cx: &gpui::App) -> Range<PointUtf16> {
        self.find_scopes(cx)
            .and_then(|s| s.into_iter().next())
            .unwrap_or_else(|| self.full_range())
    }

    fn find_next_match_from(&self, position: PointUtf16) -> Option<Range<PointUtf16>> {
        let data = self.search_data()?;
        let rope = self.snapshot.rope();
        let found = search::find_next(rope, &data, rope.point_utf16_to_offset(position))?;
        Some(rope.offset_to_point_utf16(found.start)..rope.offset_to_point_utf16(found.end))
    }

    fn find_previous_match_from(&self, position: PointUtf16) -> Option<Range<PointUtf16>> {
        let data = self.search_data()?;
        let rope = self.snapshot.rope();
        let found = search::find_previous(rope, &data, rope.point_utf16_to_offset(position))?;
        Some(rope.offset_to_point_utf16(found.start)..rope.offset_to_point_utf16(found.end))
    }

    /// Monaco `_moveToPrevMatch`.
    fn move_to_prev_match_before(
        &mut self,
        mut before: PointUtf16,
        is_recursed: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.find.can_navigate_back(self.options.find.loop_) {
            // Past the first match: stay on it.
            if let Some(next) = self.match_after_position(before, cx) {
                self.select_find_match(next, cx);
            }
            return;
        }
        let count = self.find_model().map_or(0, |m| m.matches.len());
        if count < MATCHES_LIMIT {
            let mut prev = self.match_before_position(before, cx);
            if prev
                .as_ref()
                .is_some_and(|p| p.is_empty() && p.start == before)
            {
                before = self.prev_search_position(before);
                prev = self.match_before_position(before, cx);
            }
            if let Some(prev) = prev {
                self.select_find_match(prev, cx);
            }
            return;
        }
        if self.cannot_find(cx) {
            return;
        }
        let search_range = self.search_range(cx);
        if search_range.end < before || before < search_range.start {
            before = search_range.end;
        }
        let mut position = before;
        let mut prev = self.find_previous_match_from(position);
        if prev
            .as_ref()
            .is_some_and(|p| p.is_empty() && p.start == position)
        {
            position = self.prev_search_position(position);
            prev = self.find_previous_match_from(position);
        }
        let Some(prev) = prev else {
            return;
        };
        if !is_recursed && !contains_range(&search_range, &prev) {
            return self.move_to_prev_match_before(prev.start, true, cx);
        }
        self.select_find_match(prev, cx);
    }

    /// Monaco `_moveToNextMatch`.
    fn move_to_next_match_after(&mut self, mut after: PointUtf16, cx: &mut Context<Self>) {
        if !self.find.can_navigate_forward(self.options.find.loop_) {
            // Past the last match: stay on it.
            if let Some(prev) = self.match_before_position(after, cx) {
                self.select_find_match(prev, cx);
            }
            return;
        }
        let count = self.find_model().map_or(0, |m| m.matches.len());
        if count < MATCHES_LIMIT {
            let mut next = self.match_after_position(after, cx);
            if next
                .as_ref()
                .is_some_and(|n| n.is_empty() && n.start == after)
            {
                after = self.next_search_position(after);
                next = self.match_after_position(after, cx);
            }
            if let Some(next) = next {
                self.select_find_match(next, cx);
            }
            return;
        }
        if let Some(next) = self.get_next_match(after, true, false, cx) {
            self.select_find_match(next, cx);
        }
    }

    /// Monaco `_getNextMatch`.
    fn get_next_match(
        &mut self,
        mut after: PointUtf16,
        force_move: bool,
        is_recursed: bool,
        cx: &mut Context<Self>,
    ) -> Option<Range<PointUtf16>> {
        if self.cannot_find(cx) {
            return None;
        }
        let search_range = self.search_range(cx);
        if search_range.end < after || after < search_range.start {
            after = search_range.start;
        }
        let mut position = after;
        let mut next = self.find_next_match_from(position);
        if force_move
            && next
                .as_ref()
                .is_some_and(|n| n.is_empty() && n.start == position)
        {
            position = self.next_search_position(position);
            next = self.find_next_match_from(position);
        }
        let next = next?;
        if !is_recursed && !contains_range(&search_range, &next) {
            return self.get_next_match(next.end, force_move, true, cx);
        }
        Some(next)
    }

    /// Selects the next match after the selection (Monaco `moveToNextMatch`,
    /// `editor.action.nextMatchFindAction`).
    pub fn find_next(&mut self, cx: &mut Context<Self>) {
        self.start_find_model(cx);
        let after = self.cursors.primary().selection().end();
        self.move_to_next_match_after(after, cx);
    }

    /// Selects the match before the selection (Monaco `moveToPrevMatch`,
    /// `editor.action.previousMatchFindAction`).
    pub fn find_previous(&mut self, cx: &mut Context<Self>) {
        self.start_find_model(cx);
        let before = self.cursors.primary().selection().start();
        self.move_to_prev_match_before(before, false, cx);
    }

    /// Selects the `index`th match (Monaco `moveToMatch`).
    pub fn find_go_to_match(&mut self, index: usize, cx: &mut Context<Self>) {
        let range = self
            .find_model()
            .and_then(|m| m.matches.get(index).copied())
            .and_then(|id| self.decoration_point_range(id, cx));
        if let Some(range) = range {
            self.select_find_match(range, cx);
        }
    }

    /// Monaco `_getReplacePattern`.
    fn replace_pattern(&self) -> ReplacePattern {
        if self.find.options.is_regex {
            parse_replace_string(&self.find.options.replace)
        } else {
            ReplacePattern::from_static_value(&self.find.options.replace)
        }
    }

    /// The regex captures of `range` (Monaco's `captureMatches`), with the
    /// whole match first.
    fn match_captures(&self, data: &SearchData, range: &Range<usize>) -> Vec<Option<String>> {
        let rope = self.snapshot.rope();
        let (haystack, start) = search::match_context(rope, data, range);
        search::captures(data, &haystack, range.start - start..range.end - start)
            .unwrap_or_else(|| vec![Some(rope.text_for_range(range.clone()))])
    }

    fn build_replacement(
        pattern: &ReplacePattern,
        captures: Option<&[Option<String>]>,
        preserve_case: bool,
    ) -> String {
        let borrowed: Option<Vec<Option<&str>>> =
            captures.map(|c| c.iter().map(|m| m.as_deref()).collect());
        pattern.build_replace_string(borrowed.as_deref(), preserve_case)
    }

    /// Replaces the selected match and moves to the next one; if the
    /// selection isn't a match, selects the next (Monaco `replace`).
    pub fn replace_one(&mut self, cx: &mut Context<Self>) {
        self.start_find_model(cx);
        if self.find.matches_count == 0 || self.options.read_only {
            return;
        }
        let Some(data) = self.search_data() else {
            return;
        };
        let pattern = self.replace_pattern();
        let selection = self.cursors.primary().selection();
        let selection = selection.start()..selection.end();
        let Some(next) = self.get_next_match(selection.start, false, false, cx) else {
            return;
        };
        if next != selection {
            let position = self.cursors.primary().position;
            self.set_find_start_position(position, cx);
            self.select_find_match(next, cx);
            return;
        }
        let rope = self.snapshot.rope();
        let offsets =
            rope.point_utf16_to_offset(selection.start)..rope.point_utf16_to_offset(selection.end);
        let captures = self.match_captures(&data, &offsets);
        let replacement =
            Self::build_replacement(&pattern, Some(&captures), self.find.options.preserve_case);
        // Monaco `ReplaceCommand`: the cursor ends after the replacement.
        let mut end = selection.start;
        for ch in replacement.chars() {
            if ch == '\n' {
                end = PointUtf16::new(end.row + 1, 0);
            } else {
                end.column += ch.len_utf16() as u32;
            }
        }
        let length = replacement.encode_utf16().count() as u32;
        self.with_find_edit(false, cx, |this, cx| {
            this.apply(
                cursor::EditOperation {
                    edits: vec![(selection.clone(), replacement)],
                    selections_after: vec![Selection::collapsed(end)],
                    kind: cursor::EditOperationType::Other,
                    push_stack_element_before: true,
                    push_stack_element_after: true,
                    auto_closed: Vec::new(),
                    follow_up: None,
                    auto_whitespace: Vec::new(),
                },
                cx,
            );
        });
        // Monaco moves the start along the line by the replacement's length.
        self.set_find_start_position(
            PointUtf16::new(selection.start.row, selection.start.column + length),
            cx,
        );
        self.find_research(true, None, cx);
    }

    /// Runs the model's own edit without the content-change handling
    /// (Monaco `_executeEditorCommand`).
    /// `research` re-searches once the editor sees the edit.
    fn with_find_edit(
        &mut self,
        research: bool,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Context<Self>),
    ) {
        f(self, cx);
        let version = self.model().read(cx).buffer().version();
        if let Some(model) = self.find.model.as_mut() {
            model.own_edit = Some((version, research));
        }
    }

    /// Replaces every match in one undo step, keeping the selection
    /// (Monaco `replaceAll`).
    pub fn replace_all(&mut self, cx: &mut Context<Self>) {
        self.start_find_model(cx);
        if self.find.matches_count == 0 || self.options.read_only {
            return;
        }
        let Some(data) = self.search_data() else {
            return;
        };
        let pattern = self.replace_pattern();
        let preserve_case = self.find.options.preserve_case;
        let scopes = self.find_scopes(cx);
        let matches = self.find_matches_in(scopes.as_deref(), usize::MAX);
        let rope = self.snapshot.rope().clone();
        let needs_captures = pattern.has_replacement_patterns() || preserve_case;
        let edits: Vec<(Range<usize>, String)> = matches
            .iter()
            .map(|m| {
                let offsets =
                    rope.point_utf16_to_offset(m.start)..rope.point_utf16_to_offset(m.end);
                let captures = needs_captures.then(|| self.match_captures(&data, &offsets));
                let replacement =
                    Self::build_replacement(&pattern, captures.as_deref(), preserve_case);
                (offsets, replacement)
            })
            .collect();
        // Monaco `ReplaceAllCommand`: the selection is tracked through the
        // edits.
        let source = cx.entity_id();
        let before = crate::model::metadata(self.cursors.selections());
        // Monaco then re-searches the new text.
        self.with_find_edit(true, cx, |this, cx| {
            this.model().update(cx, |model, cx| {
                model.push_stack_element();
                if model.edit_from(edits, None, Some(source), cx).is_ok() {
                    model.set_undo_metadata(Some(before), None);
                }
                model.push_stack_element();
            });
        });
    }

    /// Selects every match, keeping the selection primary if it's one
    /// (Monaco `selectAllMatches`, Alt+Enter).
    pub fn select_all_matches(&mut self, cx: &mut Context<Self>) {
        self.start_find_model(cx);
        if self.find.matches_count == 0 {
            return;
        }
        let scopes = self.find_scopes(cx);
        let mut selections: Vec<Selection> = self
            .find_matches_in(scopes.as_deref(), usize::MAX)
            .into_iter()
            .map(|m| Selection {
                anchor: m.start,
                head: m.end,
            })
            .collect();
        let primary = self.cursors.primary().selection();
        if let Some(ix) = selections
            .iter()
            .position(|s| s.start() == primary.start() && s.end() == primary.end())
        {
            selections.remove(ix);
            selections.insert(0, primary);
        }
        self.set_selections(&selections, cx);
    }

    /// Removes the find model from the current text before the editor
    /// switches models. The query and widget stay.
    pub(crate) fn detach_find_from_model(&mut self, cx: &mut Context<Self>) {
        self.stop_find_model(cx);
        self.find.search_scope = None;
        self.find.options.in_selection = false;
        // Monaco reloads the stored toggles for the new model.
        FindToggles::load(&mut self.find.options, cx);
    }

    /// Re-binds the find model to a new model while the widget shows.
    pub(crate) fn reattach_find_model(&mut self, cx: &mut Context<Self>) {
        if self.find.visible {
            self.start_find_model(cx);
        }
    }

    /// Monaco `getSelectionSearchString`: the selection's text (`multiple`
    /// allows several lines), or the word at an empty cursor unless
    /// `non_empty_only`.
    fn selection_search_string(
        &self,
        multiple: bool,
        non_empty_only: bool,
        cx: &gpui::App,
    ) -> Option<String> {
        let selection = self.cursors.primary().selection();
        if !multiple && selection.start().row != selection.end().row {
            return None;
        }
        let rope = self.snapshot.rope();
        if selection.is_empty() {
            if non_empty_only {
                return None;
            }
            let config = self.cursor_config(cx);
            cursor::word_at(&config, rope, selection.head)
                .map(|word| {
                    rope.text_for_range(
                        rope.point_utf16_to_offset(word.start)
                            ..rope.point_utf16_to_offset(word.end),
                    )
                })
                .filter(|word| word.chars().any(|c| c.is_alphanumeric() || c == '_'))
        } else {
            let range = rope.point_utf16_to_offset(selection.start())
                ..rope.point_utf16_to_offset(selection.end());
            // Monaco `SEARCH_STRING_MAX_LENGTH`.
            (range.len() < 524_288).then(|| rope.text_for_range(range))
        }
    }

    /// Monaco `FindController._start`: shows the widget, seeds the search
    /// string, maybe scopes to the selections, and binds a fresh find model.
    fn find_start(&mut self, start: FindStart, window: &mut Window, cx: &mut Context<Self>) {
        self.find_start_with(start, |_| {}, window, cx);
    }

    /// [`Self::find_start`] with Monaco's `newState`: changes to the find
    /// state made before the search string is seeded.
    fn find_start_with(
        &mut self,
        start: FindStart,
        new_state: impl FnOnce(&mut FindOptions),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::options::AutoFindInSelection;
        // The widget's controller: `find.autoFindInSelection`.
        let selection = self.cursors.primary().selection();
        let update_scope = start.update_search_scope
            || match self.options.find.auto_find_in_selection {
                AutoFindInSelection::Never => false,
                AutoFindInSelection::Always => true,
                AutoFindInSelection::Multiline => selection.start().row != selection.end().row,
            };
        self.stop_find_model(cx);
        let mut options = self.find.options.clone();
        // Whether `new_state` brings a search string (Monaco's
        // `stateChanges.searchString`).
        let previous = std::mem::take(&mut options.query);
        new_state(&mut options);
        let mut seeded = !options.query.is_empty();
        if !seeded {
            options.query = previous;
        }
        match start.seed {
            SeedMode::Single => {
                if let Some(seed) =
                    self.selection_search_string(false, start.seed_non_empty_only, cx)
                {
                    // Monaco escapes for the regex toggle as it was.
                    options.query = if self.find.options.is_regex {
                        regex::escape(&seed)
                    } else {
                        seed
                    };
                    seeded = true;
                }
            }
            SeedMode::Multiple if !update_scope => {
                if let Some(seed) = self.selection_search_string(true, false, cx) {
                    options.query = seed;
                    seeded = true;
                }
            }
            _ => {}
        }
        if !seeded && start.seed_from_global_clipboard {
            let term = self.global_find_term(cx);
            if !term.is_empty() {
                options.query = term;
            }
        }
        let was_visible = self.find.visible;
        if start.force_reveal_replace {
            self.find.replace_visible = true;
        } else if !was_visible {
            self.find.replace_visible = false;
        }
        let mut scope = None;
        if update_scope && self.cursors.selections().iter().any(|s| !s.is_empty()) {
            // Monaco stores the selections as they are.
            scope = Some(
                self.cursors
                    .selections()
                    .iter()
                    .map(|s| s.start()..s.end())
                    .collect::<Vec<_>>(),
            );
        }
        self.find.visible = true;
        if let Some(scope) = scope {
            options.in_selection = true;
            self.find.search_scope = Some(scope);
            // Already set; don't recapture it from the selections.
            self.find.options.in_selection = true;
        }
        self.change_find_options(options.clone(), false, cx);
        // Monaco's model starts incremental search at the cursor.
        self.start_find_model(cx);
        let replace = self.find.replace_visible;
        let widget = match &self.find.widget {
            Some((widget, _)) => widget.clone(),
            None => {
                let editor = cx.entity().downgrade();
                let theme = self.theme.clone();
                let widget = cx.new(|cx| FindWidget::new(editor, theme, window, cx));
                self.find.inputs = Some(widget.read(cx).inputs());
                let id = self.add_overlay_widget(
                    OverlayWidget::new(OverlayPosition::TopRight, widget.clone().into()),
                    cx,
                );
                self.find.widget = Some((widget.clone(), id));
                widget
            }
        };
        widget.update(cx, |widget, cx| {
            widget.sync_from_editor(&options, replace, window, cx);
            match start.focus {
                StartFocus::FindInput => widget.focus_find(window, cx),
                StartFocus::ReplaceInput => widget.focus_replace(window, cx),
                StartFocus::NoChange => {}
            }
        });
        // After the inputs show the search string, which sizes them.
        self.update_find_view_zone(cx);
        self.alert_find_status(cx);
        cx.notify();
    }

    /// Whether the find input has focus (Monaco `findInputFocussed`).
    fn is_find_input_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.find
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).is_find_input_focused(window, cx))
    }

    fn seed_on_start(&self) -> (SeedMode, bool) {
        use crate::options::SeedSearchString;
        let option = self.options.find.seed_search_string_from_selection;
        let seed = if option == SeedSearchString::Never {
            SeedMode::None
        } else {
            SeedMode::Single
        };
        (seed, option == SeedSearchString::Selection)
    }

    /// Opens the find widget (Monaco `actions.find`, Cmd/Ctrl+F; with
    /// `replace`, `editor.action.startFindReplaceAction`).
    pub fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (seed, seed_non_empty_only) = self.seed_on_start();
        if !replace {
            self.find_start(
                FindStart {
                    seed,
                    seed_non_empty_only,
                    seed_from_global_clipboard: self.options.find.global_find_clipboard,
                    focus: StartFocus::FindInput,
                    ..Default::default()
                },
                window,
                cx,
            );
            return;
        }
        // Monaco `StartFindReplaceAction`: seeds only from a one-line
        // selection while the find input isn't focused; focuses replace
        // when there's something to replace.
        if self.options.read_only {
            return;
        }
        let selection = self.cursors.primary().selection();
        let find_focused = self.is_find_input_focused(window, cx);
        let seed_from_selection = !selection.is_empty()
            && selection.start().row == selection.end().row
            && seed != SeedMode::None
            && !find_focused;
        self.find_start(
            FindStart {
                force_reveal_replace: true,
                seed: if seed_from_selection {
                    SeedMode::Single
                } else {
                    SeedMode::None
                },
                seed_non_empty_only,
                seed_from_global_clipboard: self.options.find.seed_search_string_from_selection
                    != crate::options::SeedSearchString::Never,
                focus: if find_focused || seed_from_selection {
                    StartFocus::ReplaceInput
                } else {
                    StartFocus::FindInput
                },
                update_search_scope: false,
            },
            window,
            cx,
        );
    }

    /// Opens find with `args` (Monaco `editor.actions.findWithArgs`): unset
    /// fields keep their values, a replace string shows the replace input,
    /// and `find_in_selection` searches in the selections. As in Monaco, an
    /// empty search string is first seeded from the selection or the word
    /// at the cursor, which wins over `args.search_string`.
    pub fn find_with_args(
        &mut self,
        args: &crate::actions::FindWithArgs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::options::SeedSearchString;
        let option = self.options.find.seed_search_string_from_selection;
        let seed = if self.find.options.query.is_empty() && option != SeedSearchString::Never {
            SeedMode::Single
        } else {
            SeedMode::None
        };
        let args = args.clone();
        self.find_start_with(
            FindStart {
                force_reveal_replace: args.replace_string.is_some(),
                seed,
                seed_non_empty_only: option == SeedSearchString::Selection,
                seed_from_global_clipboard: true,
                focus: StartFocus::FindInput,
                update_search_scope: args.find_in_selection,
            },
            move |options| {
                if let Some(query) = args.search_string {
                    options.query = query;
                }
                if let Some(replace) = args.replace_string {
                    options.replace = replace;
                }
                if let Some(is_regex) = args.is_regex {
                    options.is_regex = is_regex;
                }
                if let Some(whole_word) = args.match_whole_word {
                    options.whole_word = whole_word;
                }
                if let Some(match_case) = args.is_case_sensitive {
                    options.match_case = match_case;
                }
                if let Some(preserve_case) = args.preserve_case {
                    options.preserve_case = preserve_case;
                }
            },
            window,
            cx,
        );
        let query = self.find.options.query.clone();
        self.set_global_find_term(&query, cx);
    }

    /// The number of matches, as the find widget counts them.
    pub fn find_matches_count(&self) -> usize {
        self.find.matches_count
    }

    /// Asks for a match number in the quick input and selects that match
    /// (Monaco `editor.action.goToMatchFindAction`, "Go to Match..."): 1 is
    /// the first, -1 the last. Needs the find widget open.
    pub fn go_to_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find.visible {
            return;
        }
        if self.find.matches_count < 1 {
            // Monaco shows this as a warning notification.
            let position = self.cursors.primary().position;
            self.show_message(
                "No matches. Try searching for something else.",
                position,
                cx,
            );
            return;
        }
        self.open_go_to_match(window, cx);
    }

    /// Monaco's `toFindMatchIndex` for the "Go to Match..." input: a
    /// 0-based match index for `value` (1-based, or negative from the end).
    pub(crate) fn find_match_index(&self, value: &str) -> Option<usize> {
        let count = self.find.matches_count as i64;
        let index = parse_int(value)?;
        if index > 0 && index <= count {
            Some((index - 1) as usize)
        } else if index < 0 && index >= -count {
            Some((count + index) as usize)
        } else {
            None
        }
    }

    /// Opens find with the selection, even over several lines, as the
    /// search string (Monaco `actions.findWithSelection`, Cmd+E on macOS).
    pub fn start_find_with_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_start(
            FindStart {
                seed: SeedMode::Multiple,
                ..Default::default()
            },
            window,
            cx,
        );
        let query = self.find.options.query.clone();
        self.set_global_find_term(&query, cx);
    }

    /// Monaco `matchFindAction` (Find Next/Previous, F3, Cmd/Ctrl+G): moves
    /// to the next match, first opening find (seeded from the cursor when
    /// there's no search string) if that doesn't land on one. With
    /// `find.closeOnResult`, landing on a match from the find input closes
    /// the widget.
    pub fn find_match_action(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        let was_visible = self.find.visible;
        let run = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| -> bool {
            if this.find.model.is_none() {
                return false;
            }
            let before = this.cursors.primary().selection();
            if next {
                let after = before.end();
                this.move_to_next_match_after(after, cx);
            } else {
                let before = before.start();
                this.move_to_prev_match_before(before, false, cx);
            }
            let after = this.cursors.primary().selection();
            if after == before {
                return false;
            }
            this.push_undo_stop(cx);
            if this.options.find.close_on_result
                && was_visible
                && this.is_find_input_focused(window, cx)
            {
                this.close_find(window, cx);
            }
            true
        };
        if run(self, window, cx) {
            return;
        }
        let (seed, seed_non_empty_only) = self.seed_on_start();
        let seed = if self.find.options.query.is_empty() {
            seed
        } else {
            SeedMode::None
        };
        self.find_start(
            FindStart {
                seed,
                seed_non_empty_only,
                seed_from_global_clipboard: true,
                ..Default::default()
            },
            window,
            cx,
        );
        run(self, window, cx);
    }

    /// Monaco `SelectionMatchFindAction` (Find Next/Previous Selection,
    /// Cmd/Ctrl+F3 and with Shift): searches for the selection or the word
    /// at the cursor.
    pub fn find_selection_match_action(
        &mut self,
        next: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(search) = self.selection_search_string(false, false, cx) {
            // Monaco `setSearchString`, escaped in regex mode.
            let mut options = self.find.options.clone();
            options.query = if options.is_regex {
                regex::escape(&search)
            } else {
                search
            };
            self.change_find_options(options, false, cx);
        }
        if self.find.model.is_none() {
            self.find_start(FindStart::default(), window, cx);
        }
        if next {
            let after = self.cursors.primary().selection().end();
            self.move_to_next_match_after(after, cx);
        } else {
            let before = self.cursors.primary().selection().start();
            self.move_to_prev_match_before(before, false, cx);
        }
    }

    /// The find scope (Monaco `searchScope`), as captured from the
    /// selections.
    pub fn find_search_scope(&self) -> Option<&[Range<PointUtf16>]> {
        self.find.search_scope.as_deref()
    }

    /// Whether the query is a regex that doesn't parse (the find input shows
    /// an error).
    pub fn is_find_query_invalid(&self) -> bool {
        !self.find.options.query.is_empty() && self.search_data().is_none()
    }

    /// The find widget's height (Monaco `_getHeight`): its inputs' heights,
    /// from their own text and line height, plus the margins. The widget
    /// lays itself out at this height and reports what it measured (see
    /// [`Self::find_widget_measured`]).
    fn find_widget_height(&self, cx: &gpui::App) -> gpui::Pixels {
        match &self.find.inputs {
            Some((find, replace)) => crate::find_widget::widget_height(
                find.read(cx),
                self.find.replace_visible.then(|| replace.read(cx)),
                cx,
            ),
            None => gpui::px(crate::find_widget::FIND_INPUT_AREA_HEIGHT),
        }
    }

    /// The find widget's height as painted, measured while it was laid out
    /// for `expected` (its [`Self::find_widget_height`] then): the view zone
    /// follows the measurement (Monaco `_tryUpdateHeight` on the inputs'
    /// `onDidHeightChange`).
    pub(crate) fn find_widget_measured(
        &mut self,
        expected: gpui::Pixels,
        measured: gpui::Pixels,
        cx: &mut Context<Self>,
    ) {
        if self.find.visible && self.find.measured != Some((expected, measured)) {
            self.find.measured = Some((expected, measured));
            self.update_find_view_zone(cx);
        }
    }

    /// Monaco `_showViewZone`/`_removeViewZone`: while the widget shows,
    /// space above the first line keeps it from covering text when
    /// scrolled to the top; the scroll position moves with it so the text
    /// doesn't jump. The editor's top padding counts towards the space.
    /// The model changed and its view zones went with the old view (Monaco
    /// `FindWidget` drops `_viewZoneId` on `onDidChangeModel`).
    pub(crate) fn forget_find_view_zone(&mut self) {
        self.find.zone = None;
    }

    pub(crate) fn update_find_view_zone(&mut self, cx: &mut Context<Self>) {
        let wanted = (self.find.visible && self.options.find.add_extra_space_on_top)
            .then(|| {
                let expected = self.find_widget_height(cx);
                let height = match self.find.measured {
                    // Measured with the inputs as they are now.
                    Some((laid_out, measured)) if laid_out == expected => measured,
                    _ => expected,
                };
                height - self.options.padding.top
            })
            .filter(|height| *height > gpui::px(0.));
        let current = self.find.zone.map(|(_, height)| height);
        if wanted == current {
            return;
        }
        if let Some((id, height)) = self.find.zone.take() {
            self.remove_widget(id, cx);
            self.scroll_top = (self.scroll_top - height).max(gpui::px(0.));
        }
        if let Some(height) = wanted {
            let view = cx.new(|_| FindZoneSpacer);
            let id = self.add_view_zone(
                crate::widgets::ViewZone::new(
                    0,
                    crate::widgets::ZoneHeight::Pixels(height),
                    view.into(),
                ),
                cx,
            );
            self.find.zone = Some((id, height));
            self.scroll_top += height;
        }
        cx.notify();
    }

    /// The height of the space kept above the text for the find widget
    /// (`find.add_extra_space_on_top`), for tests.
    #[doc(hidden)]
    pub fn find_zone_height_for_tests(&self) -> Option<gpui::Pixels> {
        self.find.zone.map(|(_, height)| height)
    }

    /// Shows or hides the replace input (Monaco `isReplaceRevealed`).
    pub fn set_find_replace_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.find.replace_visible = visible;
        self.update_find_view_zone(cx);
    }

    /// Closes the find widget and returns focus to the editor.
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find.visible {
            return;
        }
        self.find.visible = false;
        if let Some((_, id)) = self.find.widget.take() {
            self.remove_widget(id, cx);
        }
        self.find.inputs = None;
        self.find.measured = None;
        self.find.last_alert = None;
        // Monaco disposes the model and clears the scope when hidden.
        self.stop_find_model(cx);
        self.update_find_view_zone(cx);
        if self.find.options.in_selection {
            self.find.options.in_selection = false;
            self.find.search_scope = None;
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Sets the selections as an explicit gesture would (Monaco's tests'
    /// `MoveTo` from the mouse), for tests.
    #[doc(hidden)]
    pub fn set_selections_explicit_for_tests(
        &mut self,
        selections: &[Selection],
        cx: &mut Context<Self>,
    ) {
        self.cursors.set_selections(selections);
        self.cursors.validate(self.snapshot.rope());
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
    }

    /// Binds the find model without showing the widget (as Monaco's tests
    /// construct `FindModelBoundToEditorModel`).
    #[doc(hidden)]
    pub fn start_find_model_for_tests(&mut self, cx: &mut Context<Self>) {
        self.start_find_model(cx);
    }

    /// (highlighted match, every match, matches position, matches count),
    /// as Monaco's find tests read them.
    #[doc(hidden)]
    pub fn find_state_for_tests(
        &self,
        cx: &gpui::App,
    ) -> (
        Option<Range<PointUtf16>>,
        Vec<Range<PointUtf16>>,
        usize,
        usize,
    ) {
        let highlighted = self
            .find_model()
            .and_then(|m| m.highlighted)
            .and_then(|id| self.decoration_point_range(id, cx));
        let mut all = self.match_ranges(cx);
        all.sort_by(compare_using_starts);
        (
            highlighted,
            all,
            self.find.matches_position,
            self.find.matches_count,
        )
    }
}

/// JavaScript's `parseInt(value)` for base 10: an optional sign and the
/// digits that follow, after leading whitespace; `None` without digits.
fn parse_int(value: &str) -> Option<i64> {
    let value = value.trim_start();
    let (sign, rest) = match value.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, value.strip_prefix('+').unwrap_or(value)),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<i64>().ok().map(|n| sign * n)
}

#[cfg(test)]
mod parse_int_tests {
    use super::parse_int;

    #[test]
    fn parses_like_javascript() {
        assert_eq!(parse_int("12"), Some(12));
        assert_eq!(parse_int(" -3x"), Some(-3));
        assert_eq!(parse_int("+4"), Some(4));
        assert_eq!(parse_int("4.9"), Some(4));
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int("-"), None);
    }
}
