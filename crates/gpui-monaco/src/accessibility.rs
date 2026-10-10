//! Screen reader support (Monaco's screen reader content: `screenReaderUtils.ts`
//! `SimplePagedScreenReaderStrategy`, MIT). Monaco fills a hidden textarea
//! with the page of lines around the selection; here those lines become
//! AccessKit text runs under the editor's text input node.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::accesskit::{self, NodeId, Role};
use gpui::{A11ySubtreeBuilder, Context};
use text::{PointUtf16, Rope};

use crate::cursor::Selection;
use crate::editor::CodeEditor;

/// What a screen reader reads: whole lines of the selection's pages.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScreenReaderContent {
    /// The first line's row.
    pub start_row: u32,
    /// The lines, without line breaks.
    pub lines: Vec<String>,
    /// The selection, in rows and UTF-16 columns.
    pub selection: Selection,
}

/// Monaco `SimplePagedScreenReaderStrategy`: the pages (of `lines_per_page`
/// lines) holding the selection's start and end, as whole lines.
pub(crate) fn screen_reader_content(
    rope: &Rope,
    selection: Selection,
    lines_per_page: u32,
) -> ScreenReaderContent {
    let lines_per_page = lines_per_page.max(1);
    let page = |row: u32| row / lines_per_page;
    let rows: Range<u32> = page(selection.start().row) * lines_per_page
        ..((page(selection.end().row) + 1) * lines_per_page).min(rope.line_count());
    ScreenReaderContent {
        start_row: rows.start,
        lines: rows.clone().map(|row| rope.line(row)).collect(),
        selection,
    }
}

/// The character index of UTF-16 `column` in `line` (AccessKit counts
/// characters).
fn char_index(line: &str, column: u32) -> usize {
    let mut units = 0;
    for (ix, ch) in line.chars().enumerate() {
        if units >= column as usize {
            return ix;
        }
        units += ch.len_utf16();
    }
    line.chars().count()
}

/// The UTF-16 column of character `index` in `line`.
fn utf16_column(line: &str, index: usize) -> u32 {
    line.chars().take(index).map(|c| c.len_utf16() as u32).sum()
}

/// Text run node ids to rows, filled while building the tree, for actions
/// coming back from assistive technology.
pub(crate) type RunRows = Rc<RefCell<HashMap<NodeId, u32>>>;

impl ScreenReaderContent {
    /// Adds a text run per line, and the selection, to the editor's node.
    pub(crate) fn build(&self, builder: &mut A11ySubtreeBuilder, rows: &RunRows) {
        let mut ids = rows.borrow_mut();
        ids.clear();
        let mut run_ids = Vec::with_capacity(self.lines.len());
        for (ix, line) in self.lines.iter().enumerate() {
            let row = self.start_row + ix as u32;
            let id = builder.synthetic_node_id(row);
            // Each run ends with its line break, as in a textarea.
            let mut text = line.clone();
            if ix + 1 < self.lines.len() {
                text.push('\n');
            }
            let mut run = accesskit::Node::new(Role::TextRun);
            run.set_character_lengths(text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>());
            run.set_value(text);
            builder.push_child(id, run);
            ids.insert(id, row);
            run_ids.push(id);
        }
        let position = |point: PointUtf16| {
            let ix = (point.row.saturating_sub(self.start_row) as usize).min(run_ids.len() - 1);
            accesskit::TextPosition {
                node: run_ids[ix],
                character_index: char_index(&self.lines[ix], point.column),
            }
        };
        if !run_ids.is_empty() {
            builder
                .parent_node()
                .set_text_selection(accesskit::TextSelection {
                    anchor: position(self.selection.anchor),
                    focus: position(self.selection.head),
                });
        }
    }
}

/// The editor selection for an AccessKit text selection over the text runs
/// in `rows`.
fn selection_from_a11y(
    rope: &Rope,
    rows: &HashMap<NodeId, u32>,
    selection: &accesskit::TextSelection,
) -> Option<Selection> {
    let point = |position: &accesskit::TextPosition| {
        let row = *rows.get(&position.node)?;
        let line = rope.line(row);
        Some(PointUtf16::new(
            row,
            utf16_column(&line, position.character_index),
        ))
    };
    Some(Selection::new(
        point(&selection.anchor)?,
        point(&selection.focus)?,
    ))
}

impl CodeEditor {
    /// What the screen reader gets for the current selection.
    pub(crate) fn screen_reader_content(&self) -> ScreenReaderContent {
        screen_reader_content(
            self.snapshot.rope(),
            self.cursors.primary().selection(),
            self.options.accessibility_page_size,
        )
    }

    /// AccessKit's `SetTextSelection`: assistive technology moved the
    /// caret or selection.
    pub(crate) fn a11y_set_selection(
        &mut self,
        selection: &accesskit::TextSelection,
        rows: &RunRows,
        cx: &mut Context<Self>,
    ) {
        let found = selection_from_a11y(self.snapshot.rope(), &rows.borrow(), selection);
        if let Some(selection) = found {
            self.set_selections(&[selection], cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_hold_the_selection() {
        let text: String = (0..25).map(|i| format!("line {i}\n")).collect();
        let rope = Rope::from(text.as_str());
        let p = PointUtf16::new;
        // Pages of 10 lines: the cursor on row 12 reads rows 10..20.
        let content = screen_reader_content(&rope, Selection::collapsed(p(12, 3)), 10);
        assert_eq!(content.start_row, 10);
        assert_eq!(content.lines.len(), 10);
        assert_eq!(content.lines[0], "line 10");
        // A selection across pages reads both.
        let content = screen_reader_content(&rope, Selection::new(p(8, 0), p(21, 0)), 10);
        assert_eq!(content.start_row, 0);
        assert_eq!(content.lines.len(), 26);
    }

    #[test]
    fn selections_from_assistive_technology() {
        let rope = Rope::from("ab\nc😀d\n");
        let rows: HashMap<NodeId, u32> = [(NodeId(7), 0), (NodeId(8), 1)].into_iter().collect();
        let position = |node, character_index| accesskit::TextPosition {
            node: NodeId(node),
            character_index,
        };
        let selection = accesskit::TextSelection {
            anchor: position(7, 1),
            focus: position(8, 2),
        };
        assert_eq!(
            selection_from_a11y(&rope, &rows, &selection),
            Some(Selection::new(PointUtf16::new(0, 1), PointUtf16::new(1, 3)))
        );
        // Unknown nodes are ignored.
        let selection = accesskit::TextSelection {
            anchor: position(9, 0),
            focus: position(9, 0),
        };
        assert_eq!(selection_from_a11y(&rope, &rows, &selection), None);
    }

    #[test]
    fn columns_and_characters() {
        assert_eq!(char_index("a😀b", 3), 2);
        assert_eq!(utf16_column("a😀b", 2), 3);
        assert_eq!(char_index("ab", 9), 2);
    }
}
