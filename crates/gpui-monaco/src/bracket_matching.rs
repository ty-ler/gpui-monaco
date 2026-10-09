//! Bracket pair highlighting and jump-to-bracket (Monaco's
//! `contrib/bracketMatching`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use gpui::Context;
use text::{Point, PointUtf16};

use crate::cursor::{self, Selection};
use crate::decorations::DecorationOptions;
use crate::editor::CodeEditor;

/// Bracket highlighting is skipped with more cursors than this.
const MAX_CURSORS: usize = 32;

fn contains(range: &Range<PointUtf16>, p: PointUtf16) -> bool {
    range.start <= p && p <= range.end
}

impl CodeEditor {
    /// The bracket pair at (or, with `enclosing`, around) `position`.
    fn bracket_pair(
        &self,
        position: PointUtf16,
        cx: &gpui::App,
    ) -> Option<(Range<PointUtf16>, Range<PointUtf16>)> {
        let model = self.model().read(cx);
        let language = model.language()?;
        let rope = self.snapshot.rope();
        // Strings and comments, queried a block of rows at a time: the search
        // can visit hundreds of brackets, and one query per bracket is slow.
        const BLOCK_ROWS: u32 = 64;
        let blocks: RefCell<HashMap<u32, Vec<Range<usize>>>> = Default::default();
        let skippable = |p: PointUtf16| {
            let offset = rope.point_utf16_to_offset(p);
            let block = p.row / BLOCK_ROWS;
            let mut blocks = blocks.borrow_mut();
            let ranges = blocks.entry(block).or_insert_with(|| {
                let max_row = rope.max_point().row;
                let start = rope.point_to_offset(Point::new((block * BLOCK_ROWS).min(max_row), 0));
                let end_row = (block + 1) * BLOCK_ROWS;
                let end = if end_row > max_row {
                    rope.len()
                } else {
                    rope.point_to_offset(Point::new(end_row, 0))
                };
                model.string_and_comment_ranges(start..end)
            });
            let ix = ranges.partition_point(|r| r.end <= offset);
            ranges.get(ix).is_some_and(|r| r.start <= offset)
        };
        cursor::matching_bracket(rope, &language.config, position, &skippable)
    }

    /// Re-highlights the bracket pairs next to the cursors (Monaco
    /// `matchBrackets: "always"`).
    pub(crate) fn update_bracket_highlights(&mut self, cx: &mut Context<Self>) {
        let mut ranges: Vec<Range<PointUtf16>> = Vec::new();
        let selections = self.cursors.selections();
        if self.options.match_brackets
            && !self.options.single_line
            && selections.len() <= MAX_CURSORS
        {
            for selection in selections.iter().filter(|s| s.is_empty()) {
                if let Some((open, close)) = self.bracket_pair(selection.head, cx)
                    && (contains(&open, selection.head) || contains(&close, selection.head))
                {
                    ranges.extend([open, close]);
                }
            }
        }
        ranges.sort_by_key(|r| (r.start, r.end));
        ranges.dedup();
        let old = std::mem::take(&mut self.bracket_decorations);
        if old.is_empty() && ranges.is_empty() {
            return;
        }
        let colors = self.theme.colors.clone();
        let rope = self.snapshot.rope();
        let new = ranges
            .into_iter()
            .map(|range| {
                (
                    rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end),
                    DecorationOptions {
                        background: Some(colors.bracket_match_background),
                        border: Some(colors.bracket_match_border),
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let owner = Some(cx.entity_id());
        self.bracket_decorations = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, new, cx)
        });
    }

    /// Moves each cursor to the matching bracket, or to the end of the
    /// enclosing pair (Monaco `editor.action.jumpToBracket`).
    pub fn jump_to_bracket(&mut self, cx: &mut Context<Self>) {
        let selections: Vec<Selection> = self
            .cursors
            .selections()
            .into_iter()
            .map(|selection| {
                let position = selection.start();
                let target = self.bracket_pair(position, cx).map(|(open, close)| {
                    if contains(&open, position) && !contains(&close, position) {
                        close.start
                    } else if contains(&close, position) {
                        open.start
                    } else {
                        close.start
                    }
                });
                Selection::collapsed(target.unwrap_or(position))
            })
            .collect();
        self.set_selections(&selections, cx);
    }

    /// Selects to the matching bracket, including the brackets (Monaco
    /// `editor.action.selectToBracket`).
    pub fn select_to_bracket(&mut self, cx: &mut Context<Self>) {
        let selections: Vec<Selection> = self
            .cursors
            .selections()
            .into_iter()
            .map(|selection| match self.bracket_pair(selection.start(), cx) {
                Some((open, close)) => Selection {
                    anchor: open.start,
                    head: close.end,
                },
                None => selection,
            })
            .collect();
        self.set_selections(&selections, cx);
    }
}
