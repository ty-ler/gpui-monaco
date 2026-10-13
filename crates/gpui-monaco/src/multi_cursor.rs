//! Multi-cursor commands (Monaco's
//! `contrib/multicursor/browser/multicursor.ts`): add the next or previous
//! occurrence (Cmd/Ctrl+D), move the last selection to the next or previous
//! occurrence (Cmd/Ctrl+K Cmd/Ctrl+D), select all occurrences
//! (Cmd/Ctrl+Shift+L), cursors at line ends (Shift+Alt+I), cursors to the
//! top or bottom, and cycling which cursor is primary.
//!
//! As in Monaco, a Cmd/Ctrl+D session makes its text the find widget's
//! search string, and from the find widget's inputs these commands act on
//! the editor, searching for the widget's string.

use std::ops::Range;

use gpui::Context;

use crate::cursor::{self, Selection};
use crate::editor::CodeEditor;
use crate::search::{self, SearchParams};
use text::PointUtf16;

/// Monaco's `MultiCursorSession`: what the current Cmd/Ctrl+D session
/// searches for and how. Sessions started from an empty selection (a word)
/// are whole-word and case-sensitive.
#[derive(Clone, Debug, Default)]
pub(crate) struct MultiCursorSession {
    /// The selections this session last produced; any other change ends it.
    selections: Vec<Selection>,
    /// Monaco `searchText`.
    search_text: String,
    whole_word: bool,
    match_case: bool,
    /// Monaco `isDisconnectedFromFindController`: started from a word, so
    /// it overrides the find toggles while it lasts.
    from_word: bool,
    /// Monaco `usesFindOptions`: changing the find widget's match case or
    /// whole word ends the session.
    uses_find_options: bool,
    /// Monaco `currentMatch`: the primary cursor's word when the session
    /// started from an empty primary selection, the first step's result.
    current_match: Option<Selection>,
    /// Set while the session changes the selections itself (Monaco
    /// `_ignoreSelectionChange`).
    applying: bool,
}

impl CodeEditor {
    fn selection_range(&self, selection: &Selection) -> Range<usize> {
        let rope = self.snapshot.rope();
        rope.point_utf16_to_offset(selection.start())..rope.point_utf16_to_offset(selection.end())
    }

    fn selection_text(&self, selection: &Selection) -> String {
        self.snapshot
            .rope()
            .text_for_range(self.selection_range(selection))
    }

    /// Monaco `_expandEmptyToWord`: an empty selection grows to the word
    /// at it (`getConfiguredWordAtPosition`: any run of characters that
    /// aren't whitespace or word separators, letters or not).
    fn word_selection(&self, selection: &Selection, cx: &gpui::App) -> Selection {
        if !selection.is_empty() {
            return *selection;
        }
        let config = self.cursor_config(cx);
        cursor::word_at_position(&config, self.snapshot.rope(), selection.head)
            .map(|word| Selection {
                anchor: word.start,
                head: word.end,
            })
            .unwrap_or(*selection)
    }

    /// Match options for a session (whole word, match case, uses find
    /// options): whole-word and case-sensitive when started from a word,
    /// else per `selected_text_match_mode`.
    fn session_options(&self, from_word: bool) -> (bool, bool, bool) {
        if from_word {
            (true, true, true)
        } else {
            let (whole_word, match_case) = self.selected_text_match_options();
            (whole_word, match_case, self.uses_find_options())
        }
    }

    pub(crate) fn uses_find_options(&self) -> bool {
        self.options.selected_text_match_mode == crate::options::SelectedTextMatchMode::FindOptions
    }

    /// (whole word, match case) for selected text (Monaco
    /// `getSelectionSearchOptions`): with `FindOptions`, the find state's
    /// toggles, open or closed (both off until set); else substrings matched
    /// as the mode says.
    pub(crate) fn selected_text_match_options(&self) -> (bool, bool) {
        use crate::options::SelectedTextMatchMode;
        match self.options.selected_text_match_mode {
            SelectedTextMatchMode::FindOptions => {
                (self.find.options.whole_word, self.find.options.match_case)
            }
            SelectedTextMatchMode::CaseSensitive => (false, true),
            SelectedTextMatchMode::CaseInsensitive => (false, false),
        }
    }

    /// The running Cmd/Ctrl+D session (Monaco `getSession`): its search
    /// text, whole word and match case.
    pub(crate) fn multi_cursor_session(&self) -> Option<(&str, bool, bool)> {
        self.current_session()
            .map(|s| (s.search_text.as_str(), s.whole_word, s.match_case))
    }

    fn current_session(&self) -> Option<&MultiCursorSession> {
        self.multi_cursor
            .as_ref()
            .filter(|session| session.applying || session.selections == self.cursors.selections())
    }

    /// Monaco's `_endSession` on `onDidChangeCursorSelection`: a selection
    /// change the session didn't make ends it, and the find toggles it
    /// overrode get their own values back.
    pub(crate) fn multi_cursor_selections_changed(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.multi_cursor else {
            return;
        };
        if session.applying || session.selections == self.cursors.selections() {
            return;
        }
        self.end_multi_cursor_session(cx);
    }

    /// Monaco `_endSession`: no session, and the find toggles a session
    /// started from a word overrode get their own values back.
    pub(crate) fn end_multi_cursor_session(&mut self, cx: &mut Context<Self>) {
        if self.multi_cursor.take().is_some_and(|s| s.from_word) {
            self.clear_find_overrides(cx);
        }
    }

    /// Monaco's session listener on `onFindReplaceStateChange`: the find
    /// widget's match case or whole word changed, which ends a session
    /// that uses them.
    pub(crate) fn multi_cursor_find_toggles_changed(&mut self, cx: &mut Context<Self>) {
        if self
            .multi_cursor
            .as_ref()
            .is_some_and(|s| s.uses_find_options && !s.applying)
        {
            self.end_multi_cursor_session(cx);
        }
    }

    /// Monaco `isFindWidgetSearch`: from the find widget (focus not in the
    /// text), with the widget showing a search string, the widget owns what
    /// is searched for.
    fn is_find_widget_search(&self, from_find_widget: bool) -> bool {
        from_find_widget && self.find.visible && !self.find.options.query.is_empty()
    }

    /// Monaco `MultiCursorSession.create`: searches for the find widget's
    /// string, or for the primary selection's text (the word at an empty
    /// cursor).
    fn new_session(&self, from_find_widget: bool, cx: &gpui::App) -> Option<MultiCursorSession> {
        if self.is_find_widget_search(from_find_widget) {
            let find = &self.find.options;
            return Some(MultiCursorSession {
                search_text: find.query.clone(),
                whole_word: find.whole_word,
                match_case: find.match_case,
                uses_find_options: true,
                ..Default::default()
            });
        }
        let selections = self.cursors.selections();
        let from_word = selections.len() == 1 && selections[0].is_empty();
        let (whole_word, match_case, uses_find_options) = self.session_options(from_word);
        let selection = self.cursors.primary().selection();
        let primary = self.word_selection(&selection, cx);
        if primary.is_empty() {
            return None;
        }
        Some(MultiCursorSession {
            search_text: self.selection_text(&primary).replace("\r\n", "\n"),
            whole_word,
            match_case,
            from_word,
            uses_find_options,
            current_match: selection.is_empty().then_some(primary),
            ..Default::default()
        })
    }

    /// Monaco `_beginSessionIfNeeded`: the running session, or a new one.
    fn begin_session(
        &mut self,
        from_find_widget: bool,
        cx: &mut Context<Self>,
    ) -> Option<MultiCursorSession> {
        if let Some(session) = self.current_session() {
            return Some(session.clone());
        }
        let session = self.new_session(from_find_widget, cx)?;
        self.start_multi_cursor_session(session.clone(), cx);
        Some(session)
    }

    /// Makes `session` the running one; its text becomes the find widget's
    /// search string (Monaco `findController.getState().change`).
    fn start_multi_cursor_session(&mut self, session: MultiCursorSession, cx: &mut Context<Self>) {
        let (search, from_word) = (session.search_text.clone(), session.from_word);
        self.multi_cursor = Some(MultiCursorSession {
            selections: self.cursors.selections(),
            applying: true,
            ..session
        });
        self.seed_find_from_multi_cursor(search, from_word, cx);
        if let Some(session) = self.multi_cursor.as_mut() {
            session.applying = false;
        }
    }

    /// Monaco `_setSelections`: sets the session's result, keeping the
    /// session.
    fn apply_session_selections(&mut self, selections: &[Selection], cx: &mut Context<Self>) {
        if let Some(session) = self.multi_cursor.as_mut() {
            session.applying = true;
        }
        self.set_selections(selections, cx);
        let selections = self.cursors.selections();
        if let Some(session) = self.multi_cursor.as_mut() {
            session.applying = false;
            session.selections = selections;
        }
    }

    /// Whether every selection has the same text (Monaco
    /// `modelRangesContainSameText`); an empty selection after the first
    /// never does.
    fn selections_share_text(&self, match_case: bool) -> bool {
        let selections = self.cursors.selections();
        let Some((first, rest)) = selections.split_first() else {
            return true;
        };
        let text = |s: &Selection| {
            let text = self.selection_text(s);
            if match_case {
                text
            } else {
                text.to_lowercase()
            }
        };
        let first = text(first);
        rest.iter().all(|s| !s.is_empty() && text(s) == first)
    }

    /// Monaco `MultiCursorSession._highlightFindOptions`, on each search
    /// step and on select all: a session that uses the find options (any
    /// started from a word) shows them, with find closed in the options
    /// widget for 2s.
    fn highlight_session_find_options(
        &mut self,
        session: &MultiCursorSession,
        cx: &mut Context<Self>,
    ) {
        if session.uses_find_options {
            self.highlight_find_options(cx);
        }
    }

    fn session_search(&self, session: &MultiCursorSession) -> Option<search::SearchData> {
        SearchParams {
            query: session.search_text.clone(),
            is_regex: false,
            match_case: session.match_case,
            word_separators: session
                .whole_word
                .then(|| self.options.word_separators.to_string()),
        }
        .parse()
    }

    fn range_selection(&self, range: Range<usize>) -> Selection {
        let rope = self.snapshot.rope();
        Selection {
            anchor: rope.offset_to_point_utf16(range.start),
            head: rope.offset_to_point_utf16(range.end),
        }
    }

    /// Monaco `_getNextMatch` / `_getPreviousMatch`: the session's first
    /// result (the primary cursor's word, when it started from an empty
    /// one), then the next match after the last selection (or previous one
    /// before it). Begins a session if there is none. `None` when there is
    /// nothing to do.
    fn session_step(
        &mut self,
        next: bool,
        from_find_widget: bool,
        cx: &mut Context<Self>,
    ) -> Option<Selection> {
        let session = self.begin_session(from_find_widget, cx)?;
        if let Some(found) = self.multi_cursor.as_mut()?.current_match.take() {
            return Some(found);
        }
        self.highlight_session_find_options(&session, cx);
        let data = self.session_search(&session)?;
        let last = *self.cursors.selections().last()?;
        let range = self.selection_range(&last);
        let rope = self.snapshot.rope();
        let found = if next {
            search::find_next(rope, &data, range.end)
        } else {
            search::find_previous(rope, &data, range.start)
        }?;
        Some(self.range_selection(found))
    }

    /// Monaco `MultiCursorSelectionController.addSelectionToNextFindMatch`:
    /// without a session, several selections that don't share their text
    /// only grow to their words.
    fn add_selection_to_next_find_match_inner(
        &mut self,
        from_find_widget: bool,
        cx: &mut Context<Self>,
    ) {
        if self.current_session().is_none() && self.cursors.selections().len() > 1 {
            let match_case = if self.is_find_widget_search(from_find_widget) {
                self.find.options.match_case
            } else {
                self.selected_text_match_options().1
            };
            if !self.selections_share_text(match_case) {
                let expanded: Vec<Selection> = self
                    .cursors
                    .selections()
                    .iter()
                    .map(|s| self.word_selection(s, cx))
                    .collect();
                self.end_multi_cursor_session(cx);
                self.set_selections(&expanded, cx);
                return;
            }
        }
        self.add_selection_to_find_match_inner(true, from_find_widget, cx);
    }

    fn add_selection_to_find_match_inner(
        &mut self,
        next: bool,
        from_find_widget: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(found) = self.session_step(next, from_find_widget, cx) else {
            return;
        };
        let mut selections = self.cursors.selections();
        if !selections.contains(&found) {
            selections.push(found);
            self.apply_session_selections(&selections, cx);
        }
        self.reveal_session_match(found, cx);
    }

    /// Monaco `_applySessionResult`: the new match shows, centered if it
    /// was off screen (`revealRangeInCenterIfOutsideViewport`).
    fn reveal_session_match(&mut self, found: Selection, cx: &mut Context<Self>) {
        self.reveal_range_in_center_if_outside_viewport(found.start()..found.end(), cx);
    }

    fn move_selection_to_find_match_inner(
        &mut self,
        next: bool,
        from_find_widget: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(found) = self.session_step(next, from_find_widget, cx) else {
            return;
        };
        let mut selections = self.cursors.selections();
        if let Some(slot) = selections.last_mut() {
            *slot = found;
        }
        self.apply_session_selections(&selections, cx);
        self.reveal_session_match(found, cx);
    }

    fn insert_cursor_at_end_of_each_line_selected_inner(&mut self, cx: &mut Context<Self>) {
        let rope = self.snapshot.rope();
        let mut cursors = Vec::new();
        for selection in self.cursors.selections() {
            if selection.is_empty() {
                continue;
            }
            let (start, end) = (selection.start(), selection.end());
            for row in start.row..end.row {
                cursors.push(Selection::collapsed(PointUtf16::new(
                    row,
                    rope.line_len_utf16(row),
                )));
            }
            if end.column > 0 {
                cursors.push(Selection::collapsed(end));
            }
        }
        if !cursors.is_empty() {
            self.set_selections(&cursors, cx);
        }
    }

    fn add_cursors_to_edge_inner(&mut self, down: bool, cx: &mut Context<Self>) {
        let first = self.cursors.selections()[0];
        let (start, end) = (first.start(), first.end());
        let rows: Vec<u32> = if down {
            (start.row..=self.snapshot.rope().max_point().row).collect()
        } else {
            (0..=start.row).rev().collect()
        };
        let rope = self.snapshot.rope();
        let selections: Vec<Selection> = rows
            .into_iter()
            .map(|row| {
                let len = rope.line_len_utf16(row);
                Selection {
                    anchor: PointUtf16::new(row, start.column.min(len)),
                    head: PointUtf16::new(row, end.column.min(len)),
                }
            })
            .collect();
        self.set_selections(&selections, cx);
    }

    fn focus_next_cursor_inner(&mut self, next: bool, cx: &mut Context<Self>) {
        let mut selections = self.cursors.selections();
        if selections.len() < 2 {
            return;
        }
        if next {
            selections.rotate_left(1);
        } else {
            selections.rotate_right(1);
        }
        self.set_selections(&selections, cx);
        // Monaco `revealPrimaryCursor(source, true)`.
        self.reveal_primary_cursor(cx);
    }

    /// The matches of `data` in the find scope (Monaco `searchScope`), or
    /// in the whole text.
    fn occurrences_in_scope(&self, data: &search::SearchData) -> Vec<Range<usize>> {
        let rope = self.snapshot.rope();
        match self.find_search_scope() {
            Some(scopes) => {
                let mut scopes: Vec<Range<usize>> = scopes
                    .iter()
                    .map(|s| rope.point_utf16_to_offset(s.start)..rope.point_utf16_to_offset(s.end))
                    .collect();
                scopes.sort_by_key(|s| s.start);
                scopes
                    .into_iter()
                    .flat_map(|scope| search::find_matches(rope, data, scope, usize::MAX))
                    .collect()
            }
            None => search::find_matches(rope, data, 0..rope.len(), usize::MAX),
        }
    }

    /// Monaco `MultiCursorSelectionController.selectAll`.
    fn select_all_occurrences_inner(&mut self, from_find_widget: bool, cx: &mut Context<Self>) {
        use crate::options::SelectedTextMatchMode;
        // Monaco: the find widget owns what is searched for when its
        // search string is a regex (from the widget, or with
        // `selectedTextMatchMode: findOptions`).
        let allow_find_regex = from_find_widget
            || self.options.selected_text_match_mode == SelectedTextMatchMode::FindOptions;
        let find = &self.find.options;
        let mut matches =
            if allow_find_regex && self.find.visible && !find.query.is_empty() && find.is_regex {
                let params = SearchParams {
                    query: find.query.clone(),
                    is_regex: true,
                    match_case: find.match_case,
                    word_separators: find
                        .whole_word
                        .then(|| self.options.word_separators.to_string()),
                };
                let Some(data) = params.parse() else {
                    return;
                };
                // No session: these selections end any running one.
                self.end_multi_cursor_session(cx);
                self.occurrences_in_scope(&data)
            } else {
                let Some(session) = self.begin_session(from_find_widget, cx) else {
                    return;
                };
                self.highlight_session_find_options(&session, cx);
                let Some(data) = self.session_search(&session) else {
                    return;
                };
                self.occurrences_in_scope(&data)
            };
        if matches.is_empty() {
            return;
        }
        // The cursor where it was invoked stays primary.
        let primary = self.selection_range(&self.cursors.primary().selection());
        if let Some(ix) = matches
            .iter()
            .position(|m| m.start <= primary.end && primary.start <= m.end)
        {
            matches.swap(0, ix);
        }
        let selections: Vec<Selection> = matches
            .into_iter()
            .map(|m| self.range_selection(m))
            .collect();
        self.apply_session_selections(&selections, cx);
    }

    /// Adds the previous occurrence of the selected text as a selection
    /// (Monaco `editor.action.addSelectionToPreviousFindMatch`).
    pub fn add_selection_to_previous_find_match(&mut self, cx: &mut Context<Self>) {
        self.multi_cursor_command(false, cx, |this, from, cx| {
            this.add_selection_to_find_match_inner(false, from, cx)
        });
    }

    /// Moves the last selection to the previous occurrence (Monaco
    /// `editor.action.moveSelectionToPreviousFindMatch`).
    pub fn move_selection_to_previous_find_match(&mut self, cx: &mut Context<Self>) {
        self.multi_cursor_command(false, cx, |this, from, cx| {
            this.move_selection_to_find_match_inner(false, from, cx)
        });
    }

    /// Shift+Alt+I: a cursor at the end of every line each selection
    /// covers (Monaco `editor.action.insertCursorAtEndOfEachLineSelected`).
    pub fn insert_cursor_at_end_of_each_line_selected(&mut self, cx: &mut Context<Self>) {
        let before = self.cursors.selections();
        self.insert_cursor_at_end_of_each_line_selected_inner(cx);
        self.announce_cursor_change(&before, cx);
    }

    /// The primary selection's columns repeated on every line from it to
    /// the last (`down`) or first line (Monaco `editor.action.addCursorsToBottom`
    /// / `addCursorsToTop`).
    pub fn add_cursors_to_edge(&mut self, down: bool, cx: &mut Context<Self>) {
        let before = self.cursors.selections();
        self.add_cursors_to_edge_inner(down, cx);
        self.announce_cursor_change(&before, cx);
    }

    /// Makes the next (or previous) cursor primary (Monaco
    /// `editor.action.focusNextCursor` / `focusPreviousCursor`), after an
    /// undo stop. Does nothing in a read-only editor.
    pub fn focus_next_cursor(&mut self, next: bool, cx: &mut Context<Self>) {
        if self.options.read_only {
            return;
        }
        self.push_undo_stop(cx);
        let before = self.cursors.selections();
        self.focus_next_cursor_inner(next, cx);
        self.announce_cursor_change(&before, cx);
    }

    /// Cmd/Ctrl+D: select the word, or add the next occurrence of the
    /// selected text as a new selection.
    pub fn add_selection_to_next_find_match(&mut self, cx: &mut Context<Self>) {
        self.multi_cursor_command(false, cx, |this, from, cx| {
            this.add_selection_to_next_find_match_inner(from, cx)
        });
    }

    /// Cmd/Ctrl+K Cmd/Ctrl+D: move the last selection to the next
    /// occurrence.
    pub fn move_selection_to_next_find_match(&mut self, cx: &mut Context<Self>) {
        self.multi_cursor_command(false, cx, |this, from, cx| {
            this.move_selection_to_find_match_inner(true, from, cx)
        });
    }

    /// Cmd/Ctrl+Shift+L: select every occurrence of the selection (or of
    /// the word at the cursor). The current selection stays primary.
    pub fn select_all_occurrences(&mut self, cx: &mut Context<Self>) {
        self.multi_cursor_command(false, cx, |this, from, cx| {
            this.select_all_occurrences_inner(from, cx)
        });
    }

    /// Runs a multicursor `command` as Monaco's `MultiCursorSelectionControllerAction`
    /// does from the find widget's inputs (`editorFocus` without text
    /// focus): the widget's search string is what's searched for.
    pub(crate) fn multi_cursor_from_find_widget(
        &mut self,
        command: &dyn gpui::Action,
        cx: &mut Context<Self>,
    ) -> bool {
        use crate::actions::{
            AddSelectionToNextFindMatch, AddSelectionToPreviousFindMatch,
            MoveSelectionToNextFindMatch, MoveSelectionToPreviousFindMatch, SelectAllOccurrences,
        };
        let any = command.as_any();
        type Run = fn(&mut CodeEditor, bool, &mut Context<CodeEditor>);
        let run: Run = if any.is::<AddSelectionToNextFindMatch>() {
            |this, from, cx| this.add_selection_to_next_find_match_inner(from, cx)
        } else if any.is::<AddSelectionToPreviousFindMatch>() {
            |this, from, cx| this.add_selection_to_find_match_inner(false, from, cx)
        } else if any.is::<MoveSelectionToNextFindMatch>() {
            |this, from, cx| this.move_selection_to_find_match_inner(true, from, cx)
        } else if any.is::<MoveSelectionToPreviousFindMatch>() {
            |this, from, cx| this.move_selection_to_find_match_inner(false, from, cx)
        } else if any.is::<SelectAllOccurrences>() {
            |this, from, cx| this.select_all_occurrences_inner(from, cx)
        } else {
            return false;
        };
        self.multi_cursor_command(true, cx, run);
        true
    }

    fn multi_cursor_command(
        &mut self,
        from_find_widget: bool,
        cx: &mut Context<Self>,
        run: impl FnOnce(&mut Self, bool, &mut Context<Self>),
    ) {
        let before = self.cursors.selections();
        run(self, from_find_widget, cx);
        self.announce_cursor_change(&before, cx);
    }

    /// Monaco's `announceCursorChange`: screen readers hear where the
    /// multicursor commands added cursors.
    pub(crate) fn announce_cursor_change(&mut self, before: &[Selection], cx: &mut Context<Self>) {
        let added: Vec<String> = self
            .cursors
            .selections()
            .iter()
            .filter(|s| !before.contains(s))
            .map(|s| format!("line {} column {}", s.head.row + 1, s.head.column + 1))
            .collect();
        if added.is_empty() {
            return;
        }
        let message = if added.len() == 1 {
            format!("Cursor added: {}", added[0])
        } else {
            format!("Cursors added: {}", added.join(", "))
        };
        self.announce_status(message, cx);
    }
}
