//! Event details matching Monaco's `IModelContentChange`
//! (`textModelEvents.ts`) and `CursorChangeReason` plus the cursor event
//! `source` (`cursorEvents.ts`).

use std::ops::Range;

use text::{Patch, PointUtf16, Rope};

/// One change in a [`TextModelEvent::Edited`](crate::TextModelEvent)
/// (Monaco `IModelContentChange`).
///
/// An event's changes are in descending order, in the coordinates of the
/// text before the event, so applying them one after another to that text
/// reproduces the new text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentChange {
    /// The replaced range (0-based rows, UTF-16 columns).
    pub range: Range<PointUtf16>,
    /// The byte offset of the replaced range (Monaco `rangeOffset`, which
    /// counts UTF-16 units).
    pub range_offset: usize,
    /// The byte length of the replaced range (Monaco `rangeLength`).
    pub range_length: usize,
    /// The new text for the range.
    pub text: String,
}

/// The [`ContentChange`]s of `patch`, which took `old` to `new` (e.g. from
/// [`Buffer::edits_since`](text::Buffer::edits_since)), in descending order.
pub fn content_changes(old: &Rope, new: &Rope, patch: &Patch) -> Vec<ContentChange> {
    patch
        .edits()
        .iter()
        .rev()
        .map(|edit| ContentChange {
            range: old.offset_to_point_utf16(edit.old.start)
                ..old.offset_to_point_utf16(edit.old.end),
            range_offset: edit.old.start,
            range_length: edit.old.len(),
            text: new.text_for_range(edit.new.clone()),
        })
        .collect()
}

/// Why the selections changed (Monaco `CursorChangeReason`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorChangeReason {
    /// Unknown or not set: edits, and selections set through the API.
    #[default]
    NotSet,
    /// The model's whole text was replaced (`set_value`) or the editor got
    /// a new model.
    ContentFlush,
    /// The text changed outside this editor and the cursors followed it.
    RecoverFromMarkers,
    /// An explicit gesture: a cursor move, a click, a drag.
    Explicit,
    /// A paste.
    Paste,
    /// An undo.
    Undo,
    /// A redo.
    Redo,
}

/// What triggered a selection change (Monaco's cursor event `source`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorChangeSource {
    /// A key binding or typing (`"keyboard"`).
    #[default]
    Keyboard,
    /// The mouse (`"mouse"`).
    Mouse,
    /// The host, through the editor's API (`"api"`).
    Api,
    /// A new model or `set_value` (`"model"`).
    Model,
    /// An edit to the model, undo and redo included (`"modelChange"`).
    ModelChange,
    /// [`CodeEditor::restore_view_state`](crate::CodeEditor::restore_view_state)
    /// (`"restoreState"`).
    RestoreState,
}

/// The details of a [`CodeEditorEvent::SelectionsChanged`](crate::CodeEditorEvent)
/// (Monaco `ICursorSelectionChangedEvent`; the selections themselves come
/// from [`CodeEditor::selections`](crate::CodeEditor::selections)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionChange {
    /// Why they changed.
    pub reason: CursorChangeReason,
    /// What changed them.
    pub source: CursorChangeSource,
}
