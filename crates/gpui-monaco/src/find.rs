//! Find and replace (Monaco's `contrib/find` model and controller).

use std::ops::Range;

use gpui::{AppContext as _, Context, Entity, Task, Window};
use text::{Anchor, Bias, PointUtf16, Version};

use crate::cursor::{self, Selection};
use crate::decorations::{DecorationId, DecorationOptions, OverviewRulerLane};
use crate::editor::CodeEditor;
use crate::find_widget::FindWidget;
use crate::search::{self, SearchData, SearchParams};
use crate::widgets::{OverlayPosition, OverlayWidget, WidgetId};

/// Monaco highlights at most this many matches.
pub const MATCHES_LIMIT: usize = 19_999;

/// The find widget's inputs and toggles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub query: String,
    pub replace: String,
    pub is_regex: bool,
    pub match_case: bool,
    pub whole_word: bool,
    /// Search only within the selection captured when this was turned on.
    pub in_selection: bool,
}

#[derive(Default)]
pub(crate) struct FindState {
    pub options: FindOptions,
    pub visible: bool,
    pub replace_visible: bool,
    scope: Option<Range<Anchor>>,
    /// Matches as of `matches_version`, in byte offsets.
    pub matches: Vec<Range<usize>>,
    matches_version: Option<Version>,
    pub current: Option<usize>,
    /// Where incremental search started (Monaco searches from the original
    /// selection as you type).
    search_start: Option<Anchor>,
    decorations: Vec<DecorationId>,
    current_decoration: Vec<DecorationId>,
    task: Option<Task<()>>,
    widget: Option<(Entity<FindWidget>, WidgetId)>,
}

impl FindState {
    /// "3 of 15", "No results", or `None` when there's no query.
    pub fn status(&self) -> Option<String> {
        if self.options.query.is_empty() {
            return None;
        }
        if self.matches.is_empty() {
            return Some("No results".into());
        }
        let total = if self.matches.len() >= MATCHES_LIMIT {
            format!("{MATCHES_LIMIT}+")
        } else {
            self.matches.len().to_string()
        };
        Some(match self.current {
            Some(ix) => format!("{} of {total}", ix + 1),
            None => format!("? of {total}"),
        })
    }
}

impl CodeEditor {
    pub fn find_options(&self) -> &FindOptions {
        &self.find.options
    }

    pub fn is_find_visible(&self) -> bool {
        self.find.visible
    }

    /// The find widget's match counter text.
    pub fn find_status(&self) -> Option<String> {
        self.find.status()
    }

    /// Matches found for the current query, in byte offsets.
    pub fn find_matches(&self) -> &[Range<usize>] {
        &self.find.matches
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

    fn scope_range(&self, cx: &gpui::App) -> Option<Range<usize>> {
        let scope = self.find.scope.as_ref()?;
        let buffer = self.model().read(cx).buffer();
        Some(buffer.resolve_range(scope))
    }

    /// Updates the query and toggles. While the widget is open, the selection
    /// moves to the first match after where the search started.
    pub fn set_find_options(&mut self, options: FindOptions, cx: &mut Context<Self>) {
        if options.in_selection && !self.find.options.in_selection {
            let selection = self.cursors.primary().selection();
            let buffer = self.model().read(cx).buffer();
            let start = buffer.point_utf16_to_offset(selection.start());
            let end = buffer.point_utf16_to_offset(selection.end());
            self.find.scope =
                (start < end).then(|| buffer.anchor_before(start)..buffer.anchor_after(end));
        } else if !options.in_selection {
            self.find.scope = None;
        }
        let query_changed = options.query != self.find.options.query
            || options.is_regex != self.find.options.is_regex
            || options.match_case != self.find.options.match_case
            || options.whole_word != self.find.options.whole_word
            || options.in_selection != self.find.options.in_selection;
        self.find.options = options;
        if query_changed {
            self.find.matches_version = None;
            if self.find.visible {
                self.move_to_match_from_search_start(cx);
            }
            self.refresh_find_matches(cx);
        }
        cx.notify();
    }

    fn move_to_match_from_search_start(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.search_data() else {
            return;
        };
        let buffer = self.model().read(cx).buffer().clone();
        let start = self
            .find
            .search_start
            .map(|anchor| buffer.resolve(&anchor))
            .unwrap_or(0);
        if let Some(found) = self.next_match_in_scope(&data, start, cx) {
            self.select_match(found, cx);
        }
    }

    fn next_match_in_scope(
        &self,
        data: &SearchData,
        from: usize,
        cx: &gpui::App,
    ) -> Option<Range<usize>> {
        let rope = self.snapshot.rope();
        match self.scope_range(cx) {
            Some(scope) => {
                let matches = search::find_matches(rope, data, scope.clone(), MATCHES_LIMIT);
                matches
                    .iter()
                    .find(|m| m.start >= from)
                    .or(matches.first())
                    .cloned()
            }
            None => search::find_next(rope, data, from),
        }
    }

    fn previous_match_in_scope(
        &self,
        data: &SearchData,
        before: usize,
        cx: &gpui::App,
    ) -> Option<Range<usize>> {
        let rope = self.snapshot.rope();
        match self.scope_range(cx) {
            Some(scope) => {
                let matches = search::find_matches(rope, data, scope, MATCHES_LIMIT);
                matches
                    .iter()
                    .rev()
                    .find(|m| m.end <= before)
                    .or(matches.last())
                    .cloned()
            }
            None => search::find_previous(rope, data, before),
        }
    }

    fn select_match(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let rope = self.snapshot.rope();
        let selection = Selection {
            anchor: rope.offset_to_point_utf16(range.start),
            head: rope.offset_to_point_utf16(range.end),
        };
        self.find.current = self
            .find
            .matches
            .binary_search_by_key(&range.start, |m| m.start)
            .ok();
        self.set_selections(&[selection], cx);
        self.update_current_match_decoration(Some(range), cx);
    }

    fn selection_offsets(&self) -> Range<usize> {
        let selection = self.cursors.primary().selection();
        let rope = self.snapshot.rope();
        rope.point_utf16_to_offset(selection.start())..rope.point_utf16_to_offset(selection.end())
    }

    /// Selects the next match after the selection (Monaco `nextMatchFindAction`).
    pub fn find_next(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.search_data() else {
            return;
        };
        let selection = self.selection_offsets();
        let from = if selection.is_empty() {
            selection.start
        } else {
            selection.end
        };
        let found = self
            .next_match_in_scope(&data, from, cx)
            .filter(|m| !(m.is_empty() && m.start == from && !selection.is_empty()));
        // An empty match at the cursor would never advance; step past it.
        let found = match found {
            Some(m)
                if m.is_empty()
                    && m.start == from
                    && selection.is_empty()
                    && from < self.snapshot.len() =>
            {
                let next = self.snapshot.clip_offset(from + 1, Bias::Right);
                self.next_match_in_scope(&data, next, cx)
            }
            other => other,
        };
        if let Some(found) = found {
            self.select_match(found, cx);
        }
    }

    /// Selects the match before the selection (Monaco
    /// `previousMatchFindAction`).
    pub fn find_previous(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.search_data() else {
            return;
        };
        let selection = self.selection_offsets();
        if let Some(found) = self.previous_match_in_scope(&data, selection.start, cx) {
            self.select_match(found, cx);
        }
    }

    /// Replaces the selected match and moves to the next one; if the
    /// selection isn't a match, just finds the next.
    pub fn replace_one(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.search_data() else {
            return;
        };
        let selection = self.selection_offsets();
        let is_match = search::find_matches(self.snapshot.rope(), &data, selection.clone(), 1)
            .first()
            .is_some_and(|m| *m == selection);
        if is_match && !self.options.read_only {
            let matched = self.snapshot.text_for_range(selection.clone());
            let replacement = search::expand_replacement(
                &data,
                self.find.options.is_regex,
                &matched,
                &self.find.options.replace,
            );
            let start = self.snapshot.offset_to_point_utf16(selection.start);
            let end = self.snapshot.offset_to_point_utf16(selection.end);
            let after = self.snapshot.offset_to_point_utf16(selection.start);
            let mut after_end = after;
            for ch in replacement.chars() {
                if ch == '\n' {
                    after_end = PointUtf16::new(after_end.row + 1, 0);
                } else {
                    after_end.column += ch.len_utf16() as u32;
                }
            }
            self.apply(
                cursor::EditOperation {
                    edits: vec![(start..end, replacement)],
                    selections_after: vec![Selection::collapsed(after_end)],
                    kind: cursor::EditOperationType::Other,
                    push_stack_element_before: true,
                    push_stack_element_after: true,
                },
                cx,
            );
        }
        self.find_next(cx);
    }

    /// Replaces every match in one undo step (Monaco `replaceAll`).
    pub fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.search_data() else {
            return;
        };
        if self.options.read_only {
            return;
        }
        let rope = self.snapshot.rope().clone();
        let range = self.scope_range(cx).unwrap_or(0..rope.len());
        let matches = search::find_matches(&rope, &data, range, usize::MAX);
        if matches.is_empty() {
            return;
        }
        let edits: Vec<(Range<usize>, String)> = matches
            .iter()
            .map(|m| {
                let matched = rope.text_for_range(m.clone());
                let replacement = search::expand_replacement(
                    &data,
                    self.find.options.is_regex,
                    &matched,
                    &self.find.options.replace,
                );
                (m.clone(), replacement)
            })
            .collect();
        let source = cx.entity_id();
        let before = crate::model::metadata(self.cursors.selections());
        self.model().update(cx, |model, cx| {
            model.push_stack_element();
            if model.edit_from(edits, None, Some(source), cx).is_ok() {
                model.set_undo_metadata(Some(before), None);
            }
            model.push_stack_element();
        });
    }

    /// Selects every match (Monaco `selectAllMatches`, Alt+Enter).
    pub fn select_all_matches(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.search_data() else {
            return;
        };
        let rope = self.snapshot.rope();
        let range = self.scope_range(cx).unwrap_or(0..rope.len());
        let selections: Vec<Selection> = search::find_matches(rope, &data, range, MATCHES_LIMIT)
            .into_iter()
            .map(|m| Selection {
                anchor: rope.offset_to_point_utf16(m.start),
                head: rope.offset_to_point_utf16(m.end),
            })
            .collect();
        if !selections.is_empty() {
            self.set_selections(&selections, cx);
        }
    }

    /// Recomputes match highlights in the background.
    pub(crate) fn refresh_find_matches(&mut self, cx: &mut Context<Self>) {
        let version = self.snapshot.version();
        if !self.find.visible || self.find.options.query.is_empty() {
            self.find.matches.clear();
            self.find.matches_version = None;
            self.find.current = None;
            self.set_find_decorations(Vec::new(), cx);
            self.update_current_match_decoration(None, cx);
            return;
        }
        if self.find.matches_version == Some(version) {
            return;
        }
        let Some(data) = self.search_data() else {
            self.find.matches.clear();
            self.set_find_decorations(Vec::new(), cx);
            return;
        };
        let rope = self.snapshot.rope().clone();
        let range = self.scope_range(cx).unwrap_or(0..rope.len());
        self.find.matches_version = Some(version);
        self.find.task = Some(cx.spawn(async move |this, cx| {
            let matches = cx
                .background_executor()
                .spawn(async move { search::find_matches(&rope, &data, range, MATCHES_LIMIT) })
                .await;
            this.update(cx, |this, cx| {
                if this.find.matches_version != Some(version) {
                    return;
                }
                let selection = this.selection_offsets();
                this.find.current = matches.iter().position(|m| *m == selection);
                this.find.matches = matches.clone();
                this.set_find_decorations(matches, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn set_find_decorations(&mut self, matches: Vec<Range<usize>>, cx: &mut Context<Self>) {
        let colors = self.theme.colors.clone();
        let owner = Some(cx.entity_id());
        let old = std::mem::take(&mut self.find.decorations);
        if old.is_empty() && matches.is_empty() {
            return;
        }
        let new = matches
            .into_iter()
            .map(|range| {
                (
                    range,
                    DecorationOptions {
                        background: Some(colors.find_match_highlight_background),
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
        self.find.decorations = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, new, cx)
        });
    }

    fn update_current_match_decoration(
        &mut self,
        range: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        let colors = self.theme.colors.clone();
        let owner = Some(cx.entity_id());
        let old = std::mem::take(&mut self.find.current_decoration);
        let new = range
            .filter(|_| self.find.visible)
            .map(|range| {
                (
                    range,
                    DecorationOptions {
                        background: Some(colors.find_match_background),
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        z_index: 1,
                        ..Default::default()
                    },
                )
            })
            .into_iter()
            .collect::<Vec<_>>();
        if old.is_empty() && new.is_empty() {
            return;
        }
        self.find.current_decoration = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, new, cx)
        });
    }

    /// Opens the find widget, seeding the query from the selection or the
    /// word at the cursor (Monaco `seedSearchStringFromSelection`).
    pub fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.cursors.primary().selection();
        let rope = self.snapshot.rope().clone();
        let seed = if !selection.is_empty() && selection.start().row == selection.end().row {
            Some(rope.text_for_range(
                rope.point_utf16_to_offset(selection.start())
                    ..rope.point_utf16_to_offset(selection.end()),
            ))
        } else if selection.is_empty() {
            let config = self.cursor_config(cx);
            cursor::word_at(&config, &rope, selection.head)
                .map(|word| {
                    rope.text_for_range(
                        rope.point_utf16_to_offset(word.start)
                            ..rope.point_utf16_to_offset(word.end),
                    )
                })
                .filter(|word| word.chars().any(|c| c.is_alphanumeric() || c == '_'))
        } else {
            None
        };
        let start = rope.point_utf16_to_offset(selection.start());
        self.find.search_start = Some(self.model().read(cx).buffer().anchor_before(start));
        self.find.visible = true;
        self.find.replace_visible = replace;
        self.find.matches_version = None;
        let mut options = self.find.options.clone();
        if let Some(seed) = seed {
            options.query = seed;
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
        self.find.options = FindOptions {
            query: String::new(),
            ..options.clone()
        };
        self.set_find_options(options.clone(), cx);
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
        self.refresh_find_matches(cx);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
}
