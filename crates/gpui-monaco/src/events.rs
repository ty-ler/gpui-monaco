//! Event details matching Monaco's `IModelContentChange`
//! (`textModelEvents.ts`), `CursorChangeReason` plus the cursor event
//! `source` (`cursorEvents.ts`), and the `ICodeEditor` event payloads of
//! `editorCommon.ts` and `editorBrowser.ts`.

use std::ops::Range;
use std::sync::Arc;

use gpui::{Modifiers, Pixels, Point, SharedString};
use text::{Patch, PointUtf16, Rope};

use crate::options::EditorOptions;

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

/// The details of a [`CodeEditorEvent::WillChangeModel`](crate::CodeEditorEvent)
/// or [`CodeEditorEvent::ModelChanged`](crate::CodeEditorEvent) (Monaco
/// `IModelChangedEvent`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelChange {
    /// The previous model's uri (Monaco `oldModelUrl`).
    pub old_uri: Option<SharedString>,
    /// The new model's uri (Monaco `newModelUrl`).
    pub new_uri: Option<SharedString>,
}

/// The details of a [`CodeEditorEvent::ConfigurationChanged`](crate::CodeEditorEvent)
/// (Monaco `ConfigurationChangedEvent`): the options before and after, with
/// the font zoom applied (Monaco's computed options).
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigurationChange {
    old: Arc<EditorOptions>,
    new: Arc<EditorOptions>,
}

impl ConfigurationChange {
    pub(crate) fn new(old: EditorOptions, new: EditorOptions) -> Self {
        Self {
            old: Arc::new(old),
            new: Arc::new(new),
        }
    }

    /// Whether the option `option` picks changed (Monaco `hasChanged`),
    /// e.g. `change.has_changed(|o| &o.font_size)`.
    pub fn has_changed<T: PartialEq + ?Sized>(
        &self,
        option: impl Fn(&EditorOptions) -> &T,
    ) -> bool {
        option(&self.old) != option(&self.new)
    }

    /// The options before the change.
    pub fn old_options(&self) -> &EditorOptions {
        &self.old
    }

    /// The options after the change.
    pub fn new_options(&self) -> &EditorOptions {
        &self.new
    }
}

/// The details of a [`CodeEditorEvent::ContentSizeChanged`](crate::CodeEditorEvent)
/// (Monaco `IContentSizeChangedEvent`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContentSizeChange {
    /// See [`CodeEditor::content_width`](crate::CodeEditor::content_width).
    pub content_width: Pixels,
    /// See [`CodeEditor::content_height`](crate::CodeEditor::content_height).
    pub content_height: Pixels,
    /// Whether the width changed.
    pub content_width_changed: bool,
    /// Whether the height changed.
    pub content_height_changed: bool,
}

/// The details of a [`CodeEditorEvent::DidPaste`](crate::CodeEditorEvent)
/// (Monaco `IPasteEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteEvent {
    /// From the primary selection's start before the paste to its start
    /// after (0-based rows, UTF-16 columns).
    pub range: Range<PointUtf16>,
    /// The language of the editor the text was copied from, if any (Monaco
    /// `languageId`, the clipboard metadata's `mode`).
    pub language_id: Option<SharedString>,
}

/// The details of a [`CodeEditorEvent::WillCopy`](crate::CodeEditorEvent) or
/// [`CodeEditorEvent::WillCut`](crate::CodeEditorEvent) (Monaco
/// `IClipboardCopyEvent`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardCopyEvent {
    /// Whether it's a cut (Monaco `isCut`).
    pub is_cut: bool,
    /// The text going to the clipboard (Monaco `dataToCopy.text`).
    pub text: String,
    /// The selections were empty, so whole lines are copied.
    pub is_from_empty_selection: bool,
    /// With several cursors, each one's text.
    pub multicursor_text: Option<Vec<String>>,
    /// The model's language (Monaco `dataToCopy.mode`).
    pub language_id: Option<SharedString>,
}

/// The details of a [`CodeEditorEvent::WillPaste`](crate::CodeEditorEvent)
/// (Monaco `IClipboardPasteEvent`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardPasteEvent {
    /// The text being pasted.
    pub text: String,
    /// From the clipboard's editor metadata (Monaco `metadata`), when the
    /// text was copied from an editor: it was copied from empty selections.
    pub is_from_empty_selection: bool,
    /// The metadata's text per cursor.
    pub multicursor_text: Option<Vec<String>>,
    /// The metadata's language id (Monaco `metadata.mode`).
    pub language_id: Option<SharedString>,
}

/// The details of a [`CodeEditorEvent::MouseWheel`](crate::CodeEditorEvent)
/// (Monaco `IMouseWheelEvent`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MouseWheelEvent {
    /// Where the mouse is, in window coordinates.
    pub position: Point<Pixels>,
    /// How far the wheel moved, in pixels (lines count as the editor's line
    /// height), before the sensitivity options apply.
    pub delta: Point<Pixels>,
    /// The wheel reported whole lines (Monaco `deltaMode`
    /// `DOM_DELTA_LINE`), as a physical mouse wheel does; trackpads report
    /// pixels.
    pub lines: Option<Point<f32>>,
    /// The modifier keys held.
    pub modifiers: Modifiers,
}
