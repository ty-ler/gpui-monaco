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
}

impl FindState {
    pub(crate) fn new() -> Self {
        Self::default()
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
    fn can_navigate_back(&self, loop_: bool) -> bool {
        self.can_navigate_in_loop(loop_) || self.matches_position != 1
    }

    /// Monaco `canNavigateForward`.
    fn can_navigate_forward(&self, loop_: bool) -> bool {
        self.can_navigate_in_loop(loop_) || self.matches_position < self.matches_count
    }

    fn can_navigate_in_loop(&self, loop_: bool) -> bool {
        loop_ || self.matches_count >= MATCHES_LIMIT
    }
}

fn match_options(colors: &crate::theme::EditorColors, overview: bool) -> DecorationOptions {
    DecorationOptions {
        background: Some(colors.find_match_highlight_background),
        overview_ruler: overview.then_some((colors.find_match_overview, OverviewRulerLane::Center)),
        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
        z_index: 10,
        ..Default::default()
    }
}

fn current_match_options(colors: &crate::theme::EditorColors) -> DecorationOptions {
    DecorationOptions {
        background: Some(colors.find_match_background),
        overview_ruler: Some((colors.find_match_overview, OverviewRulerLane::Center)),
        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
        z_index: 13,
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
        // Monaco's controller: turning on find in selection captures the
        // selections (whole lines when they span lines).
        let mut scope_changed = false;
        if options.in_selection != self.find.options.in_selection {
            let scope = options.in_selection.then(|| self.selection_scopes(cx));
            self.find.search_scope = scope.filter(|s| !s.is_empty());
            scope_changed = true;
        }
        let old = std::mem::replace(&mut self.find.options, options);
        let new = &self.find.options;
        let research = new.query != old.query
            || new.is_regex != old.is_regex
            || new.match_case != old.match_case
            || new.whole_word != old.whole_word;
        if research || scope_changed {
            let scope = scope_changed.then(|| self.find.search_scope.clone());
            self.find_research(move_cursor, scope, cx);
        }
        cx.notify();
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
        cx.notify();
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
    }

    /// Re-binds the find model to a new model while the widget shows.
    pub(crate) fn reattach_find_model(&mut self, cx: &mut Context<Self>) {
        if self.find.visible {
            self.start_find_model(cx);
        }
    }

    /// Opens the find widget (Monaco `FindController._start` for
    /// `actions.find`): the query comes from the selection or the word at
    /// the cursor (`find.seedSearchStringFromSelection`), and selections may
    /// turn on find in selection (`find.autoFindInSelection`).
    pub fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        use crate::options::{AutoFindInSelection, SeedSearchString};
        let selection = self.cursors.primary().selection();
        let rope = self.snapshot.rope().clone();
        let find_options = self.options.find.clone();
        // Monaco `getSelectionSearchString` ('single').
        let seed = match find_options.seed_search_string_from_selection {
            SeedSearchString::Never => None,
            seed if selection.start().row == selection.end().row => {
                if selection.is_empty() {
                    let config = self.cursor_config(cx);
                    (seed == SeedSearchString::Always)
                        .then(|| cursor::word_at(&config, &rope, selection.head))
                        .flatten()
                        .map(|word| {
                            rope.text_for_range(
                                rope.point_utf16_to_offset(word.start)
                                    ..rope.point_utf16_to_offset(word.end),
                            )
                        })
                        .filter(|word| word.chars().any(|c| c.is_alphanumeric() || c == '_'))
                } else {
                    Some(rope.text_for_range(
                        rope.point_utf16_to_offset(selection.start())
                            ..rope.point_utf16_to_offset(selection.end()),
                    ))
                }
            }
            _ => None,
        };
        let multiline = selection.start().row != selection.end().row;
        let update_scope = match find_options.auto_find_in_selection {
            AutoFindInSelection::Never => false,
            AutoFindInSelection::Always => true,
            AutoFindInSelection::Multiline => multiline,
        };
        self.find.visible = true;
        self.find.replace_visible = replace;
        // Monaco's model starts incremental search at the cursor.
        self.start_find_model(cx);
        let mut options = self.find.options.clone();
        if let Some(seed) = seed {
            // Monaco escapes the selection in regex mode.
            options.query = if options.is_regex {
                regex::escape(&seed)
            } else {
                seed
            };
        }
        if update_scope && self.cursors.selections().iter().any(|s| !s.is_empty()) {
            options.in_selection = true;
            // A fresh scope from the current selections.
            self.find.options.in_selection = false;
        }
        let widget = match &self.find.widget {
            Some((widget, _)) => widget.clone(),
            None => {
                let editor = cx.entity().downgrade();
                let theme = self.theme.clone();
                let widget = cx.new(|cx| FindWidget::new(editor, theme, window, cx));
                let id = self.add_overlay_widget(
                    OverlayWidget {
                        position: OverlayPosition::TopRight,
                        view: widget.clone().into(),
                    },
                    cx,
                );
                self.find.widget = Some((widget.clone(), id));
                widget
            }
        };
        self.change_find_options(options.clone(), false, cx);
        widget.update(cx, |widget, cx| {
            widget.sync_from_editor(&options, replace, window, cx);
            widget.focus_find(window, cx);
        });
        cx.notify();
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
        // Monaco disposes the model and clears the scope when hidden.
        self.stop_find_model(cx);
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
