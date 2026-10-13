//! Dragging the selection to move or copy it (Monaco `DragAndDropController`
//! and `DragAndDropCommand`, `contrib/dnd`, MIT), and double-click block
//! selection (Monaco `doubleClickSelectsBlock` in `viewController.ts`).

use gpui::{Context, Modifiers};
use text::PointUtf16;

use crate::cursor::{self, Selection, StandardTokenType};
use crate::editor::CodeEditor;
use crate::mouse_target::MouseTargetKind;

/// Monaco `hasTriggerModifier`: Alt on macOS, Ctrl elsewhere, copies.
pub(crate) fn has_trigger_modifier(modifiers: &Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.alt
    } else {
        modifiers.control
    }
}

/// Monaco `Selection.containsPosition` (edges included).
fn contains(selection: &Selection, position: PointUtf16) -> bool {
    selection.start() <= position && position <= selection.end()
}

/// The drag in progress.
#[derive(Default)]
pub(crate) struct DragAndDrop {
    /// Monaco `isDragAndDrop`: the mouse went down on the selection.
    pub active: bool,
    /// The selection being dragged, once the mouse moved.
    selection: Option<Selection>,
    /// Where it would drop (Monaco's `dnd-target` decoration).
    pub target: Option<PointUtf16>,
    /// The copy modifier is held (Monaco `_modifierPressed`).
    modifier_pressed: bool,
    /// Monaco's `mouseStyle: copy`.
    pub copy: bool,
}

impl DragAndDrop {
    /// The selection is being dragged (the mouse moved since it went down
    /// on it).
    pub fn dragging(&self) -> bool {
        self.selection.is_some()
    }
}

/// Monaco `DragAndDropCommand`: the edits that move (or copy) `selection` to
/// `target`, in current coordinates, and the selection after them.
pub(crate) fn drag_and_drop_edits(
    text: &str,
    selection: Selection,
    target: PointUtf16,
    copy: bool,
) -> (Vec<(std::ops::Range<PointUtf16>, String)>, Selection) {
    let (start, end) = (selection.start(), selection.end());
    let mut edits = Vec::new();
    if !copy {
        edits.push((start..end, String::new()));
    }
    edits.push((target..target, text.to_string()));
    let single_line = start.row == end.row;
    let at_edge = target == end || target == start;
    if contains(&selection, target) && !(copy && at_edge) {
        return (edits, selection);
    }
    let p = PointUtf16::new;
    let target_selection = if copy {
        Selection::new(
            target,
            p(
                end.row - start.row + target.row,
                if single_line {
                    target.column + end.column - start.column
                } else {
                    end.column
                },
            ),
        )
    } else if target.row > end.row {
        // Dragged down.
        Selection::new(
            p(target.row - end.row + start.row, target.column),
            p(
                target.row,
                if single_line {
                    target.column + end.column - start.column
                } else {
                    end.column
                },
            ),
        )
    } else if target.row < end.row {
        // Dragged up.
        Selection::new(
            target,
            p(
                target.row + end.row - start.row,
                if single_line {
                    target.column + end.column - start.column
                } else {
                    end.column
                },
            ),
        )
    } else if end.column <= target.column {
        // After the selection's end, on its last line.
        Selection::new(
            p(
                target.row - end.row + start.row,
                target.column - end.column + start.column,
            ),
            p(
                target.row,
                if single_line {
                    target.column
                } else {
                    end.column
                },
            ),
        )
    } else {
        // Before a one-line selection, on its line.
        Selection::new(
            p(target.row - end.row + start.row, target.column),
            p(target.row, target.column + end.column - start.column),
        )
    };
    (edits, target_selection)
}

impl CodeEditor {
    /// Monaco `MouseDownOperation.start`: a single click on the selection's
    /// text starts dragging it instead of placing the cursor.
    pub(crate) fn dnd_mouse_down(
        &mut self,
        target: MouseTargetKind,
        position: PointUtf16,
        modifiers: &Modifiers,
        click_count: usize,
    ) -> bool {
        let selection = self.cursors.primary().selection();
        let start = !self.options.read_only
            && self.options.drag_and_drop
            && !self.options.column_selection
            && !modifiers.alt
            && click_count < 2
            && !selection.is_empty()
            && target == MouseTargetKind::ContentText
            && contains(&selection, position);
        self.dnd = DragAndDrop {
            active: start,
            modifier_pressed: self.dnd.modifier_pressed,
            ..Default::default()
        };
        start
    }

    /// Monaco `_onEditorMouseDrag`: the drop caret follows the mouse.
    pub(crate) fn dnd_mouse_drag(
        &mut self,
        position: Option<PointUtf16>,
        modifiers: &Modifiers,
        cx: &mut Context<Self>,
    ) {
        if self.dnd.selection.is_none() {
            let containing: Vec<Selection> = self
                .cursors
                .selections()
                .into_iter()
                .filter(|s| position.is_some_and(|p| contains(s, p)))
                .collect();
            match containing.as_slice() {
                [selection] => self.dnd.selection = Some(*selection),
                _ => return,
            }
        }
        self.dnd.copy = has_trigger_modifier(modifiers);
        if let (Some(position), Some(selection)) = (position, self.dnd.selection) {
            if contains(&selection, position) {
                self.dnd.target = None;
            } else {
                self.dnd.target = Some(position);
                self.reveal_position(position, cx);
            }
        }
        cx.notify();
    }

    /// Monaco `_onEditorMouseDrop`.
    pub(crate) fn dnd_mouse_drop(
        &mut self,
        target: Option<(MouseTargetKind, PointUtf16)>,
        modifiers: &Modifiers,
        cx: &mut Context<Self>,
    ) {
        let drag = std::mem::take(&mut self.dnd);
        self.dnd.modifier_pressed = drag.modifier_pressed;
        cx.notify();
        let Some((kind, position)) = target else {
            return;
        };
        let hit = matches!(
            kind,
            MouseTargetKind::ContentText
                | MouseTargetKind::ContentEmpty { .. }
                | MouseTargetKind::GutterGlyphMargin
                | MouseTargetKind::GutterLineNumbers
                | MouseTargetKind::GutterLineDecorations
        );
        if !hit {
            return;
        }
        match drag.selection {
            // A click without a drag: place the cursor there.
            None => {
                let selections = if modifiers.shift {
                    let anchor = self.cursors.primary().selection().anchor;
                    vec![Selection::new(anchor, position)]
                } else {
                    self.cursors
                        .selections()
                        .into_iter()
                        .map(|s| {
                            if contains(&s, position) {
                                Selection::collapsed(position)
                            } else {
                                s
                            }
                        })
                        .collect()
                };
                self.cursors.set_selections(&selections);
                self.next_selection_change = crate::events::CursorChangeCause {
                    reason: crate::events::CursorChangeReason::Explicit,
                    source: crate::events::CursorChangeSource::Mouse,
                };
                self.cursors_changed(true, cx);
            }
            Some(selection) => {
                let copy = has_trigger_modifier(modifiers) || drag.modifier_pressed;
                let at_edge = position == selection.end() || position == selection.start();
                if !contains(&selection, position) || (copy && at_edge) {
                    let rope = self.snapshot.rope();
                    let start = rope.point_utf16_to_offset(selection.start());
                    let end = rope.point_utf16_to_offset(selection.end());
                    let text = rope.slice(start..end).to_string();
                    let (edits, after) = drag_and_drop_edits(&text, selection, position, copy);
                    self.model()
                        .update(cx, |model, _| model.push_stack_element());
                    self.execute_edits_from(
                        "editor.contrib.dragAndDrop",
                        edits,
                        Some(vec![after]),
                        cx,
                    );
                    self.model()
                        .update(cx, |model, _| model.push_stack_element());
                }
            }
        }
    }

    /// Monaco `_onEditorMouseDropCanceled` (Escape while dragging).
    pub(crate) fn dnd_cancel(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.dnd.active {
            return false;
        }
        self.dnd = DragAndDrop {
            modifier_pressed: self.dnd.modifier_pressed,
            ..Default::default()
        };
        cx.notify();
        true
    }

    /// Monaco `onEditorBlur`: the drag ends and the copy modifier is
    /// forgotten, as its key-up may go elsewhere.
    pub(crate) fn dnd_blur(&mut self, cx: &mut Context<Self>) {
        let drag = std::mem::take(&mut self.dnd);
        if drag.active || drag.target.is_some() || drag.copy {
            cx.notify();
        }
    }

    /// Monaco `onEditorKeyDown`/`onEditorKeyUp`: the copy modifier.
    pub(crate) fn dnd_modifiers_changed(&mut self, modifiers: &Modifiers, cx: &mut Context<Self>) {
        if !self.options.drag_and_drop || self.options.column_selection {
            return;
        }
        self.dnd.modifier_pressed = has_trigger_modifier(modifiers);
        if self.dnd.selection.is_some() && self.dnd.copy != self.dnd.modifier_pressed {
            self.dnd.copy = self.dnd.modifier_pressed;
            cx.notify();
        }
    }

    /// Monaco `doubleClickSelectsBlock`: double-clicking just inside a
    /// bracket pair or a quoted string selects what's inside.
    pub(crate) fn block_selection_at(
        &self,
        position: PointUtf16,
        cx: &gpui::App,
    ) -> Option<Selection> {
        if !self.options.double_click_selects_block {
            return None;
        }
        self.bracket_content_at(position, cx)
            .or_else(|| self.string_content_at(position, cx))
    }

    /// Monaco `_trySelectBracketContent`.
    fn bracket_content_at(&self, position: PointUtf16, cx: &gpui::App) -> Option<Selection> {
        let pair = |p: PointUtf16| {
            self.with_bracket_context(cx, |rope, config, skippable| {
                cursor::matching_bracket(rope, config, p, skippable)
            })
            .flatten()
        };
        // Right after an opening bracket.
        if position.column > 0
            && let Some((open, close)) = pair(PointUtf16::new(position.row, position.column - 1))
            && open.end == position
        {
            return Some(Selection::new(open.end, close.start));
        }
        // Right before a closing bracket.
        if let Some((open, close)) = pair(position)
            && close.start == position
        {
            return Some(Selection::new(open.end, close.start));
        }
        None
    }

    /// Monaco `_trySelectStringContent`: the run of string tokens around
    /// the click, when it's right after the opening quote or right before
    /// the closing one.
    fn string_content_at(&self, position: PointUtf16, cx: &gpui::App) -> Option<Selection> {
        let model = self.model().read(cx);
        let rope = self.snapshot.rope();
        let line: Vec<u16> = rope.line(position.row).encode_utf16().collect();
        let column = position.column as usize;
        if column >= line.len() {
            return None;
        }
        let is_string = |column: usize| {
            let offset = rope.point_utf16_to_offset(PointUtf16::new(position.row, column as u32));
            model.token_type_at(offset) == StandardTokenType::String
        };
        if !is_string(column) {
            return None;
        }
        let mut start = column;
        while start > 0 && is_string(start - 1) {
            start -= 1;
        }
        let mut end = column + 1;
        while end < line.len() && is_string(end) {
            end += 1;
        }
        if column != start + 1 && column + 1 != end {
            return None;
        }
        let first = line[start];
        if !matches!(char::from_u32(first.into()), Some('"' | '\'' | '`')) || end - start < 2 {
            return None;
        }
        if line[end - 1] != first {
            return None;
        }
        // Monaco skips strings with right-to-left text.
        let content = String::from_utf16_lossy(&line[start + 1..end - 1]);
        if content.chars().any(is_rtl) {
            return None;
        }
        Some(Selection::new(
            PointUtf16::new(position.row, start as u32 + 1),
            PointUtf16::new(position.row, end as u32 - 1),
        ))
    }
}

/// Monaco `strings.containsRTL`: Hebrew, Arabic and the other right-to-left
/// scripts' blocks.
fn is_rtl(ch: char) -> bool {
    matches!(ch as u32,
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(row: u32, column: u32) -> PointUtf16 {
        PointUtf16::new(row, column)
    }

    fn target_selection(selection: Selection, target: PointUtf16, copy: bool) -> Selection {
        drag_and_drop_edits("", selection, target, copy).1
    }

    #[test]
    fn moving_one_line_selections() {
        let s = Selection::new(p(0, 2), p(0, 5));
        // Later on the line: the text moves left by its length.
        assert_eq!(
            target_selection(s, p(0, 8), false),
            Selection::new(p(0, 5), p(0, 8))
        );
        // Earlier on the line.
        assert_eq!(
            target_selection(s, p(0, 0), false),
            Selection::new(p(0, 0), p(0, 3))
        );
        // Down and up.
        assert_eq!(
            target_selection(s, p(2, 1), false),
            Selection::new(p(2, 1), p(2, 4))
        );
        let s = Selection::new(p(2, 2), p(2, 5));
        assert_eq!(
            target_selection(s, p(0, 1), false),
            Selection::new(p(0, 1), p(0, 4))
        );
    }

    #[test]
    fn copying_and_dropping_inside() {
        let s = Selection::new(p(0, 2), p(0, 5));
        assert_eq!(
            target_selection(s, p(1, 0), true),
            Selection::new(p(1, 0), p(1, 3))
        );
        // Copying beside the selection is allowed.
        let (edits, after) = drag_and_drop_edits("abc", s, p(0, 5), true);
        assert_eq!(edits, vec![(p(0, 5)..p(0, 5), "abc".to_string())]);
        assert_eq!(after, Selection::new(p(0, 5), p(0, 8)));
        // Inside it, nothing moves.
        assert_eq!(target_selection(s, p(0, 3), false), s);
    }

    #[test]
    fn moving_multi_line_selections() {
        let s = Selection::new(p(1, 2), p(2, 3));
        assert_eq!(
            target_selection(s, p(4, 1), false),
            Selection::new(p(3, 1), p(4, 3))
        );
        assert_eq!(
            target_selection(s, p(0, 1), false),
            Selection::new(p(0, 1), p(1, 3))
        );
        // After the end on its last line.
        assert_eq!(
            target_selection(s, p(2, 6), false),
            Selection::new(p(1, 5), p(2, 3))
        );
    }
}
