//! Screen reader support (Monaco's screen reader content: `screenReaderUtils.ts`
//! `SimplePagedScreenReaderStrategy`, MIT). Monaco fills a hidden textarea
//! with the page of lines around the selection; here those lines become
//! AccessKit text runs under the editor's text input node.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::accesskit::{self, NodeId, Role};
use gpui::{A11ySubtreeBuilder, Context, SharedString, Window};
use text::{PointUtf16, Rope};

use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::options::AccessibilitySupport;

/// Monaco `ariaLabelForScreenReaderContent` with `accessibilitySupport:
/// 'off'` (without a keybinding to turn screen reader mode on).
pub(crate) const NOT_ACCESSIBLE_LABEL: &str = "The editor is not accessible at this time.";

/// Screen reader mode (Monaco's computed `accessibilitySupport`) and
/// announcements (Monaco `aria.alert`).
#[derive(Default)]
pub(crate) struct ScreenReaderState {
    /// Monaco `isDominatedByLongLines`, read when the model is attached.
    pub dominated_by_long_lines: bool,
    /// Whether the last frame was optimized for a screen reader.
    pub optimized: bool,
    /// The last announcement, and whether it's in the second of Monaco's two
    /// alert regions (alternated so a repeated message is read again).
    pub announcement: Option<(SharedString, bool)>,
    /// What identifies the inline suggestion announced last.
    pub announced_suggestion: Option<String>,
}

/// Adds the announcement as an assertive live region (Monaco's
/// `role="alert"` container).
pub(crate) fn build_announcement(
    builder: &mut A11ySubtreeBuilder,
    message: &SharedString,
    second: bool,
) {
    let id = builder.synthetic_node_id(("announcement", second));
    let mut node = accesskit::Node::new(Role::Alert);
    node.set_live(accesskit::Live::Assertive);
    node.set_label(message.to_string());
    builder.push_child(id, node);
}

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
    /// Whether the editor runs optimized for screen readers (Monaco's
    /// computed `accessibilitySupport` is `Enabled`): `On`, or `Auto` while
    /// assistive technology is connected.
    pub(crate) fn screen_reader_optimized(&self, window: &Window) -> bool {
        match self.options.accessibility_support {
            AccessibilitySupport::On => true,
            AccessibilitySupport::Off => false,
            AccessibilitySupport::Auto => window.is_a11y_active(),
        }
    }

    /// Whether the last frame was optimized for screen readers (see
    /// [`EditorOptions::accessibility_support`](crate::EditorOptions::accessibility_support)).
    pub fn is_screen_reader_optimized(&self) -> bool {
        self.screen_reader.optimized
    }

    /// Has screen readers read `message` now (Monaco `aria.alert`): an
    /// assertive live region under the editor's node.
    pub fn announce(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.set_announcement(message.into());
        cx.notify();
    }

    fn set_announcement(&mut self, message: SharedString) {
        // Monaco uses the second region when the first already says it.
        let second = self
            .screen_reader
            .announcement
            .as_ref()
            .is_some_and(|(last, second)| *last == message && !second);
        self.screen_reader.announcement = Some((message, second));
    }

    /// The last announcement, for tests.
    #[doc(hidden)]
    pub fn last_announcement_for_tests(&self) -> Option<SharedString> {
        self.screen_reader
            .announcement
            .as_ref()
            .map(|(message, _)| message.clone())
    }

    /// The announcement for the accessibility tree.
    pub(crate) fn announcement(&self) -> Option<(SharedString, bool)> {
        self.screen_reader.announcement.clone()
    }

    /// Monaco `InlineCompletionsController`'s
    /// `playAccessibilitySignalAndReadSuggestion`, run each frame: once
    /// another inline suggestion shows (and the suggest widget is closed),
    /// screen readers read its ghost text, or where the next edit
    /// suggestion is, with `screen_reader_announce_inline_suggestion`.
    pub(crate) fn announce_inline_suggestion(&mut self, cx: &mut Context<Self>) {
        if !self.screen_reader.optimized || self.is_suggest_visible() {
            return;
        }
        let suggestion = if let Some((row, text)) = self.ghost_text_for_screen_reader(cx) {
            Some((format!("ghost {row} {text}"), text))
        } else if let Some((ix, edits)) = self.inline_edit_state(cx) {
            let line = edits.first().map_or(0, |(range, _)| range.start.row) + 1;
            let message = if self.tab_should_accept_inline_edit(cx) {
                format!("Next edit suggestion available on line {line}, press Tab to accept")
            } else if self.tab_should_jump_to_inline_edit(cx) {
                format!("Next edit suggestion available on line {line}, press Tab to jump")
            } else {
                format!("Next edit suggestion available on line {line}")
            };
            Some((format!("edit {ix} {edits:?}"), message))
        } else {
            None
        };
        let key = suggestion.as_ref().map(|(key, _)| key.clone());
        if key == self.screen_reader.announced_suggestion {
            return;
        }
        self.screen_reader.announced_suggestion = key;
        if let Some((_, message)) = suggestion
            && self.options.screen_reader_announce_inline_suggestion
            && !message.is_empty()
        {
            self.set_announcement(message.into());
        }
    }

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
