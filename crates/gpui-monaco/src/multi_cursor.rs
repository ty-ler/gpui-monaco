//! Multi-cursor commands (Monaco's
//! `contrib/multicursor/browser/multicursor.ts`): add the next or previous
//! occurrence (Cmd/Ctrl+D), move the last selection to the next or previous
//! occurrence (Cmd/Ctrl+K Cmd/Ctrl+D), select all occurrences
//! (Cmd/Ctrl+Shift+L), cursors at line ends (Shift+Alt+I), cursors to the
//! top or bottom, and cycling which cursor is primary.

use std::ops::Range;

use gpui::Context;

use crate::cursor::{self, Selection};
use crate::editor::CodeEditor;
use crate::search::{self, SearchParams};
use text::PointUtf16;

/// How the current Cmd+D session matches text: sessions started from an
/// empty selection (a word) are whole-word and case-sensitive.
#[derive(Clone, Debug, Default)]
pub(crate) struct MultiCursorSession {
    /// The selections this session last produced; any other change ends it.
    selections: Vec<Selection>,
    whole_word: bool,
    match_case: bool,
}

impl CodeEditor {
    fn selection_range(&self, selection: &Selection) -> Range<usize> {
        let rope = self.snapshot.rope();
        rope.point_utf16_to_offset(selection.start())..rope.point_utf16_to_offset(selection.end())
    }

    fn word_selection(&self, selection: &Selection, cx: &gpui::App) -> Selection {
        if !selection.is_empty() {
            return *selection;
        }
        let config = self.cursor_config(cx);
        cursor::word_at(&config, self.snapshot.rope(), selection.head)
            .filter(|word| {
                let rope = self.snapshot.rope();
                let text = rope.text_for_range(
                    rope.point_utf16_to_offset(word.start)..rope.point_utf16_to_offset(word.end),
                );
                text.chars().any(|c| c.is_alphanumeric() || c == '_')
            })
            .map(|word| Selection {
                anchor: word.start,
                head: word.end,
            })
            .unwrap_or(*selection)
    }

    /// Match options for a session: whole-word and case-sensitive when
    /// started from a word, else per `selected_text_match_mode`.
    fn session_options(&self, from_word: bool) -> (bool, bool) {
        if from_word {
            (true, true)
        } else {
            self.selected_text_match_options()
        }
    }

    /// (whole word, match case) for selected text (Monaco
    /// `getSelectionSearchOptions`): the find widget's options while it's
    /// open with `FindOptions`, else substrings matched as the mode says.
    pub(crate) fn selected_text_match_options(&self) -> (bool, bool) {
        use crate::options::SelectedTextMatchMode;
        match self.options.selected_text_match_mode {
            SelectedTextMatchMode::FindOptions if self.find.visible => {
                (self.find.options.whole_word, self.find.options.match_case)
            }
            SelectedTextMatchMode::FindOptions | SelectedTextMatchMode::CaseSensitive => {
                (false, true)
            }
            SelectedTextMatchMode::CaseInsensitive => (false, false),
        }
    }

    fn current_session(&self) -> Option<&MultiCursorSession> {
        self.multi_cursor
            .as_ref()
            .filter(|session| session.selections == self.cursors.selections())
    }

    /// Expands empty selections to their words. Returns whether any changed.
    fn expand_empty_selections(&mut self, cx: &mut Context<Self>) -> bool {
        let selections = self.cursors.selections();
        if selections.iter().all(|s| !s.is_empty()) {
            return false;
        }
        let expanded: Vec<Selection> = selections
            .iter()
            .map(|s| self.word_selection(s, cx))
            .collect();
        let from_word = selections.len() == 1 || selections.iter().all(|s| s.is_empty());
        let (whole_word, match_case) = self.session_options(from_word);
        self.set_selections(&expanded, cx);
        self.multi_cursor = Some(MultiCursorSession {
            selections: self.cursors.selections(),
            whole_word,
            match_case,
        });
        true
    }

    /// The text shared by all selections, if they all select the same text.
    fn shared_selection_text(&self, match_case: bool) -> Option<String> {
        let rope = self.snapshot.rope();
        let mut texts = self
            .cursors
            .selections()
            .into_iter()
            .map(|s| rope.text_for_range(self.selection_range(&s)));
        let first = texts.next()?;
        let same = texts.all(|text| {
            if match_case {
                text == first
            } else {
                text.to_lowercase() == first.to_lowercase()
            }
        });
        (same && !first.is_empty()).then_some(first)
    }

    fn next_occurrence(
        &self,
        after: usize,
        whole_word: bool,
        match_case: bool,
    ) -> Option<Range<usize>> {
        let text = self.shared_selection_text(match_case)?;
        let params = SearchParams {
            query: text,
            is_regex: false,
            match_case,
            word_separators: whole_word.then(|| self.options.word_separators.to_string()),
        };
        let data = params.parse()?;
        search::find_next(self.snapshot.rope(), &data, after)
    }

    fn previous_occurrence(
        &self,
        before: usize,
        whole_word: bool,
        match_case: bool,
    ) -> Option<Range<usize>> {
        let text = self.shared_selection_text(match_case)?;
        let params = SearchParams {
            query: text,
            is_regex: false,
            match_case,
            word_separators: whole_word.then(|| self.options.word_separators.to_string()),
        };
        let data = params.parse()?;
        search::find_previous(self.snapshot.rope(), &data, before)
    }

    fn range_selection(&self, range: Range<usize>) -> Selection {
        let rope = self.snapshot.rope();
        Selection {
            anchor: rope.offset_to_point_utf16(range.start),
            head: rope.offset_to_point_utf16(range.end),
        }
    }

    /// The previous occurrence before the last selection (Monaco
    /// `_getPreviousMatch`), after expanding empty selections like Cmd+D.
    /// `None` when there is nothing to do.
    fn previous_match(&mut self, cx: &mut Context<Self>) -> Option<(Selection, bool, bool)> {
        if self.expand_empty_selections(cx) {
            return None;
        }
        let (whole_word, match_case) = self.session_flags();
        let last = *self.cursors.selections().last()?;
        let before = self.selection_range(&last).start;
        let found = self.previous_occurrence(before, whole_word, match_case)?;
        Some((self.range_selection(found), whole_word, match_case))
    }

    /// Adds the previous occurrence of the selected text as a selection
    /// (Monaco `editor.action.addSelectionToPreviousFindMatch`).
    pub fn add_selection_to_previous_find_match(&mut self, cx: &mut Context<Self>) {
        let Some((found, whole_word, match_case)) = self.previous_match(cx) else {
            return;
        };
        let mut selections = self.cursors.selections();
        if selections.contains(&found) {
            return;
        }
        selections.push(found);
        self.set_selections(&selections, cx);
        self.multi_cursor = Some(MultiCursorSession {
            selections: self.cursors.selections(),
            whole_word,
            match_case,
        });
    }

    /// Moves the last selection to the previous occurrence (Monaco
    /// `editor.action.moveSelectionToPreviousFindMatch`).
    pub fn move_selection_to_previous_find_match(&mut self, cx: &mut Context<Self>) {
        let Some((found, whole_word, match_case)) = self.previous_match(cx) else {
            return;
        };
        let mut selections = self.cursors.selections();
        if let Some(slot) = selections.last_mut() {
            *slot = found;
        }
        self.set_selections(&selections, cx);
        self.multi_cursor = Some(MultiCursorSession {
            selections: self.cursors.selections(),
            whole_word,
            match_case,
        });
    }

    /// Shift+Alt+I: a cursor at the end of every line each selection
    /// covers (Monaco `editor.action.insertCursorAtEndOfEachLineSelected`).
    pub fn insert_cursor_at_end_of_each_line_selected(&mut self, cx: &mut Context<Self>) {
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

    /// The primary selection's columns repeated on every line from it to
    /// the last (`down`) or first line (Monaco `editor.action.addCursorsToBottom`
    /// / `addCursorsToTop`).
    pub fn add_cursors_to_edge(&mut self, down: bool, cx: &mut Context<Self>) {
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

    /// Makes the next (or previous) cursor primary (Monaco
    /// `editor.action.focusNextCursor` / `focusPreviousCursor`).
    pub fn focus_next_cursor(&mut self, next: bool, cx: &mut Context<Self>) {
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
    }

    fn session_flags(&self) -> (bool, bool) {
        match self.current_session() {
            Some(session) => (session.whole_word, session.match_case),
            None => self.session_options(false),
        }
    }

    /// Cmd/Ctrl+D: select the word, or add the next occurrence of the
    /// selected text as a new selection.
    pub fn add_selection_to_next_find_match(&mut self, cx: &mut Context<Self>) {
        if self.expand_empty_selections(cx) {
            return;
        }
        let (whole_word, match_case) = self.session_flags();
        let selections = self.cursors.selections();
        let last = selections.last().copied().unwrap_or_else(|| selections[0]);
        let after = self.selection_range(&last).end;
        let Some(found) = self.next_occurrence(after, whole_word, match_case) else {
            return;
        };
        if selections.iter().any(|s| self.selection_range(s) == found) {
            return;
        }
        let rope = self.snapshot.rope();
        let mut new_selections = selections.clone();
        new_selections.push(Selection {
            anchor: rope.offset_to_point_utf16(found.start),
            head: rope.offset_to_point_utf16(found.end),
        });
        self.set_selections(&new_selections, cx);
        self.multi_cursor = Some(MultiCursorSession {
            selections: self.cursors.selections(),
            whole_word,
            match_case,
        });
    }

    /// Cmd/Ctrl+K Cmd/Ctrl+D: move the last selection to the next
    /// occurrence.
    pub fn move_selection_to_next_find_match(&mut self, cx: &mut Context<Self>) {
        if self.expand_empty_selections(cx) {
            return;
        }
        let (whole_word, match_case) = self.session_flags();
        let mut selections = self.cursors.selections();
        let Some(last) = selections.last().copied() else {
            return;
        };
        let after = self.selection_range(&last).end;
        let Some(found) = self.next_occurrence(after, whole_word, match_case) else {
            return;
        };
        let rope = self.snapshot.rope();
        let moved = Selection {
            anchor: rope.offset_to_point_utf16(found.start),
            head: rope.offset_to_point_utf16(found.end),
        };
        if let Some(slot) = selections.last_mut() {
            *slot = moved;
        }
        self.set_selections(&selections, cx);
        self.multi_cursor = Some(MultiCursorSession {
            selections: self.cursors.selections(),
            whole_word,
            match_case,
        });
    }

    /// Cmd/Ctrl+Shift+L: select every occurrence of the selection (or of
    /// the word at the cursor). The current selection stays primary.
    pub fn select_all_occurrences(&mut self, cx: &mut Context<Self>) {
        let primary = self.cursors.primary().selection();
        let from_word = primary.is_empty();
        let primary = self.word_selection(&primary, cx);
        if primary.is_empty() {
            return;
        }
        let (whole_word, match_case) = self.session_options(from_word);
        let rope = self.snapshot.rope().clone();
        let primary_range = self.selection_range(&primary);
        // Monaco: the find widget's regex search applies with
        // `selectedTextMatchMode: findOptions`.
        let params = if self.find.visible
            && !from_word
            && self.options.selected_text_match_mode
                == crate::options::SelectedTextMatchMode::FindOptions
        {
            SearchParams {
                query: self.find.options.query.clone(),
                is_regex: self.find.options.is_regex,
                match_case: self.find.options.match_case,
                word_separators: self
                    .find
                    .options
                    .whole_word
                    .then(|| self.options.word_separators.to_string()),
            }
        } else {
            SearchParams {
                query: rope.text_for_range(primary_range.clone()),
                is_regex: false,
                match_case,
                word_separators: whole_word.then(|| self.options.word_separators.to_string()),
            }
        };
        let Some(data) = params.parse() else {
            return;
        };
        let matches = search::find_matches(&rope, &data, 0..rope.len(), crate::find::MATCHES_LIMIT);
        let mut selections = vec![primary];
        selections.extend(
            matches
                .into_iter()
                .filter(|m| *m != primary_range)
                .map(|m| Selection {
                    anchor: rope.offset_to_point_utf16(m.start),
                    head: rope.offset_to_point_utf16(m.end),
                }),
        );
        self.set_selections(&selections, cx);
        self.multi_cursor = None;
    }
}
