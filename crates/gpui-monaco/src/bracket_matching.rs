//! Bracket pair highlighting and jump-to-bracket (Monaco's
//! `contrib/bracketMatching`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use gpui::Context;
use text::{Point, PointUtf16};

use crate::cursor::{self, Selection};
use crate::decorations::{DecorationOptions, OverviewRulerDecoration, OverviewRulerLane};
use crate::editor::CodeEditor;
use crate::options::MatchBrackets;
use crate::theme::TokenStyle;
use crate::view::reveal::RevealType;

/// Bracket highlighting is skipped with more cursors than this (Monaco
/// `_recomputeBrackets`).
const MAX_CURSORS: usize = 100;

fn contains(range: &Range<PointUtf16>, p: PointUtf16) -> bool {
    range.start <= p && p <= range.end
}

/// Blocks longer than this aren't measured row by row (a minified file's
/// one long line).
const MAX_ROW_TABLE_BYTES: usize = 64 * 1024;

/// A block's strings and comments for [`CodeEditor::with_bracket_context`].
struct SkipBlock {
    ranges: Vec<Range<usize>>,
    rows: Option<RowTable>,
}

/// Where a block's rows start and end, and which are ASCII, so a bracket's
/// offset doesn't take a rope seek: measured in one pass over the block.
struct RowTable {
    first_row: u32,
    /// Each row's start and end (before its line break), and whether it's
    /// all ASCII.
    rows: Vec<(usize, usize, bool)>,
}

impl RowTable {
    fn new(rope: &text::Rope, first_row: u32, range: Range<usize>) -> Self {
        let mut rows = Vec::new();
        let (mut start, mut ascii) = (range.start, true);
        let mut offset = range.start;
        for chunk in rope.chunks_in_range(range.clone()) {
            for &byte in chunk.as_bytes() {
                if byte == b'\n' {
                    rows.push((start, offset, ascii));
                    (start, ascii) = (offset + 1, true);
                } else if !byte.is_ascii() {
                    ascii = false;
                }
                offset += 1;
            }
        }
        if start < range.end || rows.is_empty() {
            rows.push((start, range.end, ascii));
        }
        Self { first_row, rows }
    }

    /// The offset of `p` on an ASCII row, where columns are bytes, as
    /// [`text::Rope::point_utf16_to_offset`] gives it.
    fn ascii_offset(&self, p: PointUtf16) -> Option<usize> {
        let &(start, end, ascii) = self.rows.get(p.row.checked_sub(self.first_row)? as usize)?;
        ascii.then(|| (start + p.column as usize).min(end))
    }
}

impl CodeEditor {
    /// The bracket pair at (or, with `enclosing`, around) `position`.
    /// Runs `f` with the text, the language configuration and whether the
    /// bracket at a position is in a string or comment (so it's ignored).
    pub(crate) fn with_bracket_context<R>(
        &self,
        cx: &gpui::App,
        f: impl FnOnce(&text::Rope, &syntax::LanguageConfiguration, &dyn Fn(PointUtf16) -> bool) -> R,
    ) -> Option<R> {
        let model = self.model().read(cx);
        let language = model.language()?;
        let rope = self.snapshot.rope();
        // Strings and comments, queried a block of rows at a time: the search
        // can visit hundreds of brackets, and one query per bracket is slow.
        // The model keeps the blocks' tokens until the syntax changes.
        const BLOCK_ROWS: u32 = crate::model::SKIP_BLOCK_ROWS;
        let blocks: RefCell<HashMap<u32, SkipBlock>> = Default::default();
        let skippable = |p: PointUtf16| {
            let block = p.row / BLOCK_ROWS;
            let mut blocks = blocks.borrow_mut();
            let block = blocks.entry(block).or_insert_with(|| {
                let first_row = block * BLOCK_ROWS;
                let max_row = rope.max_point().row;
                let start = rope.point_to_offset(Point::new(first_row.min(max_row), 0));
                let end_row = (block + 1) * BLOCK_ROWS;
                let end = if end_row > max_row {
                    rope.len()
                } else {
                    rope.point_to_offset(Point::new(end_row, 0))
                };
                let ranges = model.bracket_skip_ranges(start..end);
                // Nothing to skip (e.g. plain text) needs no offsets.
                let rows = (!ranges.is_empty() && end - start <= MAX_ROW_TABLE_BYTES)
                    .then(|| RowTable::new(rope, first_row, start..end));
                SkipBlock { ranges, rows }
            });
            if block.ranges.is_empty() {
                return false;
            }
            let offset = block
                .rows
                .as_ref()
                .and_then(|rows| rows.ascii_offset(p))
                .unwrap_or_else(|| rope.point_utf16_to_offset(p));
            let ranges = &block.ranges;
            let ix = ranges.partition_point(|r| r.end <= offset);
            ranges.get(ix).is_some_and(|r| r.start <= offset)
        };
        Some(f(rope, &language.config, &skippable))
    }

    /// Monaco `matchBracket`, else `findEnclosingBrackets`.
    fn bracket_pair(
        &self,
        position: PointUtf16,
        cx: &gpui::App,
    ) -> Option<(Range<PointUtf16>, Range<PointUtf16>)> {
        self.with_bracket_context(cx, |rope, config, skippable| {
            cursor::matching_bracket(rope, config, position, skippable)
        })?
    }

    /// Deletes the bracket pair at or around each cursor (Monaco
    /// `editor.action.removeBrackets`, Cmd/Ctrl+Alt+Backspace). Each
    /// cursor's pair is its own undo step, with undo stops before and after
    /// (Monaco `removeBrackets` wraps each `executeEdits` in
    /// `pushUndoStop`); a pair shared by several cursors is removed once.
    pub fn remove_brackets(&mut self, cx: &mut Context<Self>) {
        // Each pair as byte ranges in the text before any removal.
        let mut pairs: Vec<[Range<usize>; 2]> = Vec::new();
        for selection in self.cursors.selections() {
            if let Some((open, close)) = self.bracket_pair(selection.head, cx) {
                let to_offsets = |r: Range<PointUtf16>| {
                    self.snapshot.point_utf16_to_offset(r.start)
                        ..self.snapshot.point_utf16_to_offset(r.end)
                };
                let pair = [to_offsets(open), to_offsets(close)];
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
        }
        // The ranges removed so far, to carry the later pairs through them.
        let mut removed: Vec<Range<usize>> = Vec::new();
        for pair in pairs {
            let shift = |offset: usize| -> usize {
                offset
                    - removed
                        .iter()
                        .filter(|r| r.end <= offset)
                        .map(|r| r.len())
                        .sum::<usize>()
            };
            let edits = pair
                .iter()
                .map(|r| {
                    let start = self.snapshot.offset_to_point_utf16(shift(r.start));
                    let end = self.snapshot.offset_to_point_utf16(shift(r.end));
                    (start..end, String::new())
                })
                .collect();
            let operation = cursor::EditOperation::from_edits(
                self.snapshot.rope(),
                &self.cursors.selections(),
                edits,
            );
            self.apply(operation, cx);
            removed.extend(pair);
        }
    }

    /// Re-highlights the bracket pairs at the cursors (Monaco
    /// `BracketMatchingController._recomputeBrackets`): the pair next to a
    /// cursor, with an overview ruler mark, or with `matchBrackets:
    /// "always"` the pair enclosing it. Only while the editor or one of its
    /// widgets (the find input, a focused hover) has focus.
    /// Matched brackets take `editorBracketMatch.foreground` when the theme
    /// sets it, over their pair colorization (Monaco's
    /// `bracket-match-inline`).
    pub(crate) fn update_bracket_highlights(&mut self, cx: &mut Context<Self>) {
        // (open, close, whether it's next to the cursor).
        let mut pairs: Vec<(Range<PointUtf16>, Range<PointUtf16>, bool)> = Vec::new();
        let selections = self.cursors.selections();
        let always = self.options.match_brackets == MatchBrackets::Always;
        if self.options.match_brackets != MatchBrackets::Never
            && self.widget_focused
            && !self.options.single_line
            && selections.len() <= MAX_CURSORS
        {
            for selection in selections.iter().filter(|s| s.is_empty()) {
                let position = selection.head;
                let found = self.with_bracket_context(cx, |rope, config, skippable| {
                    cursor::match_or_enclosing(rope, config, position, skippable, always)
                });
                if let Some(Some(pair)) = found {
                    pairs.push(pair);
                }
            }
        }
        let mut ranges: Vec<(Range<PointUtf16>, bool)> = pairs
            .into_iter()
            .flat_map(|(open, close, near)| [(open, near), (close, near)])
            .collect();
        ranges.sort_by_key(|(r, near)| (r.start, r.end, !near));
        ranges.dedup_by(|a, b| a.0 == b.0);
        let old = std::mem::take(&mut self.bracket_decorations);
        if old.is_empty() && ranges.is_empty() {
            return;
        }
        let colors = self.theme.colors.clone();
        let foreground = colors.bracket_match_foreground.map(|color| TokenStyle {
            foreground: Some(color),
            ..Default::default()
        });
        let rope = self.snapshot.rope();
        let new = ranges
            .into_iter()
            .map(|(range, near)| {
                (
                    rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end),
                    DecorationOptions {
                        background: Some(colors.bracket_match_background),
                        border: Some(colors.bracket_match_border),
                        text: foreground,
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        // Monaco marks only a pair next to the cursor.
                        overview_ruler: near.then(|| {
                            OverviewRulerDecoration::new(
                                colors.overview_ruler_bracket_match,
                                OverviewRulerLane::Center,
                            )
                        }),
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

    /// Moves each cursor to the matching bracket, to the end of the
    /// enclosing pair, or else to the next bracket (Monaco
    /// `editor.action.jumpToBracket`), and reveals the first.
    pub fn jump_to_bracket(&mut self, cx: &mut Context<Self>) {
        let selections: Vec<Selection> = self
            .cursors
            .selections()
            .into_iter()
            .map(|selection| {
                let position = selection.start();
                let target = self.with_bracket_context(cx, |rope, config, skippable| {
                    match cursor::match_or_enclosing(rope, config, position, skippable, true) {
                        Some((open, close, true)) => {
                            if contains(&open, position) && !contains(&close, position) {
                                Some(close.start)
                            } else if contains(&close, position) {
                                Some(open.start)
                            } else {
                                None
                            }
                        }
                        Some((_, close, false)) => Some(close.start),
                        None => cursor::next_bracket(rope, config, position, skippable)
                            .map(|next| next.start),
                    }
                });
                Selection::collapsed(target.flatten().unwrap_or(position))
            })
            .collect();
        self.set_selections(&selections, cx);
        // Monaco `revealRange(newSelections[0])`.
        self.reveal_range(
            selections[0].start()..selections[0].end(),
            RevealType::Simple,
            cx,
        );
    }

    /// Selects each cursor's bracket pair (Monaco
    /// `editor.action.selectToBracket`): the pair at the cursor, else around
    /// it, else of the next bracket, from the opening bracket to the
    /// closing one, or the other way when the cursor is at the closing
    /// bracket. With `select_brackets` the brackets are selected too,
    /// without it only what's between them. Cursors with no pair are
    /// dropped, and with none at all nothing changes. Reveals the first.
    pub fn select_to_bracket(&mut self, select_brackets: bool, cx: &mut Context<Self>) {
        let selections: Vec<Selection> = self
            .cursors
            .selections()
            .into_iter()
            .filter_map(|selection| {
                let position = selection.start();
                let (open, close) = self
                    .with_bracket_context(cx, |rope, config, skippable| {
                        cursor::matching_bracket(rope, config, position, skippable).or_else(|| {
                            let next = cursor::next_bracket(rope, config, position, skippable)?;
                            cursor::match_bracket(rope, config, next.start, skippable)
                        })
                    })
                    .flatten()?;
                let (from, to) = if select_brackets {
                    (open.start, close.end)
                } else {
                    (open.end, close.start)
                };
                // Backwards from a closing bracket.
                Some(if contains(&close, position) {
                    Selection::new(to, from)
                } else {
                    Selection::new(from, to)
                })
            })
            .collect();
        if selections.is_empty() {
            return;
        }
        self.set_selections(&selections, cx);
        // Monaco `revealRange(newSelections[0])`.
        self.reveal_range(
            selections[0].start()..selections[0].end(),
            RevealType::Simple,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use text::{Point, PointUtf16, Rope};

    use super::RowTable;

    /// A row table's offsets are the rope's, on every ASCII row of a
    /// block, and it has none for the others.
    #[test]
    fn row_table_offsets_match_the_rope() {
        for text in ["", "\n", "ab\n\ncd(e)\n", "a(\né[\nx\n\n{}", "(\n😀)\n[]"] {
            let rope = Rope::from(text);
            let max_row = rope.max_point().row;
            for (first, last) in [(0, max_row), (1.min(max_row), max_row), (0, 0)] {
                let start = rope.point_to_offset(Point::new(first, 0));
                let end = if last < max_row {
                    rope.point_to_offset(Point::new(last + 1, 0))
                } else {
                    rope.len()
                };
                let table = RowTable::new(&rope, first, start..end);
                for row in 0..=max_row {
                    let line = rope.line(row);
                    for column in 0..=line.encode_utf16().count() as u32 + 1 {
                        let p = PointUtf16::new(row, column);
                        let in_block = (first..=last).contains(&row);
                        match table.ascii_offset(p) {
                            Some(offset) => {
                                assert!(in_block && line.is_ascii(), "{text:?} {p:?}");
                                assert_eq!(offset, rope.point_utf16_to_offset(p), "{text:?} {p:?}");
                            }
                            // Only a block's final empty row may go without.
                            None => assert!(
                                !in_block || !line.is_ascii() || line.is_empty(),
                                "{text:?} {p:?}"
                            ),
                        }
                    }
                }
            }
        }
    }
}
