//! Find-based multi-cursor commands (Monaco's
//! `contrib/multicursor/browser/multicursor.ts`): add next occurrence
//! (Cmd/Ctrl+D), move last selection to next occurrence (Cmd/Ctrl+K
//! Cmd/Ctrl+D) and select all occurrences (Cmd/Ctrl+Shift+L).

use std::ops::Range;

use gpui::Context;

use crate::cursor::{self, Selection};
use crate::editor::CodeEditor;
use crate::search::{self, SearchParams};

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

    /// Match options for a session: from the find widget when it's open,
    /// whole-word and case-sensitive when started from a word.
    fn session_options(&self, from_word: bool) -> (bool, bool) {
        if from_word {
            (true, true)
        } else if self.find.visible {
            (self.find.options.whole_word, self.find.options.match_case)
        } else {
            (false, true)
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
        let params = if self.find.visible && !from_word {
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
