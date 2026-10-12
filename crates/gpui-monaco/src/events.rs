//! Event details matching Monaco's `IModelContentChange`
//! (`textModelEvents.ts`), `CursorChangeReason` plus the cursor event
//! `source` (`cursorEvents.ts`), and the `ICodeEditor` event payloads of
//! `editorCommon.ts` and `editorBrowser.ts`.

use std::ops::Range;
use std::sync::Arc;

use gpui::{Modifiers, Pixels, Point, SharedString};
use text::{LineEnding, Patch, PointUtf16, Rope};

use crate::cursor::Selection;
use crate::options::EditorOptions;

/// One change in a [`TextModelEvent::Edited`](crate::TextModelEvent)
/// (Monaco `IModelContentChange`).
///
/// An event's changes are in descending order, in the coordinates of the
/// text before the event, so applying them one after another to that text
/// reproduces the new text.
///
/// There are two kinds of offsets. `range_offset` and `range_length` are
/// UTF-8 byte offsets into the stored text, whose line breaks are always
/// `\n` (what [`TextModel::offset_at`](crate::TextModel::offset_at) and
/// [`Buffer`](text::Buffer) use). `range_offset_utf16` and
/// `range_length_utf16` are Monaco's: UTF-16 code units, with each line
/// break counted as the model's line ending at the time (two for CRLF),
/// as [`TextModel::get_offset_at`](crate::TextModel::get_offset_at)
/// counts them. Code ported from Monaco wants the latter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentChange {
    /// The replaced range (0-based rows, UTF-16 columns).
    pub range: Range<PointUtf16>,
    /// The byte offset of the replaced range in the stored (`\n`) text.
    pub range_offset: usize,
    /// The byte length of the replaced range in the stored (`\n`) text.
    pub range_length: usize,
    /// The offset of the replaced range as Monaco counts it (Monaco
    /// `rangeOffset`): UTF-16 units, line breaks as the model's line ending.
    pub range_offset_utf16: usize,
    /// The length of the replaced range as Monaco counts it (Monaco
    /// `rangeLength`).
    pub range_length_utf16: usize,
    /// The new text for the range, with `\n` line breaks (Monaco's has the
    /// model's line ending).
    pub text: String,
}

/// Monaco's offset of byte `offset` of `rope`: UTF-16 units, each line
/// break counted as `eol`.
pub(crate) fn utf16_offset(rope: &Rope, offset: usize, eol: LineEnding) -> usize {
    let row = rope.offset_to_point(offset).row as usize;
    rope.offset_to_offset_utf16(offset).0 + row * (eol.as_str().len() - 1)
}

/// The [`ContentChange`]s of `patch`, which took `old` to `new` (e.g. from
/// [`Buffer::edits_since`](text::Buffer::edits_since)), in descending order.
/// `eol` is the line ending of `old`, which the UTF-16 offsets count.
pub fn content_changes(
    old: &Rope,
    new: &Rope,
    patch: &Patch,
    eol: LineEnding,
) -> Vec<ContentChange> {
    patch
        .edits()
        .iter()
        .rev()
        .map(|edit| {
            let start = utf16_offset(old, edit.old.start, eol);
            ContentChange {
                range: old.offset_to_point_utf16(edit.old.start)
                    ..old.offset_to_point_utf16(edit.old.end),
                range_offset: edit.old.start,
                range_length: edit.old.len(),
                range_offset_utf16: start,
                range_length_utf16: utf16_offset(old, edit.old.end, eol) - start,
                text: new.text_for_range(edit.new.clone()),
            }
        })
        .collect()
}

/// Monaco `setEOL`'s change: the whole of `rope` replaced with itself,
/// its length counted with the `old` line ending.
pub(crate) fn eol_change(rope: &Rope, old: LineEnding) -> ContentChange {
    ContentChange {
        range: PointUtf16::zero()..rope.max_point_utf16(),
        range_offset: 0,
        range_length: rope.len(),
        range_offset_utf16: 0,
        range_length_utf16: utf16_offset(rope, rope.len(), old),
        text: rope.to_string(),
    }
}

/// The details of a
/// [`CodeEditorEvent::ModelContentChanged`](crate::CodeEditorEvent)
/// (Monaco `IModelContentChangedEvent`), from the model's
/// [`TextModelEvent::Edited`](crate::TextModelEvent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelContentChange {
    /// The changes, in descending order (Monaco `changes`).
    pub changes: Arc<[ContentChange]>,
    /// The model's line ending (Monaco `eol`).
    pub eol: LineEnding,
    /// The model's version after the change (Monaco `versionId`).
    pub version_id: text::Version,
    /// An undo (Monaco `isUndoing`).
    pub is_undoing: bool,
    /// A redo (Monaco `isRedoing`).
    pub is_redoing: bool,
    /// The whole text was replaced by
    /// [`TextModel::set_value`](crate::TextModel::set_value) (Monaco
    /// `isFlush`).
    pub is_flush: bool,
    /// Only the line ending changed (Monaco `isEolChange`).
    pub is_eol_change: bool,
}

/// Which model options changed, in a
/// [`TextModelEvent::OptionsChanged`](crate::TextModelEvent) (Monaco
/// `IModelOptionsChangedEvent`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelOptionsChange {
    /// `tab_size` changed (Monaco `tabSize`).
    pub tab_size: bool,
    /// `indent_size` changed (Monaco `indentSize`).
    pub indent_size: bool,
    /// `insert_spaces` changed (Monaco `insertSpaces`).
    pub insert_spaces: bool,
    /// `trim_auto_whitespace` changed (Monaco `trimAutoWhitespace`).
    pub trim_auto_whitespace: bool,
}

/// What a [`TextModelEvent::DecorationsChanged`](crate::TextModelEvent)
/// affects, from the options of the decorations added, removed or changed
/// (Monaco `IModelDecorationsChangedEvent`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecorationsChange {
    /// The minimap: a decoration with `minimap` options (Monaco
    /// `affectsMinimap`).
    pub affects_minimap: bool,
    /// The overview ruler: a decoration with an `overview_ruler` mark or a
    /// marker `severity` (Monaco `affectsOverviewRuler`).
    pub affects_overview_ruler: bool,
    /// The glyph margin: a decoration with a `glyph` (Monaco
    /// `affectsGlyphMargin`).
    pub affects_glyph_margin: bool,
    /// The line numbers: a decoration with a `line_number_color` (Monaco
    /// `affectsLineNumber`).
    pub affects_line_number: bool,
}

impl DecorationsChange {
    /// This change, plus what a decoration with `options` affects.
    pub(crate) fn with(self, options: &crate::decorations::DecorationOptions) -> Self {
        Self {
            affects_minimap: self.affects_minimap || options.minimap.is_some(),
            affects_overview_ruler: self.affects_overview_ruler
                || options.overview_ruler.is_some()
                || options.severity.is_some(),
            affects_glyph_margin: self.affects_glyph_margin || options.glyph.is_some(),
            affects_line_number: self.affects_line_number || options.line_number_color.is_some(),
        }
    }
}

/// The details of a [`CodeEditorEvent::ScrollChanged`](crate::CodeEditorEvent)
/// (Monaco `IScrollEvent`). It's only reported when one of the four
/// changed, so syncing two editors' scroll positions both ways settles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollEvent {
    /// See [`CodeEditor::scroll_top`](crate::CodeEditor::scroll_top).
    pub scroll_top: Pixels,
    /// See [`CodeEditor::scroll_left`](crate::CodeEditor::scroll_left).
    pub scroll_left: Pixels,
    /// See [`CodeEditor::scroll_width`](crate::CodeEditor::scroll_width);
    /// zero before the first frame.
    pub scroll_width: Pixels,
    /// See [`CodeEditor::scroll_height`](crate::CodeEditor::scroll_height);
    /// zero before the first frame.
    pub scroll_height: Pixels,
    /// Whether `scroll_top` changed (Monaco `scrollTopChanged`).
    pub scroll_top_changed: bool,
    /// Whether `scroll_left` changed (Monaco `scrollLeftChanged`).
    pub scroll_left_changed: bool,
    /// Whether `scroll_width` changed (Monaco `scrollWidthChanged`).
    pub scroll_width_changed: bool,
    /// Whether `scroll_height` changed (Monaco `scrollHeightChanged`).
    pub scroll_height_changed: bool,
}

/// The details of a
/// [`CodeEditorEvent::CursorPositionChanged`](crate::CodeEditorEvent)
/// (Monaco `ICursorPositionChangedEvent`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorPositionChange {
    /// The primary cursor's position (Monaco `position`).
    pub position: PointUtf16,
    /// The other cursors' positions (Monaco `secondaryPositions`).
    pub secondary_positions: Vec<PointUtf16>,
    /// Why they changed.
    pub reason: CursorChangeReason,
    /// What changed them.
    pub source: CursorChangeSource,
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
/// (Monaco `ICursorSelectionChangedEvent`). It's only reported when the
/// selections changed, as Monaco compares the cursor states.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionChange {
    /// Why they changed.
    pub reason: CursorChangeReason,
    /// What changed them.
    pub source: CursorChangeSource,
    /// The primary selection (Monaco `selection`).
    pub selection: Selection,
    /// The other selections (Monaco `secondarySelections`).
    pub secondary_selections: Vec<Selection>,
    /// The selections last reported, `None` for the first report (Monaco
    /// `oldSelections`).
    pub old_selections: Option<Vec<Selection>>,
    /// The model's version (Monaco `modelVersionId`).
    pub model_version_id: text::Version,
    /// The model's version at the last report, 0 for the first (Monaco
    /// `oldModelVersionId`).
    pub old_model_version_id: text::Version,
}

/// Why and how the selections are about to change: what the next
/// [`SelectionChange`] reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CursorChangeCause {
    pub reason: CursorChangeReason,
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
/// (Monaco `ConfigurationChangedEvent`): the computed options before and
/// after, as [`CodeEditor::computed_options`](crate::CodeEditor::computed_options)
/// returns them.
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
    /// The selections are a column selection's (Monaco `dataToCopy.isBlock`),
    /// so pasting them at one cursor pastes a block.
    pub is_block: bool,
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
    /// The metadata says it was copied from a column selection (Monaco
    /// `metadata.isBlock`).
    pub is_block: bool,
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
