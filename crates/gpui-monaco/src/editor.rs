//! [`CodeEditor`]: the editor view entity.

use gpui::prelude::FluentBuilder as _;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, CursorStyle as MouseCursor, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, Render, ScrollWheelEvent, StatefulInteractiveElement as _, Styled, Subscription, Task,
    UTF16Selection, Window, div, point, px,
};
use serde::{Deserialize, Serialize};
use text::{Bias, BufferSnapshot, OffsetUtf16, PointUtf16};

use crate::actions::*;
use crate::cursor::{
    self, CursorCollection, CursorConfig, EditOperation, EditOperationType, MoveCommand, Selection,
    SingleCursorState,
};
use crate::element::{EditorElement, EditorLayout, layout_segment, row_segment};
use crate::events::{
    ClipboardCopyEvent, ClipboardPasteEvent, ConfigurationChange, ContentSizeChange,
    CursorChangeCause, CursorChangeReason, CursorChangeSource, CursorPositionChange,
    DecorationsChange, ModelChange, ModelContentChange, ModelOptionsChange, MouseWheelEvent,
    PasteEvent, ScrollEvent, SelectionChange,
};
use crate::model::{EditKind, TextModel, TextModelEvent, metadata};
use crate::options::EditorOptions;
use crate::options::LocationKind;
use crate::theme::{EditorColors, Theme};
use crate::view::folding::{
    FoldState, FoldingRangeData, compute_indent_ranges_limited, marker_ranges, sanitize_ranges,
};
use crate::view::view_lines::{ViewLines, WrapSettings, normalize_hidden};
use crate::widgets::{
    ContentWidget, ContentWidgetState, OverlayPosition, OverlayWidget, OverlayWidgetState,
    ViewZone, WidgetId, Widgets, ZoneHeight, ZoneState,
};

/// Events emitted by [`CodeEditor`].
#[derive(Clone, Debug, PartialEq)]
pub enum CodeEditorEvent {
    /// The cursors or selections changed, and why (Monaco
    /// `onDidChangeCursorSelection`). Only reported when they changed.
    SelectionsChanged(SelectionChange),
    /// The cursors or selections changed (Monaco
    /// `onDidChangeCursorPosition`), reported right before
    /// [`CodeEditorEvent::SelectionsChanged`], as Monaco does.
    CursorPositionChanged(CursorPositionChange),
    /// The scroll position or the scrollable size changed (Monaco
    /// `onDidScrollChange`). Only reported when something changed, so two
    /// editors can sync their scroll positions both ways.
    ScrollChanged(ScrollEvent),
    /// The model's text or line ending changed (Monaco's editor-level
    /// `onDidChangeModelContent`), forwarding
    /// [`TextModelEvent::Edited`] for whichever model the editor shows.
    ModelContentChanged(ModelContentChange),
    /// The model's decorations changed (Monaco's editor-level
    /// `onDidChangeModelDecorations`), forwarding
    /// [`TextModelEvent::DecorationsChanged`].
    ModelDecorationsChanged(DecorationsChange),
    /// The model's options changed (Monaco's editor-level
    /// `onDidChangeModelOptions`), forwarding
    /// [`TextModelEvent::OptionsChanged`].
    ModelOptionsChanged(ModelOptionsChange),
    /// New syntax highlighting is available for the model (Monaco's
    /// editor-level `onDidChangeModelTokens`, without the line ranges),
    /// forwarding [`TextModelEvent::SyntaxUpdated`].
    ModelTokensChanged,
    /// The model's language configuration changed (Monaco's editor-level
    /// `onDidChangeModelLanguageConfiguration`), forwarding
    /// [`TextModelEvent::LanguageConfigurationChanged`].
    ModelLanguageConfigurationChanged,
    /// An edit was refused because the editor is read-only (Monaco
    /// `onDidAttemptReadOnlyEdit`).
    ReadOnlyEditAttempt,
    /// The editor gained focus (Monaco `onDidFocusEditorText`).
    Focused,
    /// The editor lost focus (Monaco `onDidBlurEditorText`).
    Blurred,
    /// Go to definition led to another model: the host should open it.
    /// Not emitted when an opener from
    /// [`register_editor_opener`](crate::register_editor_opener) opened it.
    OpenLocation(crate::providers::Location),
    /// Monaco's "Open Definition to the Side" (`revealDefinitionAside`,
    /// or Cmd/Ctrl+Alt+click on a symbol) found this location, in any
    /// model: the host should open it in an editor beside this one. Not
    /// emitted when an editor opener opened it.
    OpenLocationToSide(crate::providers::Location),
    /// A link was followed (Cmd/Ctrl+click or `editor.action.openLink`):
    /// the host opens it. Relative `file://./` links are resolved against
    /// the model's URI. Not emitted when an opener from
    /// [`register_link_opener`](crate::register_link_opener) opened it.
    OpenLink(gpui::SharedString),
    /// A command no handler is registered for was run (e.g. by clicking a
    /// code lens): the host runs it.
    ExecuteCommand(crate::providers::Command),
    /// A mouse button went down over the editor (Monaco `onMouseDown`).
    MouseDown(crate::mouse_target::EditorMouseEvent),
    /// A mouse button went up (Monaco `onMouseUp`).
    MouseUp(crate::mouse_target::EditorMouseEvent),
    /// The mouse moved over the editor (Monaco `onMouseMove`).
    MouseMove(crate::mouse_target::EditorMouseEvent),
    /// The mouse left the editor (Monaco `onMouseLeave`).
    MouseLeave,
    /// The context menu was requested with a right click (Monaco
    /// `onContextMenu`). The editor shows its own unless the
    /// `context_menu` option is off.
    ContextMenu(crate::mouse_target::EditorMouseEvent),
    /// The editor is about to show another model (Monaco
    /// `onWillChangeModel`), from [`CodeEditor::set_model`].
    WillChangeModel(ModelChange),
    /// The editor shows another model (Monaco `onDidChangeModel`), from
    /// [`CodeEditor::set_model`].
    ModelChanged(ModelChange),
    /// The model's language changed (Monaco's editor-level
    /// `onDidChangeModelLanguage`), forwarding
    /// [`TextModelEvent::LanguageChanged`].
    ModelLanguageChanged {
        /// The previous language id (Monaco `oldLanguage`).
        old_language: Option<gpui::SharedString>,
        /// The new language id (Monaco `newLanguage`).
        new_language: Option<gpui::SharedString>,
    },
    /// The options changed (Monaco `onDidChangeConfiguration`), through
    /// [`CodeEditor::update_options`] or the font zoom.
    ConfigurationChanged(ConfigurationChange),
    /// The layout changed (Monaco `onDidLayoutChange`), reported after the
    /// frame that changed it, the first included.
    LayoutChanged(crate::layout_info::LayoutInfo),
    /// The content width or height changed (Monaco
    /// `onDidContentSizeChange`), reported after the frame that changed it.
    ContentSizeChanged(ContentSizeChange),
    /// The editor or a widget in it gained focus (Monaco
    /// `onDidFocusEditorWidget`), e.g. the find widget's input.
    WidgetFocused,
    /// Focus left the editor and its widgets (Monaco
    /// `onDidBlurEditorWidget`).
    WidgetBlurred,
    /// Text is about to be typed (Monaco `onWillType`), from the keyboard
    /// or [`CodeEditor::type_text`].
    WillType(gpui::SharedString),
    /// Text was typed (Monaco `onDidType`).
    DidType(gpui::SharedString),
    /// The clipboard's text is about to be pasted (Monaco `onWillPaste`).
    WillPaste(ClipboardPasteEvent),
    /// The editor pasted (Monaco `onDidPaste`). Not emitted when a paste
    /// edit provider handled the paste.
    DidPaste(PasteEvent),
    /// The selection is about to be copied (Monaco `onWillCopy`).
    WillCopy(ClipboardCopyEvent),
    /// The selection is about to be cut (Monaco `onWillCut`).
    WillCut(ClipboardCopyEvent),
    /// A key went down while the editor had focus, before key bindings
    /// handle it (Monaco `onKeyDown`).
    KeyDown(gpui::Keystroke),
    /// A key went up while the editor had focus (Monaco `onKeyUp`).
    KeyUp(gpui::Keystroke),
    /// The mouse wheel turned over the editor (Monaco `onMouseWheel`),
    /// before the editor scrolls or zooms.
    MouseWheel(MouseWheelEvent),
    /// The selected text is being dragged (Monaco `onMouseDrag`), with the
    /// `drag_and_drop` option.
    MouseDrag(crate::mouse_target::EditorMouseEvent),
    /// Dragged text was dropped (Monaco `onMouseDrop`).
    MouseDrop(crate::mouse_target::EditorMouseEvent),
    /// Dragging text was cancelled with Escape (Monaco
    /// `onMouseDropCanceled`).
    MouseDropCanceled,
    /// An IME composition started (Monaco `onDidCompositionStart`): the
    /// input method marked text. See [`CodeEditor::in_composition`].
    CompositionStarted,
    /// The IME composition ended (Monaco `onDidCompositionEnd`): it was
    /// committed, cancelled or unmarked.
    CompositionEnded,
    /// The hidden lines changed (Monaco `onDidChangeHiddenAreas`): folding
    /// or [`CodeEditor::set_hidden_areas`], reported when the next frame
    /// applies them.
    HiddenAreasChanged,
    /// View zones were added, removed or resized (Monaco
    /// `onDidChangeViewZones`), the editor's own (code lenses, peek views)
    /// included.
    ViewZonesChanged,
    /// The editor started an operation that may change the model or the
    /// view several times (Monaco `onBeginUpdate`): an edit, typing,
    /// pasting, undo or redo, setting selections or the model. Nested
    /// operations report once; [`CodeEditorEvent::EndUpdate`] follows. Not
    /// every operation is bracketed, as in Monaco.
    BeginUpdate,
    /// The operation [`CodeEditorEvent::BeginUpdate`] reported finished
    /// (Monaco `onEndUpdate`).
    EndUpdate,
}

/// Monaco's folding update scheduler waits at least this long.
const FOLD_UPDATE_DELAY: Duration = Duration::from_millis(200);

/// Monaco `collectSyntaxRanges`: the valid ranges from every folding range
/// provider, ranked by registration order, or `None` when none answered.
pub(crate) async fn collect_provided_folding_ranges(
    providers: Vec<Task<anyhow::Result<Option<Vec<crate::providers::FoldingRange>>>>>,
    line_count: u32,
) -> Option<Vec<FoldingRangeData>> {
    let mut provided: Option<Vec<FoldingRangeData>> = None;
    for (rank, task) in providers.into_iter().enumerate() {
        match task.await {
            Ok(Some(ranges)) => provided.get_or_insert_default().extend(
                ranges
                    .into_iter()
                    .filter(|r| r.start_row < r.end_row && r.end_row < line_count)
                    .map(|r| FoldingRangeData {
                        start: r.start_row,
                        end: r.end_row,
                        rank,
                        kind: r.kind,
                        manual: false,
                    }),
            ),
            Ok(None) => {}
            Err(err) => log::warn!("folding ranges failed: {err:#}"),
        }
    }
    provided
}

/// Saved per-model view state (Monaco `ICodeEditorViewState`, from
/// `saveViewState`).
///
/// Besides the selections and the scroll position it keeps the first
/// visible position, which [`CodeEditor::restore_view_state`] scrolls back
/// to even when wrapping or view zones moved it, and the folded regions
/// (Monaco's folding contribution state).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    /// The selections; the first is the primary one.
    pub selections: Vec<Selection>,
    /// Vertical scroll offset in pixels.
    pub scroll_top: Pixels,
    /// Horizontal scroll offset in pixels.
    pub scroll_left: Pixels,
    /// The model position at the top of the first visible line (Monaco
    /// `firstPosition`), once the editor was laid out.
    pub first_position: Option<PointUtf16>,
    /// The first visible line's top relative to the viewport's (Monaco
    /// `firstPositionDeltaTop`, 0 or less).
    pub first_position_delta_top: Pixels,
    /// The folding state (Monaco `FoldingStateMemento`).
    pub folding: FoldingViewState,
}

/// Saved folding state (Monaco `FoldingStateMemento`): the collapsed and
/// manual regions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldingViewState {
    /// The collapsed regions and manual ranges (Monaco `collapsedRegions`).
    pub regions: Vec<FoldingRegionState>,
    /// The model's line count when saved (Monaco `lineCount`).
    pub line_count: u32,
}

/// One saved folding region (Monaco `ILineMemento`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldingRegionState {
    /// Its header row, 0-based.
    pub start_row: u32,
    /// Its last row, 0-based.
    pub end_row: u32,
    /// Whether it's collapsed.
    pub is_collapsed: bool,
    /// A manual range (Monaco `FoldSource.userDefined`).
    pub manual: bool,
    /// A hash of its second and last lines (Monaco `checksum`): if they
    /// changed, the region isn't restored.
    pub checksum: Option<i32>,
}

/// An API reveal, done in the next prepaint (Monaco
/// `ViewRevealRangeRequestEvent`).
#[derive(Clone, Debug)]
pub(crate) struct PendingReveal {
    /// One range, or every cursor's selection.
    pub ranges: Vec<Range<PointUtf16>>,
    /// The ranges are the cursors (Monaco's `selections`): no scrolling
    /// when they don't fit.
    pub cursors: bool,
    pub reveal: crate::view::reveal::RevealType,
    /// Also scroll horizontally (Monaco `revealHorizontal`).
    pub horizontal: bool,
    /// Monaco `minimalReveal`.
    pub minimal: bool,
    pub scroll_type: crate::view::reveal::ScrollType,
}

/// A horizontal reveal, done once the target lines are laid out (Monaco
/// `HorizontalRevealRequest`).
#[derive(Clone, Debug)]
pub(crate) struct HorizontalReveal {
    /// Single-line ranges.
    pub ranges: Vec<Range<PointUtf16>>,
    pub cursors: bool,
    pub minimal: bool,
}

/// Clipboard metadata so pastes can reproduce Monaco's empty-selection and
/// multi-cursor behavior.
#[derive(Serialize, Deserialize)]
pub(crate) struct ClipboardMetadata {
    is_from_empty_selection: bool,
    multicursor_text: Option<Vec<String>>,
    /// The language id of the editor it was copied from (Monaco `mode`).
    #[serde(default)]
    mode: Option<String>,
    /// The copy whose `prepareDocumentPaste` data a paste merges in
    /// (Monaco's copy metadata `id`).
    #[serde(default)]
    pub(crate) id: Option<String>,
    /// The MIME types that data has (Monaco `providerCopyMimeTypes`).
    #[serde(default)]
    pub(crate) provider_copy_mime_types: Vec<String>,
}

/// Where the mouse is outside the text while drag-selecting (Monaco
/// `outsidePosition`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragOutside {
    Above,
    Below,
    Left,
    Right,
}

impl DragOutside {
    fn vertical(self) -> bool {
        matches!(self, Self::Above | Self::Below)
    }
}

/// How often drag scrolling runs (Monaco: every animation frame).
const DRAG_SCROLL_FRAME: Duration = Duration::from_millis(16);

/// Scrolling while drag-selecting outside the text.
struct DragScroll {
    outside: DragOutside,
    distance: Pixels,
    mouse: Point<Pixels>,
    /// The line under the mouse, for left and right.
    row: u32,
    /// When the last frame scrolled (Monaco `DragScrollingOperation._tick`).
    last_time: web_time::Instant,
    _task: Task<()>,
}

impl DragScroll {
    fn vertical(&self) -> bool {
        self.outside.vertical()
    }
}

/// Monaco `MouseDownState`'s gesture state while a mouse button is down
/// (`MouseDownOperation`): what the mouse down started, which every drag
/// step dispatches again (Monaco `_dispatchMouse` / `dispatchMouse`).
struct MouseSelection {
    /// The click count (Monaco `MouseDownState.count`).
    count: usize,
    /// The gesture started on the line numbers (Monaco
    /// `startedOnLineNumbers`): it selects whole lines.
    started_on_line_numbers: bool,
    /// The middle button started it (Monaco `middleButton`): it
    /// column-selects.
    middle_button: bool,
    /// The modifiers, updated as the mouse moves (Monaco `setModifiers`).
    modifiers: gpui::Modifiers,
    /// The mouse went down on injected text (an inlay hint), where a
    /// double click selects nothing (Monaco `onInjectedText`).
    on_injected_text: bool,
}

/// Monaco `MouseDownState`'s click counting: the platform's click count,
/// reset unless the clicks land on the same position within 400ms.
#[derive(Default)]
struct MouseDownCount {
    last_position: Option<PointUtf16>,
    last_position_equal_count: usize,
    last_count: usize,
    last_time: Option<web_time::Instant>,
}

impl MouseDownCount {
    /// Monaco `MouseDownState.CLEAR_MOUSE_DOWN_COUNT_TIME`.
    const CLEAR_TIME: Duration = Duration::from_millis(400);

    /// Monaco `MouseDownState.trySetCount`: the count of a mouse down at
    /// `position` the platform counted `count`.
    fn try_set_count(
        &mut self,
        mut count: usize,
        position: PointUtf16,
        now: web_time::Instant,
    ) -> usize {
        // a. Too much time has passed.
        if self
            .last_time
            .is_none_or(|last| now.saturating_duration_since(last) > Self::CLEAR_TIME)
        {
            count = 1;
        }
        self.last_time = Some(now);
        // b. No jump from a single to a triple click in one go.
        count = count.min(self.last_count + 1);
        // c. The clicks are at different positions.
        if self.last_position == Some(position) {
            self.last_position_equal_count += 1;
        } else {
            self.last_position_equal_count = 1;
        }
        self.last_position = Some(position);
        self.last_count = count.min(self.last_position_equal_count);
        self.last_count
    }
}

/// Monaco `CoreNavigationCommands.CreateCursor`: adds `state` as a cursor;
/// with several cursors, a click in one of them removes it instead (a
/// toggle).
fn create_cursor(
    mut states: Vec<SingleCursorState>,
    state: SingleCursorState,
) -> Vec<SingleCursorState> {
    if states.len() > 1
        && let Some(ix) = states.iter().position(|s| {
            let selection = s.selection();
            selection.start() <= state.position && state.position <= selection.end()
        })
    {
        states.remove(ix);
        return states;
    }
    states.push(state);
    states
}

#[derive(Clone, Copy)]
pub(crate) enum ScrollbarDrag {
    Vertical { grab_offset: Pixels },
    Horizontal { grab_offset: Pixels },
}

/// Undo groups: consecutive edits of the same group merge into one undo step.
fn undo_group(kind: EditOperationType) -> Option<u64> {
    match kind {
        EditOperationType::TypingOther
        | EditOperationType::TypingFirstSpace
        | EditOperationType::TypingConsecutiveSpace => Some(1),
        EditOperationType::DeletingLeft => Some(2),
        EditOperationType::DeletingRight => Some(3),
        EditOperationType::Other => None,
    }
}

/// A Monaco-like code editor view over a [`TextModel`].
pub struct CodeEditor {
    model: Entity<TextModel>,
    /// `model`, for decorations collections, which may need it while the
    /// editor is being updated.
    pub(crate) shared_model: crate::api::SharedModel,
    pub(crate) snapshot: BufferSnapshot,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) cursors: CursorCollection,
    /// The cursors' view states from the last view-based move or click
    /// (Monaco `CursorState.viewState`); see [`Self::cursor_view_states`].
    cursor_views: Option<CursorViewStates>,
    /// The options with the font zoom applied; what rendering reads.
    pub(crate) options: EditorOptions,
    /// The options as the host set them.
    user_options: EditorOptions,
    pub(crate) zoom_gesture: Option<crate::font_zoom::ZoomGesture>,
    /// Check for unusual line terminators before the next frame.
    pub(crate) unusual_line_terminators_pending: bool,
    pub(crate) unicode_highlights: crate::unicode_highlighter::UnicodeHighlightState,
    pub(crate) dnd: crate::dnd::DragAndDrop,
    pub(crate) drop_paste: crate::drop_paste::DropPasteState,
    /// Screen reader text run ids to rows.
    a11y_rows: crate::accessibility::RunRows,
    /// Screen reader mode and announcements.
    pub(crate) screen_reader: crate::accessibility::ScreenReaderState,
    /// Monaco `inertialScroll`'s fling after wheel events, while it runs.
    inertial_scroll_task: Option<Task<()>>,
    pub(crate) theme: Arc<Theme>,
    pub(crate) scroll_top: Pixels,
    pub(crate) scroll_left: Pixels,
    pub(crate) layout: Option<EditorLayout>,
    #[doc(hidden)]
    /// Shared with in-flight background work (the minimap) and the frame
    /// being painted, so changes go through `Arc::make_mut`.
    pub view_lines: Arc<ViewLines>,
    pub(crate) folds: FoldState,
    pub(crate) gutter_hovered: bool,
    /// The row of the folding control under the mouse, for its tooltip.
    fold_control_hovered: Option<u32>,
    pub(crate) widgets: Widgets,
    pub(crate) find: crate::find::FindState,
    pub(crate) multi_cursor: Option<crate::multi_cursor::MultiCursorSession>,
    pub(crate) bracket_decorations: Vec<crate::decorations::DecorationId>,
    pub(crate) navigation: crate::navigation_history::NavigationHistory,
    pub(crate) occurrences: crate::occurrences::OccurrenceHighlights,
    pub(crate) bracket_colors: crate::bracket_colors::BracketColors,
    /// A reveal requested by the API, done in the next prepaint.
    pub(crate) pending_reveal: Option<PendingReveal>,
    /// What the next prepaint reveals horizontally (after a reveal).
    pub(crate) pending_reveal_x: Option<HorizontalReveal>,
    /// Where the scroll position anchors after
    /// [`CodeEditor::restore_view_state`]: the first visible position and
    /// its top relative to the viewport (Monaco `reduceRestoreState`).
    pub(crate) pending_scroll_anchor: Option<(PointUtf16, Pixels)>,
    /// The layout info last reported with `LayoutChanged`.
    pub(crate) last_layout_info: Option<crate::layout_info::LayoutInfo>,
    /// The content width and height last reported with
    /// `ContentSizeChanged`.
    pub(crate) last_content_size: Option<(Pixels, Pixels)>,
    /// Monaco's `overtype` input mode (Insert key).
    pub(crate) overtype: bool,
    pub(crate) hover: crate::hover::HoverState,
    pub(crate) marker_navigation: crate::markers::MarkerNavigation,
    pub(crate) definition: crate::definition::DefinitionLink,
    pub(crate) signature_help: crate::signature_help::SignatureHelpState,
    pub(crate) suggest: crate::completion::SuggestState,
    pub(crate) snippet: crate::snippet_session::SnippetState,
    pub(crate) links: crate::links::LinksState,
    pub(crate) semantic: crate::semantic_tokens::SemanticTokensState,
    pub(crate) inline: crate::inline_completions::controller::InlineCompletionsState,
    pub(crate) linked: crate::linked_editing::LinkedEditingState,
    pub(crate) code_lens: crate::code_lens::CodeLensState,
    pub(crate) quick: Option<crate::quick_access::QuickAccessState>,
    /// Actions from [`CodeEditor::add_action`].
    pub(crate) sticky: crate::sticky_model::StickyModelState,
    pub(crate) colors: crate::color_detector::ColorDetectorState,
    pub(crate) section_headers: crate::section_headers::SectionHeaderState,
    pub(crate) color_picker: crate::color_picker::ColorPickerState,
    pub(crate) context_menu: crate::context_menu::ContextMenuState,
    pub(crate) added_actions: Vec<crate::editor_actions::EditorActionDescriptor>,
    /// Context keys, the host's commands and their keybindings.
    pub(crate) commands: crate::editor_commands::EditorCommandsState,
    pub(crate) minimap_cache: crate::minimap::MinimapCache,
    /// The overview ruler's decoration marks.
    pub(crate) overview_ruler_cache: crate::view::overview_ruler::OverviewRulerCache,
    /// The editor font's measured character widths, for `Advanced`
    /// wrapping.
    pub(crate) char_widths: Option<Arc<crate::element::CharWidths>>,
    /// The wrap classifier for the break character options it was built
    /// from (Monaco recreates its line breaks computer when they change).
    wrap_classifier: Option<(
        gpui::SharedString,
        gpui::SharedString,
        Arc<crate::view::line_breaks::WrapClassifier>,
    )>,
    pub(crate) minimap_hovered: bool,
    /// The pointer is over the minimap's slider (`minimapSlider.hoverBackground`).
    pub(crate) minimap_slider_hovered: bool,
    pub(crate) minimap_drag: Option<crate::minimap::MinimapDrag>,
    fold_task: Option<Task<()>>,
    /// The editor has focus (Monaco `hasWidgetFocus`, for bracket matching).
    pub(crate) has_focus: bool,
    /// Rows hidden by [`CodeEditor::set_hidden_areas`], besides folds, by
    /// source (Monaco keeps them per source).
    hidden_areas: Vec<(Option<gpui::SharedString>, Vec<Range<u32>>)>,
    /// The version `fold_task` was started for.
    fold_requested: Option<text::Version>,
    /// The `refresh_folding_ranges` count the ranges were computed at.
    fold_epoch: u64,
    /// A background fold computation is running (dropping `fold_task`
    /// wouldn't stop it), and whether a newer version waits for it.
    fold_computing: bool,
    fold_rerun: bool,
    pub(crate) autoscroll_requested: bool,
    /// The autoscroll comes from a mouse gesture (Monaco ignores
    /// `cursorSurroundingLines` for those by default).
    pub(crate) autoscroll_from_mouse: bool,
    /// The autoscroll is a mouse gesture's `NavigationCommandRevealType.Minimal`
    /// (Monaco `revealAllCursors(_, revealHorizontal, true)`): no extra line
    /// of room.
    pub(crate) autoscroll_minimal: bool,
    /// The autoscroll is only vertical (Monaco's line select reveals with
    /// `revealHorizontal: false`).
    pub(crate) autoscroll_vertical_only: bool,
    /// Animations of the view (smooth scrolling, the caret, blinking).
    pub(crate) animation: crate::view::animation::ViewAnimation,
    pub(crate) cursor_visible: bool,
    /// IME composition range, in byte offsets.
    pub(crate) marked_range: Option<Range<usize>>,
    /// The part of the composition the IME selected (byte offsets), drawn
    /// as its active clause (Monaco `edit-context-composition-primary`).
    pub(crate) marked_selection: Option<Range<usize>>,
    /// Nesting of [`CodeEditor::begin_update`] (Monaco `_updateCounter`).
    update_depth: usize,
    pub(crate) prev_edit_kind: Option<EditOperationType>,
    /// The undo group the next edit uses instead of its kind's (Monaco
    /// `executeEdits` joins the open step).
    pub(crate) edit_group: Option<u64>,
    /// The reason and source the next `SelectionsChanged` reports.
    pub(crate) next_selection_change: CursorChangeCause,
    /// The selections and model version last reported (Monaco's cursor
    /// state, which events are only fired for when it changes).
    last_cursor_state: Option<(Vec<Selection>, text::Version)>,
    /// The scroll state last reported with `ScrollChanged`: top, left,
    /// width, height.
    last_scroll_state: Option<[Pixels; 4]>,
    /// The inline message, if one is showing.
    pub(crate) message: crate::message::InlineMessage,
    /// Navigation commands in flight and the jump highlight.
    pub(crate) goto: crate::goto::GotoState,
    /// The peek view, if open.
    pub(crate) peek: crate::peek::PeekState,
    /// The inspect tokens widget, if launched.
    pub(crate) inspect_tokens: crate::developer::InspectTokensState,
    /// Requests Escape can cancel (Monaco `editor.cancelOperation`).
    pub(crate) cancellable: crate::editor_state::CancellableOperations,
    /// A banner above the text and its height (Monaco `setBanner`).
    pub(crate) banner: Option<(gpui::AnyView, Pixels)>,
    /// The rename box and requests.
    pub(crate) rename: crate::rename::RenameState,
    /// The lightbulb, the code action menu and requests.
    pub(crate) code_actions: crate::code_action::CodeActionState,
    /// Inlay hints and their requests.
    pub(crate) inlay: crate::inlay_hints::InlayHintState,
    /// Expand and shrink selection state.
    pub(crate) smart_select: crate::smart_select::SmartSelectState,
    /// This editor is a peek view's preview: F4 and unhandled Escape go to
    /// the editor around it.
    pub(crate) in_peek: bool,
    /// This editor is an input in one of the editor's widgets (see
    /// [`CodeEditor::new_widget_input`]).
    pub(crate) widget_input: bool,
    /// Take focus on the next render (after closing a peek view).
    pub(crate) refocus: bool,
    /// A running format document, selection or paste request.
    pub(crate) format_task: Option<Task<()>>,
    /// A running format on type request.
    pub(crate) format_on_type_task: Option<Task<()>>,
    /// The primary cursor line's top relative to the viewport in the last
    /// frame.
    pub(crate) last_cursor_offset: Option<Pixels>,
    /// Keep the primary cursor line at this height in the viewport next
    /// frame (Monaco `StableEditorScrollState`).
    pub(crate) pending_cursor_offset: Option<Pixels>,
    mouse_selection: Option<MouseSelection>,
    mouse_down_count: MouseDownCount,
    /// Scrolling while drag-selecting outside the text.
    drag_scroll: Option<DragScroll>,
    pub(crate) scrollbar_drag: Option<ScrollbarDrag>,
    /// A held scrollbar arrow's repeat.
    scrollbar_arrow_task: Option<Task<()>>,
    blink_epoch: usize,
    _blink_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CodeEditorEvent> for CodeEditor {}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CodeEditor {
    /// Creates an editor showing `model`.
    pub fn new(model: Entity<TextModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(model, false, window, cx)
    }

    /// A one-line input inside one of the editor's widgets (find, replace,
    /// rename, quick input), with its own internal model. Monaco's are
    /// plain `<input>`s, so it isn't in [`crate::editors`] or
    /// [`crate::models`] and is announced to no one; it has
    /// [`widget_input_options`], the input background (whatever the theme)
    /// and no font zoom.
    pub(crate) fn new_widget_input(
        text: &str,
        theme: Arc<Theme>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = cx.new(|_| TextModel::new_internal(text));
        let mut editor = Self::build(model, true, window, cx);
        editor.widget_input = true;
        editor.set_theme(theme, cx);
        editor.update_options(widget_input_options(), cx);
        editor
    }

    /// The font zoom level this editor follows: none for widget inputs
    /// (Monaco's inputs are styled by CSS, not the editor's zoom).
    pub(crate) fn zoom_level(&self, cx: &App) -> f32 {
        if self.widget_input {
            0.
        } else {
            crate::font_zoom::editor_zoom_level(cx)
        }
    }

    fn build(
        model: Entity<TextModel>,
        internal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::model::register_model(cx, &model);
        crate::registry::track_editor(internal, cx);
        let dominated_by_long_lines = model.read(cx).is_dominated_by_long_lines();
        // A tab stop, like Monaco's textarea, for tab focus mode and hosts.
        let focus_handle = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            cx.subscribe(&model, Self::on_model_event),
            cx.on_focus(&focus_handle, window, |this, _, cx| {
                this.has_focus = true;
                this.update_bracket_highlights(cx);
                this.restart_blink(cx);
                cx.emit(CodeEditorEvent::Focused);
                cx.notify();
            }),
            // Monaco `onDidBlurEditorText`: the text lost focus, maybe to
            // one of the editor's widgets (the hover, the find input).
            cx.on_blur(&focus_handle, window, |this, _, cx| {
                this.has_focus = false;
                this.update_bracket_highlights(cx);
                this.mouse_selection = None;
                this.drag_scroll = None;
                this.dnd_cancel(cx);
                this.close_message(cx);
                cx.emit(CodeEditorEvent::Blurred);
                cx.notify();
            }),
            // Monaco's widget focus: the text or a widget inside the editor.
            cx.on_focus_in(&focus_handle, window, |this, _, cx| {
                this.find_widget_focus_in(cx);
                cx.emit(CodeEditorEvent::WidgetFocused);
            }),
            // Monaco `onDidBlurEditorWidget`: focus left the editor and its
            // widgets. Suggest (`SuggestController`) and parameter hints
            // (`ParameterHintsModel`) cancel, and so do inline suggestions;
            // the hover goes too (focusing it, Cmd/Ctrl+K Cmd/Ctrl+I, keeps
            // it).
            cx.on_focus_out(&focus_handle, window, |this, _, _, cx| {
                this.hide_hover(cx);
                this.hide_suggest(cx);
                this.hide_signature_help(cx);
                this.inline_blurred(cx);
                cx.emit(CodeEditorEvent::WidgetBlurred);
                cx.notify();
            }),
            // Monaco `onKeyDown` fires before key bindings run, so it's an
            // interceptor rather than a key down listener (which bound keys
            // never reach).
            {
                let editor = cx.weak_entity();
                let focus = focus_handle.clone();
                cx.intercept_keystrokes(move |event, window, cx| {
                    if focus.is_focused(window) {
                        let keystroke = event.keystroke.clone();
                        editor
                            .update(cx, |this, cx| {
                                this.dismiss_keyboard_definition_link(cx);
                                // Monaco `InspectTokensController`: Escape
                                // closes it, whatever else it does.
                                if keystroke.key == "escape" {
                                    this.stop_inspect_tokens(cx);
                                }
                                cx.emit(CodeEditorEvent::KeyDown(keystroke))
                            })
                            .ok();
                    }
                })
            },
        ];
        let mut subscriptions = subscriptions;
        // Providers coming and going (e.g. a language server starting) and
        // refresh requests are noticed when painting.
        subscriptions.extend(crate::providers::observe_language_features(cx));
        subscriptions.extend(crate::editor_commands::subscriptions(cx));
        subscriptions.push(
            cx.observe_global::<crate::font_zoom::EditorZoom>(|this, cx| this.on_zoom_changed(cx)),
        );
        // Monaco `onBeforeDetached` when the editor goes away.
        subscriptions.push(cx.on_release(|this: &mut Self, cx| {
            this.model.update(cx, |model, cx| model.detach_editor(cx));
        }));
        model.update(cx, |model, cx| {
            model.attach_editor(cx);
            model.ensure_syntax(cx);
        });
        let snapshot = model.read(cx).snapshot();
        Self {
            shared_model: std::rc::Rc::new(std::cell::RefCell::new(model.downgrade())),
            model,
            snapshot: snapshot.clone(),
            focus_handle,
            cursors: CursorCollection::new(SingleCursorState::from_position(PointUtf16::zero())),
            cursor_views: None,
            options: crate::font_zoom::zoomed_options(
                &EditorOptions::default(),
                crate::font_zoom::editor_zoom_level(cx),
            ),
            user_options: EditorOptions::default(),
            zoom_gesture: None,
            unusual_line_terminators_pending: true,
            unicode_highlights: Default::default(),
            dnd: Default::default(),
            drop_paste: Default::default(),
            a11y_rows: Default::default(),
            screen_reader: crate::accessibility::ScreenReaderState {
                dominated_by_long_lines,
                ..Default::default()
            },
            inertial_scroll_task: None,
            theme: crate::theme::global_theme(cx).unwrap_or_else(|| Arc::new(Theme::vs_dark())),
            scroll_top: px(0.),
            scroll_left: px(0.),
            layout: None,
            autoscroll_requested: false,
            autoscroll_from_mouse: false,
            autoscroll_minimal: false,
            autoscroll_vertical_only: false,
            animation: Default::default(),
            cursor_visible: true,
            marked_range: None,
            marked_selection: None,
            update_depth: 0,
            prev_edit_kind: None,
            edit_group: None,
            next_selection_change: CursorChangeCause::default(),
            last_cursor_state: None,
            last_scroll_state: None,
            message: Default::default(),
            goto: Default::default(),
            peek: Default::default(),
            inspect_tokens: Default::default(),
            cancellable: Default::default(),
            banner: None,
            rename: Default::default(),
            code_actions: Default::default(),
            inlay: Default::default(),
            smart_select: Default::default(),
            in_peek: false,
            widget_input: false,
            refocus: false,
            format_task: None,
            format_on_type_task: None,
            last_cursor_offset: None,
            pending_cursor_offset: None,
            mouse_selection: None,
            mouse_down_count: MouseDownCount::default(),
            drag_scroll: None,
            scrollbar_drag: None,
            scrollbar_arrow_task: None,
            blink_epoch: 0,
            view_lines: Arc::new(ViewLines::new(snapshot.rope().clone(), snapshot.version())),
            folds: FoldState::default(),
            gutter_hovered: false,
            fold_control_hovered: None,
            widgets: Widgets::default(),
            find: crate::find::FindState::new(cx),
            multi_cursor: None,
            bracket_decorations: Vec::new(),
            overtype: false,
            occurrences: Default::default(),
            bracket_colors: Default::default(),
            pending_reveal: None,
            pending_reveal_x: None,
            pending_scroll_anchor: None,
            last_layout_info: None,
            last_content_size: None,
            navigation: crate::navigation_history::NavigationHistory::starting_at(
                vec![Selection::collapsed(PointUtf16::zero())],
                snapshot.version(),
            ),
            hover: Default::default(),
            marker_navigation: Default::default(),
            definition: Default::default(),
            signature_help: Default::default(),
            suggest: Default::default(),
            snippet: Default::default(),
            links: Default::default(),
            semantic: Default::default(),
            inline: Default::default(),
            linked: Default::default(),
            code_lens: Default::default(),
            quick: None,
            added_actions: Vec::new(),
            commands: Default::default(),
            sticky: Default::default(),
            colors: Default::default(),
            section_headers: Default::default(),
            color_picker: Default::default(),
            context_menu: Default::default(),
            minimap_cache: Default::default(),
            overview_ruler_cache: Default::default(),
            char_widths: None,
            wrap_classifier: None,
            minimap_hovered: false,
            minimap_slider_hovered: false,
            minimap_drag: None,
            fold_task: None,
            has_focus: false,
            fold_requested: None,
            fold_epoch: 0,
            hidden_areas: Vec::new(),
            fold_computing: false,
            fold_rerun: false,
            _blink_task: None,
            _subscriptions: subscriptions,
        }
    }

    /// Creates an editor with a new model holding `text`.
    pub fn with_text(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = cx.new(|_| TextModel::new(text));
        Self::new(model, window, cx)
    }

    /// The model this editor shows.
    pub fn model(&self) -> &Entity<TextModel> {
        &self.model
    }

    /// Shows a different model, resetting cursors and scroll (Monaco
    /// `setModel`). Save and restore [`ViewState`] to keep them per model.
    pub fn set_model(&mut self, model: Entity<TextModel>, cx: &mut Context<Self>) {
        // Monaco does nothing for the same model.
        self.begin_update(cx);
        if model != self.model {
            self.set_model_inner(model, cx);
        }
        self.end_update(cx);
    }

    fn set_model_inner(&mut self, model: Entity<TextModel>, cx: &mut Context<Self>) {
        let change = ModelChange {
            old_uri: self.model.read(cx).uri().cloned(),
            new_uri: model.read(cx).uri().cloned(),
        };
        cx.emit(CodeEditorEvent::WillChangeModel(change.clone()));
        crate::model::register_model(cx, &model);
        self.request_unusual_line_terminators_check();
        // Popups and decorations this editor owns refer to the old model: its
        // decoration ids would name unrelated decorations in the new one.
        self.hide_hover(cx);
        self.hide_suggest(cx);
        self.cancel_snippet(false, cx);
        self.clear_links(cx);
        self.clear_linked_editing(cx);
        self.reset_semantic_tokens(cx);
        self.semantic = Default::default();
        self.hide_signature_help(cx);
        self.close_message(cx);
        self.close_peek(cx);
        self.cancel_rename(false, cx);
        self.stop_inspect_tokens(cx);
        self.hide_code_action_menu(cx);
        self.code_actions.lightbulb = None;
        self.reset_inlay_hints(cx);
        self.close_marker_zone(cx);
        self.clear_definition_link(cx);
        self.detach_find_from_model(cx);
        let brackets = std::mem::take(&mut self.bracket_decorations);
        if !brackets.is_empty() {
            self.model.update(cx, |model, cx| {
                model.delta_decorations(None, &brackets, Vec::new(), cx)
            });
        }
        // Monaco `_postDetachModelCleanup`: the decorations this editor owns
        // go with it.
        let owner = cx.entity_id();
        self.model.update(cx, |model, cx| {
            model.remove_all_decorations_with_owner(owner, cx)
        });
        self.minimap_cache = Default::default();
        self.overview_ruler_cache = Default::default();
        self.cancel_selection_anchor(cx);
        self.reset_navigation_history();
        self.clear_occurrence_highlights(cx);
        self.bracket_colors = Default::default();

        self._subscriptions[0] = cx.subscribe(&model, Self::on_model_event);
        // Monaco `_detachModel` and `_attachModel`.
        self.model.update(cx, |model, cx| model.detach_editor(cx));
        model.update(cx, |model, cx| {
            model.attach_editor(cx);
            model.ensure_syntax(cx);
        });
        // Monaco `setIsDominatedByLongLines` when a model is attached.
        self.screen_reader.dominated_by_long_lines = model.read(cx).is_dominated_by_long_lines();
        self.snapshot = model.read(cx).snapshot();
        self.view_lines = Arc::new(ViewLines::new(
            self.snapshot.rope().clone(),
            self.snapshot.version(),
        ));
        self.folds = FoldState::default();
        self.fold_task = None;
        self.fold_requested = None;
        self.hidden_areas.clear();
        self.fold_computing = false;
        self.fold_rerun = false;
        *self.shared_model.borrow_mut() = model.downgrade();
        self.model = model;
        self.cursors = CursorCollection::new(SingleCursorState::from_position(PointUtf16::zero()));
        self.cursors
            .set_merge_overlapping(self.options.multi_cursor_merge_overlapping);
        self.scroll_top = px(0.);
        self.scroll_left = px(0.);
        self.pending_reveal = None;
        self.pending_reveal_x = None;
        self.pending_scroll_anchor = None;
        self.set_marked_range(None, cx);
        self.prev_edit_kind = None;
        self.record_cursor_change();
        self.reattach_find_model(cx);
        // A new cursor for the new model: reported whatever it was before.
        self.last_cursor_state = None;
        self.emit_cursor_events(
            CursorChangeCause {
                reason: CursorChangeReason::ContentFlush,
                source: CursorChangeSource::Model,
            },
            cx,
        );
        self.emit_scroll_changed(cx);
        cx.emit(CodeEditorEvent::ModelChanged(change));
        cx.notify();
    }

    /// Captures the selections, the scroll position and the folded regions
    /// (Monaco `saveViewState`).
    pub fn save_view_state(&self, cx: &App) -> ViewState {
        // Monaco `ViewModel.saveState`: the first visible line and how far
        // above the viewport its top is.
        let (first_position, first_position_delta_top) = match self.layout.as_ref() {
            Some(layout) => {
                let line_height = layout.metrics.line_height;
                let count = self.view_lines.line_count().max(1);
                let line = layout.zones.line_at(self.scroll_top, line_height, count);
                let position = self.view_lines.to_model(line, 0);
                let top = layout.zones.line_y(line, line_height);
                (Some(position), top - self.scroll_top)
            }
            None => (None, px(0.)),
        };
        ViewState {
            selections: self.cursors.selections(),
            scroll_top: self.scroll_top,
            scroll_left: self.scroll_left,
            first_position,
            first_position_delta_top,
            folding: self.save_folding_state(cx),
        }
    }

    /// Restores a [`ViewState`] from [`CodeEditor::save_view_state`] (Monaco
    /// `restoreViewState`): the selections, clamped to the current text,
    /// the folded regions whose lines didn't change, and the scroll
    /// position, anchored on the first visible line once laid out. Emits
    /// [`CodeEditorEvent::SelectionsChanged`].
    pub fn restore_view_state(&mut self, state: &ViewState, cx: &mut Context<Self>) {
        if !state.selections.is_empty() {
            self.cursors.set_selections(&state.selections);
            self.cursors.validate(self.snapshot.rope());
        }
        self.restore_folding_state(&state.folding, cx);
        self.animation.cancel_scroll();
        self.scroll_top = state.scroll_top;
        self.scroll_left = state.scroll_left;
        self.pending_scroll_anchor = state
            .first_position
            .map(|position| (position, state.first_position_delta_top));
        self.pending_reveal = None;
        self.pending_reveal_x = None;
        self.autoscroll_requested = false;
        self.emit_cursor_events(
            CursorChangeCause {
                reason: CursorChangeReason::NotSet,
                source: CursorChangeSource::RestoreState,
            },
            cx,
        );
        self.emit_scroll_changed(cx);
        cx.notify();
    }

    /// The editor options, as set (Monaco `getRawOptions`); the editor uses
    /// them [validated](EditorOptions::validated) and zoomed.
    #[expect(
        clippy::misnamed_getters,
        reason = "`options` holds the validated, zoomed values; hosts get theirs back"
    )]
    pub fn options(&self) -> &EditorOptions {
        &self.user_options
    }

    /// Replaces the editor options (Monaco `updateOptions`, but with a full set).
    ///
    /// The editor uses them [validated](EditorOptions::validated) as
    /// Monaco does; [`options`](Self::options) returns them as given.
    pub fn update_options(&mut self, options: EditorOptions, cx: &mut Context<Self>) {
        let old_from_all_providers = self.options.occurrences_highlight_from_all_providers;
        let zoomed = crate::font_zoom::zoomed_options(&options, self.zoom_level(cx));
        let old = std::mem::replace(&mut self.options, zoomed);
        // Monaco `tabIndex`.
        if old.tab_index != self.options.tab_index {
            let tab_index = self.options.tab_index;
            self.focus_handle = self
                .focus_handle
                .clone()
                .tab_index(tab_index as isize)
                .tab_stop(tab_index >= 0);
        }
        // Monaco `onDidChangeConfiguration` fires when an option changed.
        let change = (old != self.options).then(|| {
            CodeEditorEvent::ConfigurationChanged(ConfigurationChange::new(
                old,
                self.options.clone(),
            ))
        });
        if options.unusual_line_terminators != self.user_options.unusual_line_terminators {
            self.request_unusual_line_terminators_check();
        }
        self.user_options = options;
        self.cursors
            .set_merge_overlapping(self.options.multi_cursor_merge_overlapping);
        self.bracket_colors = Default::default();
        self.refresh_bracket_colors(cx);
        self.update_bracket_highlights(cx);
        self.reset_inlay_hints(cx);
        // Monaco `ParameterHintsModel.onEditorConfigurationChange`.
        if !self.options.parameter_hints.enabled {
            self.hide_signature_help(cx);
        }
        // Monaco `WordHighlighter`: changing how providers are asked stops
        // the highlights and asks again.
        if old_from_all_providers != self.options.occurrences_highlight_from_all_providers {
            self.restart_word_highlights(cx);
        }
        if !self.options.inertial_scroll {
            self.stop_inertial_scroll();
        }
        if let Some(change) = change {
            cx.emit(change);
        }
        cx.notify();
    }

    /// The current theme's chrome colors.
    pub fn colors(&self) -> &EditorColors {
        &self.theme.colors
    }

    /// The current theme. Defaults to the one set with
    /// [`crate::set_theme`], else [`Theme::vs_dark`].
    pub fn theme(&self) -> &Arc<Theme> {
        &self.theme
    }

    /// Sets the theme (Monaco `editor.setTheme`).
    pub fn set_theme(&mut self, theme: Arc<Theme>, cx: &mut Context<Self>) {
        // Monaco's inputs paint `input.background`, not the editor's.
        self.theme = if self.widget_input {
            let mut theme = (*theme).clone();
            theme.colors.background = theme.colors.input_background;
            theme.colors.gutter_background = theme.colors.input_background;
            theme.colors.placeholder_foreground = theme.colors.input_placeholder_foreground;
            Arc::new(theme)
        } else {
            theme
        };
        // The bracket highlights carry the theme's colors.
        self.update_bracket_highlights(cx);
        cx.notify();
    }

    /// All selections; the first is the primary one.
    pub fn selections(&self) -> Vec<Selection> {
        self.cursors.selections()
    }

    /// Replaces the selections (the first becomes primary), clamped to the text, and
    /// emits [`CodeEditorEvent::SelectionsChanged`]. An empty slice is ignored.
    pub fn set_selections(&mut self, selections: &[Selection], cx: &mut Context<Self>) {
        if selections.is_empty() {
            return;
        }
        self.begin_update(cx);
        // The model may have changed since our last event (e.g. `set_value`
        // just now); the new selections refer to its current text.
        let latest = self.model.read(cx).snapshot();
        if latest.version() != self.snapshot.version() {
            self.snapshot = latest;
        }
        self.cursors.set_selections(selections);
        self.cursors.validate(self.snapshot.rope());
        self.next_selection_change.source = CursorChangeSource::Api;
        self.cursors_changed(true, cx);
        self.end_update(cx);
    }

    /// Monaco `_beginUpdate`: the outermost call emits
    /// [`CodeEditorEvent::BeginUpdate`].
    pub(crate) fn begin_update(&mut self, cx: &mut Context<Self>) {
        self.update_depth += 1;
        if self.update_depth == 1 {
            cx.emit(CodeEditorEvent::BeginUpdate);
        }
    }

    /// Monaco `_endUpdate`: the outermost call emits
    /// [`CodeEditorEvent::EndUpdate`].
    pub(crate) fn end_update(&mut self, cx: &mut Context<Self>) {
        self.update_depth = self.update_depth.saturating_sub(1);
        if self.update_depth == 0 {
            cx.emit(CodeEditorEvent::EndUpdate);
        }
    }

    /// Whether an IME composition is in progress (Monaco `inComposition`).
    pub fn in_composition(&self) -> bool {
        self.marked_range.is_some()
    }

    /// Sets the IME composition range, emitting
    /// [`CodeEditorEvent::CompositionStarted`] or
    /// [`CodeEditorEvent::CompositionEnded`] when one starts or ends
    /// (Monaco `_startComposition`/`_endComposition`).
    pub(crate) fn set_marked_range(&mut self, range: Option<Range<usize>>, cx: &mut Context<Self>) {
        let was = self.marked_range.is_some();
        let is = range.is_some();
        self.marked_range = range;
        if !was && is {
            cx.emit(CodeEditorEvent::CompositionStarted);
        } else if was && !is {
            cx.emit(CodeEditorEvent::CompositionEnded);
        }
    }

    /// The scroll offset in pixels (`x` is the left, `y` the top).
    pub fn scroll_position(&self) -> Point<Pixels> {
        point(self.scroll_left, self.scroll_top)
    }

    /// Scrolls to `position` in pixels right away (Monaco
    /// `setScrollPosition` with its default `ScrollType.Immediate`),
    /// clamped to the scroll range, and emits
    /// [`CodeEditorEvent::ScrollChanged`].
    pub fn set_scroll_position(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.set_scroll_position_with_scroll_type(
            position,
            crate::view::reveal::ScrollType::Immediate,
            cx,
        );
    }

    /// [`CodeEditor::set_scroll_position`], animating with
    /// [`ScrollType::Smooth`](crate::ScrollType) when `smooth_scrolling` is
    /// on (Monaco `setScrollPosition(position, scrollType)`).
    pub fn set_scroll_position_with_scroll_type(
        &mut self,
        position: Point<Pixels>,
        scroll_type: crate::view::reveal::ScrollType,
        cx: &mut Context<Self>,
    ) {
        self.apply_scroll_type(scroll_type);
        self.scroll_left = position.x;
        self.scroll_top = position.y;
        self.clamp_scroll();
        self.emit_scroll_changed(cx);
        cx.notify();
    }

    /// Reports the scroll position and scrollable size with
    /// [`CodeEditorEvent::ScrollChanged`] if they changed since the last
    /// report (Monaco's `Scrollable` only fires when its state changed).
    pub(crate) fn emit_scroll_changed(&mut self, cx: &mut Context<Self>) {
        let state = [
            self.scroll_top,
            self.scroll_left,
            self.scroll_width().unwrap_or_default(),
            self.scroll_height().unwrap_or_default(),
        ];
        let old = self.last_scroll_state.replace(state);
        let changed = |i: usize| old.is_none_or(|old| old[i] != state[i]);
        let event = ScrollEvent {
            scroll_top: state[0],
            scroll_left: state[1],
            scroll_width: state[2],
            scroll_height: state[3],
            scroll_top_changed: changed(0),
            scroll_left_changed: changed(1),
            scroll_width_changed: changed(2),
            scroll_height_changed: changed(3),
        };
        if event.scroll_top_changed
            || event.scroll_left_changed
            || event.scroll_width_changed
            || event.scroll_height_changed
        {
            cx.emit(CodeEditorEvent::ScrollChanged(event));
        }
    }

    /// Makes the next scroll change animate or jump.
    pub(crate) fn apply_scroll_type(&mut self, scroll_type: crate::view::reveal::ScrollType) {
        match scroll_type {
            crate::view::reveal::ScrollType::Smooth => {
                self.animation.smooth_scroll_requested = true;
            }
            crate::view::reveal::ScrollType::Immediate => {
                self.animation.smooth_scroll_requested = false;
                self.animation.cancel_scroll();
            }
        }
    }

    /// The height of the lines and view zones as of the last layout
    /// (Monaco `getContentHeight`). Unlike Monaco's, it leaves out what
    /// Monaco adds to the lines' height: with `scroll_beyond_last_line`,
    /// the editor's height less a line and `padding.bottom`; without it,
    /// the horizontal scrollbar's height when that shows. The scrollable
    /// height, with the `scroll_beyond_last_line` room, is
    /// [`CodeEditor::scroll_height`].
    pub fn content_height(&self) -> Option<Pixels> {
        self.layout.as_ref().map(|layout| layout.content_height)
    }

    /// Scrolls without clamping to the last frame's range, for hosts that
    /// resize the editor as they scroll it (the next frame clamps).
    pub(crate) fn set_scroll_position_unclamped(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.scroll_position() != position {
            self.scroll_left = position.x;
            self.scroll_top = position.y;
            cx.notify();
        }
    }

    /// Keeps the scroll position within the last frame's scroll range (the
    /// next frame clamps again for the current one).
    fn clamp_scroll(&mut self) {
        let max = self
            .layout
            .as_ref()
            .map_or(point(Pixels::MAX, Pixels::MAX), |layout| layout.max_scroll);
        self.scroll_top = self.scroll_top.clamp(px(0.), max.y);
        self.scroll_left = self.scroll_left.clamp(px(0.), max.x);
    }

    /// Hides whole rows besides folds (Monaco `setHiddenAreas` without a
    /// source): each range is 0-based rows, end exclusive. Replaces the
    /// areas set before without a source; see
    /// [`CodeEditor::set_hidden_areas_from`].
    pub fn set_hidden_areas(&mut self, areas: Vec<Range<u32>>, cx: &mut Context<Self>) {
        self.set_hidden_areas_inner(None, areas, cx);
    }

    /// Hides whole rows for `source` (Monaco `setHiddenAreas(ranges,
    /// source)`): each source's areas replace only that source's, and the
    /// rows hidden are the union of every source's. The diff editor hides
    /// its unchanged regions as the source `"diffEditor"`.
    pub fn set_hidden_areas_from(
        &mut self,
        source: impl Into<gpui::SharedString>,
        areas: Vec<Range<u32>>,
        cx: &mut Context<Self>,
    ) {
        self.set_hidden_areas_inner(Some(source.into()), areas, cx);
    }

    fn set_hidden_areas_inner(
        &mut self,
        source: Option<gpui::SharedString>,
        areas: Vec<Range<u32>>,
        cx: &mut Context<Self>,
    ) {
        let index = self.hidden_areas.iter().position(|(s, _)| *s == source);
        match index {
            Some(index) if self.hidden_areas[index].1 == areas => return,
            Some(index) if areas.is_empty() => {
                self.hidden_areas.remove(index);
            }
            Some(index) => self.hidden_areas[index].1 = areas,
            None if areas.is_empty() => return,
            None => self.hidden_areas.push((source, areas)),
        }
        cx.notify();
    }

    /// The rows hidden by [`CodeEditor::set_hidden_areas`] and
    /// [`CodeEditor::set_hidden_areas_from`], of every source, sorted and
    /// merged.
    pub fn hidden_areas(&self) -> Vec<Range<u32>> {
        normalize_hidden(
            self.hidden_areas
                .iter()
                .flat_map(|(_, areas)| areas.iter().cloned())
                .collect(),
        )
    }

    /// Inserts a host view between lines (Monaco `changeViewZones` +
    /// `addZone`).
    pub fn add_view_zone(&mut self, zone: ViewZone, cx: &mut Context<Self>) -> WidgetId {
        self.add_zone(zone.after_line, zone.height, zone.view, false, cx)
    }

    /// Adds a zone; `full_width` ones also cover the gutter (Monaco's zone
    /// widgets).
    pub(crate) fn add_zone(
        &mut self,
        after_line: u32,
        height: ZoneHeight,
        view: gpui::AnyView,
        full_width: bool,
        cx: &mut Context<Self>,
    ) -> WidgetId {
        let id = self.widgets.next_id();
        let buffer = self.model.read(cx).buffer();
        let anchor = (after_line > 0).then(|| {
            let row = (after_line - 1).min(buffer.max_point().row);
            buffer.anchor_before(buffer.point_to_offset(text::Point::new(row, u32::MAX)))
        });
        self.widgets.zones.push(ZoneState {
            id,
            anchor,
            height,
            view,
            full_width,
            captures_wheel: false,
            min_width: px(0.),
        });
        self.widgets.zones_generation += 1;
        cx.emit(CodeEditorEvent::ViewZonesChanged);
        cx.notify();
        id
    }

    /// Changes the height of a zone added with [`CodeEditor::add_view_zone`].
    pub fn set_view_zone_height(
        &mut self,
        id: WidgetId,
        height: ZoneHeight,
        cx: &mut Context<Self>,
    ) {
        if let Some(zone) = self.widgets.zones.iter_mut().find(|z| z.id == id)
            && zone.height != height
        {
            zone.height = height;
            cx.emit(CodeEditorEvent::ViewZonesChanged);
            cx.notify();
        }
    }

    /// Adds a host view anchored to a text position (Monaco
    /// `addContentWidget`).
    pub fn add_content_widget(
        &mut self,
        widget: ContentWidget,
        cx: &mut Context<Self>,
    ) -> WidgetId {
        let id = self.widgets.next_id();
        let anchor = self.anchor_for(widget.position, cx);
        self.widgets.content.push(ContentWidgetState {
            id,
            anchor,
            preference: widget.preference,
            view: widget.view,
            allow_overflow: widget.allow_editor_overflow,
            layer: crate::widgets::layer::DEFAULT,
            room: Default::default(),
        });
        cx.notify();
        id
    }

    /// Adds a view in the glyph margin beside a line (Monaco
    /// `addGlyphMarginWidget`). Remove it with [`CodeEditor::remove_widget`].
    pub fn add_glyph_margin_widget(
        &mut self,
        widget: crate::widgets::GlyphMarginWidget,
        cx: &mut Context<Self>,
    ) -> WidgetId {
        let id = self.widgets.next_id();
        let anchor = self.anchor_for(PointUtf16::new(widget.row, 0), cx);
        self.widgets.glyphs.push(crate::widgets::GlyphWidgetState {
            id,
            anchor,
            view: widget.view,
            lane: widget.lane,
            z_index: widget.z_index,
        });
        cx.notify();
        id
    }

    /// Moves a content widget (Monaco `layoutContentWidget`).
    pub fn set_content_widget_position(
        &mut self,
        id: WidgetId,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        let anchor = self.anchor_for(position, cx);
        if let Some(widget) = self.widgets.content.iter_mut().find(|w| w.id == id) {
            widget.anchor = anchor;
            cx.notify();
        }
    }

    /// Adds a host view pinned to the editor (Monaco `addOverlayWidget`).
    pub fn add_overlay_widget(
        &mut self,
        widget: OverlayWidget,
        cx: &mut Context<Self>,
    ) -> WidgetId {
        let id = self.widgets.next_id();
        self.widgets.overlays.push(OverlayWidgetState {
            id,
            position: widget.position,
            view: widget.view,
            allow_overflow: widget.allow_editor_overflow,
            layer: crate::widgets::layer::DEFAULT,
        });
        cx.notify();
        id
    }

    /// Adds one of the editor's own content widgets that overflow it
    /// (Monaco's widgets with `allowEditorOverflow`), painted at `layer`;
    /// returns its id and where it learns the room it has.
    pub(crate) fn add_overflow_content_widget(
        &mut self,
        position: PointUtf16,
        preference: Vec<crate::widgets::ContentWidgetPreference>,
        view: gpui::AnyView,
        layer: usize,
        cx: &mut Context<Self>,
    ) -> (WidgetId, crate::widgets::OverflowRoomCell) {
        let id = self.add_content_widget(
            ContentWidget {
                position,
                preference,
                view,
                allow_editor_overflow: true,
            },
            cx,
        );
        self.widgets.set_layer(id, layer);
        (id, self.widgets.room(id))
    }

    /// Adds one of the editor's own overlay widgets that overflow it,
    /// painted at `layer`.
    pub(crate) fn add_overflow_overlay_widget(
        &mut self,
        position: OverlayPosition,
        view: gpui::AnyView,
        layer: usize,
        cx: &mut Context<Self>,
    ) -> WidgetId {
        let id = self.add_overlay_widget(
            OverlayWidget {
                position,
                view,
                allow_editor_overflow: true,
            },
            cx,
        );
        self.widgets.set_layer(id, layer);
        id
    }

    /// Removes a view zone, content widget or overlay widget.
    pub fn remove_widget(&mut self, id: WidgetId, cx: &mut Context<Self>) {
        let zones = self.widgets.zones_generation;
        if self.widgets.remove(id) {
            if self.widgets.zones_generation != zones {
                cx.emit(CodeEditorEvent::ViewZonesChanged);
            }
            cx.notify();
        }
    }

    fn anchor_for(&self, position: PointUtf16, cx: &App) -> text::Anchor {
        let buffer = self.model.read(cx).buffer();
        let position = buffer.clip_point_utf16(position, Bias::Left);
        buffer.anchor_before(buffer.point_utf16_to_offset(position))
    }

    /// The editor's size at its last layout.
    pub(crate) fn layout_size(&self) -> Option<gpui::Size<Pixels>> {
        Some(self.layout.as_ref()?.bounds.size)
    }

    /// Window position of the top-left of `point`'s character, if it's laid
    /// out. Monaco's `getScrolledVisiblePosition` is relative to the editor
    /// and has the line's height:
    /// [`CodeEditor::scrolled_visible_position_in_editor`].
    pub fn scrolled_visible_position(&self, point: PointUtf16) -> Option<Point<Pixels>> {
        let layout = self.layout.as_ref()?;
        let line = layout.line_for_point(point)?;
        Some(gpui::point(
            layout.text_origin_x() + line.x_for_column(point.column),
            layout.line_top(line.view_line),
        ))
    }

    /// (sticky scroll rows, whether a minimap is laid out) from the last
    /// layout.
    #[doc(hidden)]
    pub fn sticky_rows_for_tests(&self) -> (Vec<u32>, bool) {
        self.layout.as_ref().map_or((Vec::new(), false), |layout| {
            (
                layout.sticky_lines.iter().map(|line| line.row).collect(),
                layout.minimap.is_some(),
            )
        })
    }

    /// The editor's bounds and the visible parts of its zones and widgets
    /// (top-most last) from the last layout, for tests.
    #[doc(hidden)]
    pub fn widget_bounds_for_tests(
        &self,
    ) -> (
        Bounds<Pixels>,
        Vec<(Bounds<Pixels>, crate::mouse_target::EmbeddedKind)>,
    ) {
        self.layout
            .as_ref()
            .map_or_else(Default::default, |layout| {
                (
                    layout.bounds,
                    layout
                        .widget_bounds
                        .iter()
                        .copied()
                        .zip(layout.widget_kinds.iter().copied())
                        .collect(),
                )
            })
    }

    /// (content y of the last view line, line height) from the last layout.
    #[doc(hidden)]
    pub fn layout_for_tests(&self) -> Option<(Pixels, Pixels)> {
        let layout = self.layout.as_ref()?;
        let last = self.view_lines.line_count().saturating_sub(1);
        Some((
            layout.zones.line_y(last, layout.metrics.line_height),
            layout.metrics.line_height,
        ))
    }

    /// The minimap's bounds and height per view line, and the text area's
    /// left edge, for tests.
    #[doc(hidden)]
    pub fn minimap_for_tests(&self) -> Option<(Bounds<Pixels>, Pixels, Pixels)> {
        let layout = self.layout.as_ref()?;
        let minimap = layout.minimap?;
        Some((
            minimap.bounds,
            minimap.line_height,
            layout.text_bounds.left(),
        ))
    }

    /// The line number labels of the last frame, by row, for tests.
    #[doc(hidden)]
    pub fn line_number_labels_for_tests(&self) -> Vec<(u32, String)> {
        self.layout
            .as_ref()
            .map_or_else(Vec::new, |l| l.line_number_labels.clone())
    }

    /// The line numbers of the last frame: row, left edge and color, for
    /// tests.
    #[doc(hidden)]
    pub fn line_number_styles_for_tests(&self) -> Vec<(u32, Pixels, Hsla)> {
        self.layout
            .as_ref()
            .map_or_else(Vec::new, |l| l.line_number_styles.clone())
    }

    /// The sticky lines of the last frame: row, visible bounds and folding
    /// control bounds, topmost first, and the widget's height, for tests.
    #[doc(hidden)]
    #[allow(clippy::type_complexity)]
    pub fn sticky_lines_for_tests(
        &self,
    ) -> (Vec<(u32, Bounds<Pixels>, Option<Bounds<Pixels>>)>, Pixels) {
        self.layout.as_ref().map_or((Vec::new(), px(0.)), |l| {
            let lines = l
                .sticky_lines
                .iter()
                .map(|line| (line.row, line.bounds, line.fold.map(|f| f.bounds)))
                .collect();
            let height = l
                .sticky_lines
                .iter()
                .map(|line| line.bounds.bottom() - l.bounds.top())
                .fold(px(0.), Pixels::max);
            (lines, height)
        })
    }

    /// The current line highlight's boxes in the last frame: view line,
    /// whether it's the gutter's, bounds, background and whether it has a
    /// border, for tests.
    #[doc(hidden)]
    #[allow(clippy::type_complexity)]
    pub fn line_highlights_for_tests(
        &self,
    ) -> Vec<(u32, bool, Bounds<Pixels>, Option<Hsla>, bool)> {
        self.layout.as_ref().map_or_else(Vec::new, |l| {
            l.line_highlights
                .iter()
                .map(|h| {
                    (
                        h.view_line,
                        h.margin,
                        h.bounds,
                        h.background,
                        h.border.is_some(),
                    )
                })
                .collect()
        })
    }

    /// The glyph margin glyphs and widgets drawn in the last frame, as (view
    /// line, lane index), for tests.
    #[doc(hidden)]
    pub fn glyphs_for_tests(&self) -> Vec<(u32, usize)> {
        self.layout
            .as_ref()
            .map_or_else(Vec::new, |l| l.glyphs_drawn.clone())
    }

    /// The folding controls of the last frame: row, bounds and whether
    /// collapsed, for tests.
    #[doc(hidden)]
    pub fn fold_controls_for_tests(&self) -> Vec<(u32, Bounds<Pixels>, bool)> {
        self.layout.as_ref().map_or_else(Vec::new, |l| {
            l.fold_controls
                .iter()
                .map(|c| (c.row, c.bounds, c.collapsed))
                .collect()
        })
    }

    /// The cursors of the last frame and whether they're outlined, for
    /// tests.
    #[doc(hidden)]
    pub fn cursor_shapes_for_tests(&self) -> (Vec<Bounds<Pixels>>, bool) {
        self.layout
            .as_ref()
            .map_or_else(|| (Vec::new(), false), |l| l.cursors.clone())
    }

    /// Each cursor's (view line, view column), Monaco's
    /// `CursorState.viewState.position`, for tests.
    #[doc(hidden)]
    pub fn cursor_view_positions_for_tests(&self) -> Vec<(u32, u32)> {
        self.cursor_view_positions()
            .into_iter()
            .map(|p| (p.row, p.column))
            .collect()
    }

    /// The (row, UTF-16 columns) of each view line laid out in the last
    /// frame, for tests.
    #[doc(hidden)]
    pub fn view_line_segments_for_tests(&self) -> Vec<(u32, Range<u32>)> {
        self.layout.as_ref().map_or_else(Vec::new, |l| {
            l.lines
                .iter()
                .map(|line| (line.row, line.columns.clone()))
                .collect()
        })
    }

    /// The rows and left edges (from the text's start) of the
    /// `word_wrap_indicator` glyphs in the last frame, for tests.
    #[doc(hidden)]
    pub fn wrap_indicators_for_tests(&self) -> Vec<(u32, Pixels)> {
        self.layout
            .as_ref()
            .map_or_else(Vec::new, |l| l.wrap_indicators.clone())
    }

    /// The mouse cursor over the text without links, for tests.
    #[doc(hidden)]
    pub fn text_mouse_cursor_for_tests(&self) -> MouseCursor {
        self.text_mouse_cursor()
    }

    /// The minimap's (opacity, takes the mouse) in the last frame, for
    /// tests.
    #[doc(hidden)]
    pub fn minimap_visibility_for_tests(&self) -> Option<(f32, bool)> {
        let minimap = self.layout.as_ref()?.minimap?;
        Some((minimap.opacity, minimap.interactive))
    }

    /// The vertical scrollbar's (bar, thumb) bounds, for tests.
    #[doc(hidden)]
    pub fn vertical_scrollbar_for_tests(&self) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
        let bar = self.layout.as_ref()?.vertical_scrollbar?;
        Some((bar.bar, bar.thumb))
    }

    /// The overview ruler from the last layout, for tests.
    #[doc(hidden)]
    pub fn overview_ruler_for_tests(&self) -> Option<OverviewRulerForTests> {
        let ruler = self.overview_ruler_cache.painted.as_ref()?;
        let marks = |marks: &[(crate::view::overview_ruler::CanvasRect, Hsla)]| {
            marks
                .iter()
                .map(|(rect, color)| (ruler.bounds_of(*rect), *color))
                .collect()
        };
        Some(OverviewRulerForTests {
            bounds: ruler.bounds,
            background: ruler.background,
            marks: marks(&ruler.marks),
            cursors: marks(&ruler.cursors),
            border: ruler.border_rects(),
        })
    }

    /// The top scroll shadow's box from the last layout, for tests.
    #[doc(hidden)]
    pub fn scroll_shadow_for_tests(&self) -> Option<Bounds<Pixels>> {
        self.layout.as_ref()?.scroll_decoration
    }

    /// Whether the blinking cursor is in its visible phase, for tests.
    #[doc(hidden)]
    pub fn cursor_visible_for_tests(&self) -> bool {
        self.cursor_visible
    }

    /// (shown scroll top, max horizontal scroll, text area height, line
    /// height) from the last layout, for tests.
    #[doc(hidden)]
    pub fn view_metrics_for_tests(&self) -> Option<(Pixels, Pixels, Pixels, Pixels)> {
        let layout = self.layout.as_ref()?;
        Some((
            layout.scroll_top,
            layout.max_scroll.x,
            layout.text_bounds.size.height,
            layout.metrics.line_height,
        ))
    }

    pub(crate) fn cursor_config(&self, cx: &App) -> CursorConfig {
        let model_options = self.model.read(cx).options();
        CursorConfig {
            tab_size: model_options.tab_size,
            indent_size: model_options.indent_size,
            insert_spaces: model_options.insert_spaces,
            word_separators: self.options.word_separators.to_string(),
            word_segmenter_locales: self
                .options
                .word_segmenter_locales
                .iter()
                .map(ToString::to_string)
                .collect(),
            sticky_tab_stops: self.options.sticky_tab_stops,
            use_tab_stops: self.options.use_tab_stops,
            trim_whitespace_on_delete: self.options.trim_whitespace_on_delete,
            empty_selection_clipboard: self.options.empty_selection_clipboard,
            auto_closing_brackets: self.options.auto_closing_brackets,
            auto_closing_quotes: self.options.auto_closing_quotes,
            auto_closing_comments: self.options.auto_closing_comments,
            auto_closing_overtype: self.options.auto_closing_overtype,
            auto_closing_delete: self.options.auto_closing_delete,
            auto_surround: self.options.auto_surround,
            auto_indent: self.options.auto_indent,
            overtype: self.overtype,
            overtype_on_paste: self.options.overtype_on_paste,
            multi_cursor_paste: self.options.multi_cursor_paste,
        }
    }

    pub(crate) fn marked_range_points(&self) -> Option<Range<PointUtf16>> {
        let range = self.marked_range.as_ref()?;
        Some(
            self.snapshot.offset_to_point_utf16(range.start)
                ..self.snapshot.offset_to_point_utf16(range.end),
        )
    }

    /// The IME's selected part of the composition (its active clause), if
    /// it selected any text.
    pub(crate) fn marked_selection_points(&self) -> Option<Range<PointUtf16>> {
        self.marked_range.as_ref()?;
        let range = self.marked_selection.as_ref().filter(|r| !r.is_empty())?;
        Some(
            self.snapshot.offset_to_point_utf16(range.start)
                ..self.snapshot.offset_to_point_utf16(range.end),
        )
    }

    /// The IME composition range and its selected part (active clause), as
    /// UTF-16 columns, for tests.
    #[doc(hidden)]
    pub fn composition_for_tests(&self) -> Option<(Range<PointUtf16>, Option<Range<PointUtf16>>)> {
        Some((self.marked_range_points()?, self.marked_selection_points()))
    }

    /// Monaco `compositionType` across cursors: replaces `range` (bytes),
    /// which must be around the collapsed primary cursor on its line, by
    /// `text` at every cursor (the same number of characters before and
    /// after each), leaving each cursor `position_delta` UTF-16 units from
    /// the end of its text. Returns the primary cursor's new composition
    /// range, or `None` (nothing done) when `range` isn't around the
    /// primary cursor.
    fn composition_type_at_cursors(
        &mut self,
        range: Range<usize>,
        text: &str,
        position_delta: i32,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let primary = self.cursors.primary().selection();
        let head = primary.head;
        let start = self.snapshot.offset_to_point_utf16(range.start);
        let end = self.snapshot.offset_to_point_utf16(range.end);
        if !primary.is_empty() || start.row != head.row || end.row != head.row {
            return None;
        }
        if start > head || head > end {
            return None;
        }
        let operation = cursor::composition_type(
            self.snapshot.rope(),
            &self.cursors,
            text,
            head.column - start.column,
            end.column - head.column,
            position_delta,
            self.prev_edit_kind,
        );
        self.apply(operation, cx);
        let head = self.cursors.primary().position;
        let len = text.encode_utf16().count() as i64;
        let column = (head.column as i64 - position_delta as i64 - len).max(0) as u32;
        let start = self
            .snapshot
            .point_utf16_to_offset(PointUtf16::new(head.row, column));
        Some(start..start + text.len())
    }

    fn page_lines(&self) -> u32 {
        self.layout.as_ref().map_or(20, EditorLayout::page_rows)
    }

    // ----- model synchronisation -------------------------------------------------

    fn on_model_event(
        &mut self,
        model: Entity<TextModel>,
        event: &TextModelEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextModelEvent::Edited {
                source,
                kind,
                restore,
                changes,
                version,
                eol,
                is_eol_change,
                ..
            } => {
                // Monaco's view model forwards the content change before
                // the cursors react to it.
                cx.emit(CodeEditorEvent::ModelContentChanged(ModelContentChange {
                    changes: changes.clone(),
                    eol: *eol,
                    version_id: *version,
                    is_undoing: *kind == EditKind::Undo,
                    is_redoing: *kind == EditKind::Redo,
                    is_flush: *kind == EditKind::SetValue,
                    is_eol_change: *is_eol_change,
                }));
                let own = *source == Some(cx.entity_id());
                // Monaco skips undoing.
                if *kind != EditKind::Undo
                    && changes.iter().any(|change| {
                        crate::unusual_line_terminators::contains_unusual_line_terminators(
                            &change.text,
                        )
                    })
                {
                    self.request_unusual_line_terminators_check();
                }
                // Monaco cancels the rename box when the text changes, and
                // asks for inlay hints again.
                self.cancel_rename(false, cx);
                self.inlay_hints_after_edit(cx);
                self.semantic_tokens_after_edit(changes, cx);
                self.inline_completions_after_edit(*kind, cx);
                self.linked_editing_after_edit(changes, own, cx);
                let latest = model.read(cx).snapshot();
                if latest.version() == self.snapshot.version() {
                    return;
                }
                self.folds.prune(model.read(cx).buffer());
                let restored = own
                    && matches!(kind, EditKind::Undo | EditKind::Redo)
                    && restore
                        .as_ref()
                        .and_then(|meta| meta.downcast_ref::<Vec<Selection>>())
                        .map(|selections| self.cursors.set_selections(selections))
                        .is_some();
                if !restored {
                    let patch = model.read(cx).buffer().edits_since(self.snapshot.version());
                    let old = &self.snapshot;
                    let transform = |p: PointUtf16, bias: Bias| {
                        let offset = old.point_utf16_to_offset(p);
                        latest.offset_to_point_utf16(patch.transform_offset(offset, bias))
                    };
                    let selections: Vec<Selection> = self
                        .cursors
                        .selections()
                        .into_iter()
                        .map(|s| Selection {
                            anchor: transform(s.anchor, Bias::Left),
                            head: transform(s.head, Bias::Left),
                        })
                        .collect();
                    self.cursors.set_selections(&selections);
                }
                // Undo/redo and edits from elsewhere: stop treating closing
                // characters as auto-inserted rather than tracking them.
                self.cursors.auto_closed_ranges_mut().clear();
                self.snapshot = latest;
                self.cursors.validate(self.snapshot.rope());
                if *kind == EditKind::SetValue {
                    self.set_marked_range(None, cx);
                }
                self.find_content_changed(*kind == EditKind::SetValue, cx);
                if !own {
                    self.prev_edit_kind = None;
                    self.occurrences_after_edit(cx);
                }
                if !own || matches!(kind, EditKind::Undo | EditKind::Redo) {
                    self.next_selection_change = CursorChangeCause {
                        reason: match kind {
                            EditKind::Undo => CursorChangeReason::Undo,
                            EditKind::Redo => CursorChangeReason::Redo,
                            EditKind::SetValue => CursorChangeReason::ContentFlush,
                            EditKind::Edit => CursorChangeReason::RecoverFromMarkers,
                        },
                        source: match kind {
                            EditKind::SetValue => CursorChangeSource::Model,
                            _ => CursorChangeSource::ModelChange,
                        },
                    };
                    self.cursors_changed(own, cx);
                }
                cx.notify();
            }
            TextModelEvent::SyntaxUpdated => {
                self.update_bracket_highlights(cx);
                self.refresh_bracket_colors(cx);
                // Syntax folding follows the new trees.
                if self.options.folding
                    && self.options.folding_strategy == crate::options::FoldingStrategy::Auto
                {
                    self.compute_fold_ranges(cx);
                }
                cx.emit(CodeEditorEvent::ModelTokensChanged);
                cx.notify();
            }
            TextModelEvent::LanguageChanged {
                old_language,
                new_language,
            } => {
                // Monaco closes the peek view.
                self.close_peek(cx);
                // And the inspect tokens widget.
                self.stop_inspect_tokens(cx);
                self.reset_inlay_hints(cx);
                cx.emit(CodeEditorEvent::ModelLanguageChanged {
                    old_language: old_language.clone(),
                    new_language: new_language.clone(),
                });
                cx.notify();
            }
            TextModelEvent::MarkersChanged => {
                self.schedule_auto_code_actions(cx);
                cx.notify();
            }
            TextModelEvent::LanguageConfigurationChanged => {
                // Brackets may have changed: find the pairs again.
                self.bracket_colors = Default::default();
                cx.emit(CodeEditorEvent::ModelLanguageConfigurationChanged);
                cx.notify();
            }
            TextModelEvent::OptionsChanged(change) => {
                cx.emit(CodeEditorEvent::ModelOptionsChanged(*change));
                cx.notify();
            }
            TextModelEvent::DecorationsChanged(change) => {
                cx.emit(CodeEditorEvent::ModelDecorationsChanged(*change));
                cx.notify();
            }
            TextModelEvent::LineEndingChanged => cx.notify(),
            TextModelEvent::AttachedChanged => {}
        }
    }

    /// Applies an edit computed by the cursor core: converts it to byte
    /// ranges, pushes it to the model with undo grouping and cursor metadata,
    /// then moves the cursors to the operation's resulting selections.
    /// Returns where the text of each of the operation's edits ended up, in
    /// byte offsets of the new text (empty if nothing was applied).
    pub(crate) fn apply(
        &mut self,
        operation: EditOperation,
        cx: &mut Context<Self>,
    ) -> Vec<Range<usize>> {
        // Monaco's view model batches its changes (`batchChanges`).
        self.begin_update(cx);
        let ranges = self.apply_inner(operation, cx);
        self.end_update(cx);
        ranges
    }

    fn apply_inner(
        &mut self,
        mut operation: EditOperation,
        cx: &mut Context<Self>,
    ) -> Vec<Range<usize>> {
        // Monaco `_executeCursorEdit`: read-only editors refuse cursor
        // edits, and say so.
        if self.options.read_only {
            self.read_only_edit_attempt(cx);
            return Vec::new();
        }
        // The hover's color picker edits the text without closing.
        if !self.color_picker.editing {
            self.hide_hover(cx);
        }
        if operation.edits.is_empty() {
            if operation.selections_after != self.cursors.selections() {
                self.cursors.set_selections(&operation.selections_after);
                self.cursors_changed(true, cx);
            }
            return Vec::new();
        }
        let snapshot = self.snapshot.clone();
        let edits: Vec<(Range<usize>, String, bool)> = operation
            .edits
            .iter()
            .enumerate()
            .map(|(i, (range, text))| {
                (
                    snapshot.point_utf16_to_offset(range.start)
                        ..snapshot.point_utf16_to_offset(range.end),
                    text.clone(),
                    operation.auto_whitespace.contains(&i),
                )
            })
            .collect();
        let selections_before = self.cursors.selections();
        let cursor_rows: Vec<_> = selections_before
            .iter()
            .map(|s| s.start().row..=s.end().row)
            .collect();
        let source = cx.entity_id();
        let group = self.edit_group.or(undo_group(operation.kind));
        let selections_after = &mut operation.selections_after;
        let result = self.model.update(cx, |model, cx| {
            if operation.push_stack_element_before {
                model.push_stack_element();
            }
            let result = model.push_cursor_edit_operations(
                Some(&cursor_rows),
                edits,
                group,
                Some(source),
                cx,
            );
            if let Ok(applied) = &result {
                crate::model::shift_past_trimmed(selections_after, &applied.trimmed);
                model.set_undo_metadata(
                    Some(metadata(selections_before)),
                    Some(metadata(selections_after.clone())),
                );
            }
            if operation.push_stack_element_after {
                model.push_stack_element();
            }
            result
        });
        let Ok(applied) = result else {
            return Vec::new();
        };
        self.cursors
            .track_auto_closed(snapshot.rope(), &operation.edits, &operation.auto_closed);
        self.snapshot = self.model.read(cx).snapshot();
        self.cursors.set_selections(&operation.selections_after);
        self.cursors.validate(self.snapshot.rope());
        self.prev_edit_kind = Some(operation.kind);
        self.cursors_changed(true, cx);
        if let Some(follow_up) = operation.follow_up {
            self.apply(*follow_up, cx);
        }
        applied.ranges
    }

    /// Whether `modifiers` add a cursor on click (Monaco
    /// `multiCursorModifier`).
    fn is_add_cursor_modifier(&self, modifiers: &gpui::Modifiers) -> bool {
        match self.options.multi_cursor_modifier {
            crate::options::MultiCursorModifier::Alt => modifiers.alt,
            crate::options::MultiCursorModifier::CtrlCmd => modifiers.secondary(),
        }
    }

    /// Whether `modifiers` show and follow definition links: the modifier
    /// that doesn't add cursors.
    pub(crate) fn is_definition_modifier(&self, modifiers: &gpui::Modifiers) -> bool {
        match self.options.multi_cursor_modifier {
            crate::options::MultiCursorModifier::Alt => modifiers.secondary(),
            crate::options::MultiCursorModifier::CtrlCmd => modifiers.alt,
        }
    }

    /// Monaco's `hasSideBySideModifier` for Cmd/Ctrl+click on a symbol: Alt,
    /// or Cmd/Ctrl when Alt adds cursors.
    pub(crate) fn is_side_by_side_modifier(&self, modifiers: &gpui::Modifiers) -> bool {
        match self.options.multi_cursor_modifier {
            crate::options::MultiCursorModifier::Alt => modifiers.alt,
            crate::options::MultiCursorModifier::CtrlCmd => modifiers.secondary(),
        }
    }

    pub(crate) fn cursors_changed(&mut self, autoscroll: bool, cx: &mut Context<Self>) {
        // Monaco `multiCursorLimit`: extra cursors are dropped.
        let limit = self.options.multi_cursor_limit.max(1);
        if self.cursors.all().len() > limit {
            let kept = self.cursors.all()[..limit].to_vec();
            self.cursors.set_states(kept);
        }
        // Cursors set from model positions (edits, `set_selections`, ...)
        // get their view positions from the model again, as Monaco's
        // `CursorState.fromModelState`.
        if self
            .cursor_views
            .as_ref()
            .is_some_and(|views| views.model.as_slice() != self.cursors.all())
        {
            self.cursor_views = None;
        }
        // Monaco `_emitStateChangedIfNecessary`: nothing happens when the
        // cursor state (the selections and the model version) is the same.
        if self
            .last_cursor_state
            .as_ref()
            .is_some_and(|(selections, version)| {
                *version == self.snapshot.version() && *selections == self.cursors.selections()
            })
        {
            let cause = std::mem::take(&mut self.next_selection_change);
            // Keyboard and mouse moves still reveal the cursor, as
            // Monaco's commands do; the API's don't.
            if autoscroll && cause.source != CursorChangeSource::Api {
                self.autoscroll_requested = true;
                self.autoscroll_minimal = false;
                self.autoscroll_vertical_only = false;
                self.autoscroll_from_mouse = cause.source == CursorChangeSource::Mouse;
                cx.notify();
            }
            return;
        }
        self.suggest_cursor_changed();
        self.write_selection_to_primary(cx);
        self.snippet_selections_changed(cx);
        if autoscroll {
            self.autoscroll_requested = true;
            self.autoscroll_minimal = false;
            self.autoscroll_vertical_only = false;
            self.autoscroll_from_mouse =
                self.next_selection_change.source == CursorChangeSource::Mouse;
            self.animation.smooth_scroll_requested = true;
        }
        self.record_cursor_change();
        self.close_message(cx);
        self.check_symbol_navigation(cx);
        self.inspect_tokens_cursor_changed(cx);
        self.check_rename(cx);
        self.dismiss_keyboard_definition_link(cx);
        self.smart_select_cursor_changed();
        self.schedule_auto_code_actions(cx);
        self.update_bracket_highlights(cx);
        self.multi_cursor_selections_changed(cx);
        self.update_occurrence_highlights(cx);
        self.restart_blink(cx);
        let change = std::mem::take(&mut self.next_selection_change);
        self.inline_selections_changed(&change, cx);
        self.linked_editing_cursor_changed(cx);
        // Monaco `ParameterHintsModel.onCursorChange`: clicks close parameter
        // hints, other changes re-query them while they're shown.
        if change.source == CursorChangeSource::Mouse {
            self.hide_signature_help(cx);
        } else if self.is_signature_help_visible() {
            self.trigger_signature_help(crate::providers::SignatureHelpTrigger::ContentChange, cx);
        }
        self.color_picker_cursor_changed(cx);
        self.find_cursor_changed(change.reason, cx);
        // Monaco pauses the caret transition when the cursor count changes,
        // and for non-explicit moves with `explicit`.
        let count = self.cursors.all().len();
        self.animation.caret_jump = count
            != std::mem::replace(&mut self.animation.cursor_count, count)
            || (self.options.cursor_smooth_caret_animation
                == crate::options::CursorSmoothCaretAnimation::Explicit
                && change.reason != CursorChangeReason::Explicit);
        // The cursor moving means the keyboard is back in the text.
        self.sticky.focused = None;
        self.emit_cursor_events(change, cx);
        cx.notify();
    }

    /// Records the cursor state and, when the selections differ from the
    /// last ones reported, emits [`CodeEditorEvent::CursorPositionChanged`]
    /// and [`CodeEditorEvent::SelectionsChanged`] (Monaco's outgoing
    /// `CursorStateChangedEvent`, fired only when a cursor's model state
    /// changed).
    pub(crate) fn emit_cursor_events(&mut self, cause: CursorChangeCause, cx: &mut Context<Self>) {
        let selections = self.cursors.selections();
        let version = self.snapshot.version();
        let old = self
            .last_cursor_state
            .replace((selections.clone(), version));
        if old.as_ref().is_some_and(|(old, _)| *old == selections) {
            return;
        }
        let (old_selections, old_model_version_id) = match old {
            Some((selections, version)) => (Some(selections), version),
            None => (None, 0),
        };
        cx.emit(CodeEditorEvent::CursorPositionChanged(
            CursorPositionChange {
                position: selections[0].head,
                secondary_positions: selections[1..].iter().map(|s| s.head).collect(),
                reason: cause.reason,
                source: cause.source,
            },
        ));
        cx.emit(CodeEditorEvent::SelectionsChanged(SelectionChange {
            reason: cause.reason,
            source: cause.source,
            selection: selections[0],
            secondary_selections: selections[1..].to_vec(),
            old_selections,
            model_version_id: version,
            old_model_version_id,
        }));
    }

    /// Monaco `_getCursorBlinking`: read-only editors don't blink.
    pub(crate) fn cursor_blinking(&self) -> crate::options::CursorBlinking {
        if self.options.read_only {
            crate::options::CursorBlinking::Solid
        } else {
            self.options.cursor_blinking
        }
    }

    /// Monaco `_updateBlinking`: restarts the blink after the cursor moves
    /// or the editor is focused.
    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        use crate::options::CursorBlinking;
        self.cursor_visible = true;
        self.blink_epoch += 1;
        self.animation.blink_started = web_time::Instant::now();
        let epoch = self.blink_epoch;
        match self.cursor_blinking() {
            CursorBlinking::Solid => {
                self._blink_task = None;
                return;
            }
            CursorBlinking::Blink => {}
            _ => {
                // The CSS animations start after one blink interval and
                // run 20 half-second iterations; repaint while they change
                // the cursor (Monaco's run on the compositor).
                let style = self.cursor_blinking();
                self._blink_task = Some(cx.spawn(async move |this, cx| {
                    let start = web_time::Instant::now();
                    while let Some(delay) =
                        crate::view::animation::cursor_repaint_delay(style, start.elapsed())
                    {
                        cx.background_executor().timer(delay).await;
                        let keep_going = this
                            .update(cx, |this, cx| {
                                cx.notify();
                                this.blink_epoch == epoch
                            })
                            .unwrap_or(false);
                        if !keep_going {
                            break;
                        }
                    }
                }));
                return;
            }
        }
        self._blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.blink_epoch != epoch {
                            return false;
                        }
                        this.cursor_visible = !this.cursor_visible;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }

    /// Each cursor's state in view coordinates (view line, view column;
    /// Monaco `CursorState.viewState`): the view state the last view-based
    /// move or click left, while it still maps to the cursor's model
    /// position (Monaco `validateViewPosition`), else the model position's
    /// view position. At a wrap point the two differ: End leaves the caret
    /// at the end of the first segment, where the model position would put
    /// it at the start of the next.
    pub(crate) fn cursor_view_states(&self) -> Vec<SingleCursorState> {
        let lines = &self.view_lines;
        let cached = self
            .cursor_views
            .as_ref()
            .filter(|views| views.model.as_slice() == self.cursors.all());
        let validate = |view: Option<PointUtf16>, model: PointUtf16| match view {
            Some(view) if lines.to_model(view.row, view.column) == model => view,
            _ => {
                let (line, column) = lines.to_view(model, false);
                PointUtf16::new(line, column)
            }
        };
        self.cursors
            .all()
            .iter()
            .enumerate()
            .map(|(ix, state)| {
                let view = cached.map(|views| &views.view[ix]);
                SingleCursorState {
                    selection_start: validate(
                        view.map(|v| v.selection_start.start),
                        state.selection_start.start,
                    )
                        ..validate(
                            view.map(|v| v.selection_start.end),
                            state.selection_start.end,
                        ),
                    position: validate(view.map(|v| v.position), state.position),
                    ..state.clone()
                }
            })
            .collect()
    }

    /// Records the cursors' view states after setting their model states
    /// (Monaco `Cursor._setState` with a view state); `None`s take the
    /// model position's. Dropped if the cursors merged.
    pub(crate) fn set_cursor_view_states(&mut self, views: Vec<Option<SingleCursorState>>) {
        if views.len() != self.cursors.all().len() || views.iter().all(Option::is_none) {
            self.cursor_views = None;
            return;
        }
        let lines = &self.view_lines;
        let to_view = |p: PointUtf16| {
            let (line, column) = lines.to_view(p, false);
            PointUtf16::new(line, column)
        };
        let view = views
            .into_iter()
            .zip(self.cursors.all())
            .map(|(view, model)| view.unwrap_or_else(|| map_state(model, to_view)))
            .collect();
        self.cursor_views = Some(CursorViewStates {
            model: self.cursors.all().to_vec(),
            view,
        });
    }

    /// The view position of the cursors' heads (view line, view column).
    pub(crate) fn cursor_view_positions(&self) -> Vec<PointUtf16> {
        if self.cursor_views.is_none() {
            // The common case: straight from the model.
            return self
                .cursors
                .all()
                .iter()
                .map(|state| {
                    let (line, column) = self.view_lines.to_view(state.position, false);
                    PointUtf16::new(line, column)
                })
                .collect();
        }
        self.cursor_view_states()
            .into_iter()
            .map(|state| state.position)
            .collect()
    }

    // ----- navigation ---------------------------------------------------------

    pub(crate) fn move_cursors(
        &mut self,
        command: MoveCommand,
        select: bool,
        cx: &mut Context<Self>,
    ) {
        // Monaco `columnSelection`: Shift+arrows extend a column selection.
        if select && self.options.column_selection {
            let column = match command {
                MoveCommand::Left => Some(cursor::ColumnSelectCommand::Left),
                MoveCommand::Right => Some(cursor::ColumnSelectCommand::Right),
                MoveCommand::Up => Some(cursor::ColumnSelectCommand::Up),
                MoveCommand::Down => Some(cursor::ColumnSelectCommand::Down),
                MoveCommand::PageUp => Some(cursor::ColumnSelectCommand::PageUp),
                MoveCommand::PageDown => Some(cursor::ColumnSelectCommand::PageDown),
                _ => None,
            };
            if let Some(column) = column {
                self.column_select(column, cx);
                return;
            }
        }
        if !select && self.is_suggest_visible() {
            let handled = match command {
                MoveCommand::Up => Some(self.suggest_navigate(-1, cx)),
                MoveCommand::Down => Some(self.suggest_navigate(1, cx)),
                MoveCommand::PageUp => Some(self.suggest_navigate_page(false, cx)),
                MoveCommand::PageDown => Some(self.suggest_navigate_page(true, cx)),
                _ => None,
            };
            match handled {
                Some(true) => return,
                Some(false) => {}
                None => self.hide_suggest(cx),
            }
        }
        let config = self.cursor_config(cx);
        let page = self.page_lines();
        let before = self.cursors.selections();
        // Like Monaco, movement runs on the cursors' view states (wrapped
        // segments, folded rows skipped); edits run on the model.
        let lines = self.view_lines.clone();
        let to_model = |p: PointUtf16| lines.to_model(p.row, p.column);
        let view_states = self.cursor_view_states();
        let model = ViewCursorModel(&lines);
        // (model state, view state if the move set one).
        let moved: Vec<(SingleCursorState, Option<SingleCursorState>)> = match command {
            MoveCommand::Home | MoveCommand::End => {
                let rope = self.snapshot.rope();
                self.cursors
                    .all()
                    .iter()
                    .zip(&view_states)
                    .map(|(model_state, view_state)| {
                        let by_view = if command == MoveCommand::Home {
                            // Monaco `_moveToLineStart`: by view line unless
                            // on the first segment or at the view line's
                            // first non-whitespace column.
                            let first_segment =
                                view_state.position.column == model_state.position.column;
                            let first_non_blank = cursor::first_non_whitespace_column(
                                &model,
                                view_state.position.row,
                            )
                            .unwrap_or(0);
                            !first_segment && view_state.position.column != first_non_blank
                        } else {
                            // Monaco `_moveToLineEnd`: by view line unless at
                            // its end, or on the last segment.
                            let view_max = lines.line_max_column(view_state.position.row);
                            let model_max = rope.line_len_utf16(model_state.position.row);
                            view_state.position.column != view_max
                                && view_max - view_state.position.column
                                    != model_max - model_state.position.column
                        };
                        let (cursor_model, state): (&dyn cursor::CursorModel, _) = if by_view {
                            (&model, view_state)
                        } else {
                            (rope, model_state)
                        };
                        let moved = if command == MoveCommand::Home {
                            cursor::move_to_beginning_of_line(cursor_model, state, select)
                        } else {
                            cursor::move_to_end_of_line(cursor_model, state, select, false)
                        };
                        if by_view {
                            (map_state(&moved, to_model), Some(moved))
                        } else {
                            (moved, None)
                        }
                    })
                    .collect()
            }
            // Monaco `MoveOperations.moveLeft`/`moveRight` on the view model:
            // positions are normalized (`normalizePosition`) so a step over
            // a wrap point, where two view positions are one model
            // position, moves the cursor. Moving right never ends at a wrapped
            // segment's end.
            MoveCommand::Left | MoveCommand::Right => view_states
                .iter()
                .map(|view_state| {
                    let right = command == MoveCommand::Right;
                    let moved = if view_state.has_selection() && !select {
                        if right {
                            cursor::move_right(&config, &model, view_state, select, 1)
                        } else {
                            cursor::move_left(&config, &model, view_state, select, 1)
                        }
                    } else {
                        // Monaco normalizes just the position it moves
                        // from; the selection start stays (and is the same
                        // position when not selecting).
                        let position = lines.normalize_view_position(view_state.position, right);
                        let state = SingleCursorState {
                            selection_start: if select {
                                view_state.selection_start.clone()
                            } else {
                                position..position
                            },
                            position,
                            ..view_state.clone()
                        };
                        if right {
                            let moved = cursor::move_right(&config, &model, &state, select, 1);
                            let position = lines.normalize_view_position(moved.position, true);
                            state.move_to(select, position, 0)
                        } else {
                            cursor::move_left(&config, &model, &state, select, 1)
                        }
                    };
                    (map_state(&moved, to_model), Some(moved))
                })
                .collect(),
            // Monaco `cursorLineStart`/`cursorLineEnd` move on model lines.
            MoveCommand::LineStart | MoveCommand::LineEnd => {
                let mut model_cursors = self.cursors.clone();
                cursor::move_cursors(
                    &config,
                    self.snapshot.rope(),
                    &mut model_cursors,
                    command,
                    select,
                    page,
                );
                model_cursors
                    .all()
                    .iter()
                    .map(|state| (state.clone(), None))
                    .collect()
            }
            _ => {
                let mut view_cursors = self.cursors.clone();
                view_cursors.set_states(view_states);
                cursor::move_cursors(&config, &model, &mut view_cursors, command, select, page);
                view_cursors
                    .all()
                    .iter()
                    .map(|state| (map_state(state, to_model), Some(state.clone())))
                    .collect()
            }
        };
        let (model_states, view_states): (Vec<_>, Vec<_>) = moved.into_iter().unzip();
        self.cursors.set_states(model_states);
        self.cursors.validate(self.snapshot.rope());
        self.set_cursor_view_states(view_states);
        // Like Monaco, paging also scrolls the viewport by a page.
        if matches!(command, MoveCommand::PageUp | MoveCommand::PageDown)
            && let Some(layout) = self.layout.as_ref()
        {
            let delta = layout.metrics.line_height * page as f32;
            self.scroll_top = if command == MoveCommand::PageDown {
                self.scroll_top + delta
            } else {
                (self.scroll_top - delta).max(px(0.))
            };
        }
        self.prev_edit_kind = None;
        if self.cursors.selections() != before {
            self.model.update(cx, |model, _| model.push_stack_element());
        }
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        cursor::select_all(self.snapshot.rope(), &mut self.cursors);
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(false, cx);
    }

    fn expand_line_selection(
        &mut self,
        _: &ExpandLineSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cursor::expand_line_selection(self.snapshot.rope(), &mut self.cursors);
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
    }

    fn add_cursor_above(&mut self, _: &AddCursorAbove, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.cursors.selections();
        let config = self.cursor_config(cx);
        cursor::add_cursor_above(&config, self.snapshot.rope(), &mut self.cursors);
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
        self.announce_cursor_change(&before, cx);
    }

    fn add_cursor_below(&mut self, _: &AddCursorBelow, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.cursors.selections();
        let config = self.cursor_config(cx);
        cursor::add_cursor_below(&config, self.snapshot.rope(), &mut self.cursors);
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
        self.announce_cursor_change(&before, cx);
    }

    /// Escape while parameter hints show (Monaco `closeParameterHints`):
    /// Monaco weighs drag-and-drop, linked editing, the suggest widget and
    /// ghost text higher, so those go first.
    fn close_parameter_hints(
        &mut self,
        _: &CloseParameterHints,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dnd.active
            || self.linked_editing_active()
            || self.is_suggest_visible()
            || self.inline_suggestion_visible(cx)
            || self.inline_edit_visible(cx)
        {
            self.remove_secondary_cursors(&RemoveSecondaryCursors, window, cx);
        } else {
            self.hide_signature_help(cx);
        }
    }

    fn remove_secondary_cursors(
        &mut self,
        _: &RemoveSecondaryCursors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.escape(false, window, cx);
    }

    /// Escape's chain of Monaco commands, or with `secondary` Shift+Escape's
    /// (only the commands Monaco gives Shift+Escape as a secondary key).
    pub(crate) fn escape(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.dnd_cancel(cx) {
            // Monaco `onMouseDropCanceled`: Escape while dragging.
            cx.emit(CodeEditorEvent::MouseDropCanceled);
            return;
        }
        if self.options.single_line {
            cx.propagate();
            return;
        }
        if self.linked_editing_active() {
            self.clear_linked_editing(cx);
            return;
        }
        if self.is_suggest_visible() {
            self.hide_suggest(cx);
            return;
        }
        if !secondary && (self.inline_suggestion_visible(cx) || self.inline_edit_visible(cx)) {
            self.hide_inline_suggestion(cx);
            return;
        }
        if self.in_snippet_mode() {
            self.cancel_snippet(true, cx);
            return;
        }
        if self.close_marker_zone(cx) {
            return;
        }
        if self.is_signature_help_visible() {
            self.hide_signature_help(cx);
            return;
        }
        if !secondary && self.is_hover_visible() {
            self.hide_hover(cx);
            return;
        }
        if !secondary && self.is_message_visible() {
            self.close_message(cx);
            return;
        }
        // Monaco's `closeReferenceSearch` outranks closing find; its
        // Escape keybinding is off with `stablePeek`.
        if !self.options.stable_peek && self.close_peek(cx) {
            return;
        }
        if !secondary && self.cancel_symbol_navigation(cx) {
            return;
        }
        // Monaco's `closeFindWidget` and `cancelSelectionAnchor` outrank
        // `removeSecondaryCursors`.
        if self.find.visible {
            self.close_find(window, cx);
            return;
        }
        // Monaco `editor.cancelOperation` (weight `EditorContrib`) outranks
        // the core commands.
        if !secondary && self.cancel_operation(cx) {
            return;
        }
        if !secondary && self.cancel_selection_anchor(cx) {
            return;
        }
        let primary = self.cursors.primary().clone();
        if self.cursors.all().len() > 1 {
            self.cursors.set_states(vec![primary]);
            // Monaco `removeSecondaryCursors` says so.
            self.announce("Removed secondary cursors", cx);
        } else if primary.has_selection() {
            self.cursors
                .set_states(vec![SingleCursorState::from_position(primary.position)]);
        } else {
            cx.propagate();
            return;
        }
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
    }

    // ----- editing ------------------------------------------------------------

    /// Types `text` at every cursor, as if from the keyboard. Emits
    /// [`CodeEditorEvent::WillType`] and [`CodeEditorEvent::DidType`]
    /// around it (Monaco's `'keyboard'` source).
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let typed = gpui::SharedString::from(text.to_string());
        self.begin_update(cx);
        cx.emit(CodeEditorEvent::WillType(typed.clone()));
        self.type_text_inner(text, cx);
        cx.emit(CodeEditorEvent::DidType(typed));
        self.end_update(cx);
    }

    fn type_text_inner(&mut self, text: &str, cx: &mut Context<Self>) {
        let single_line;
        let text = if self.options.single_line && text.contains('\n') {
            single_line = text.replace(['\r', '\n'], " ");
            single_line.as_str()
        } else {
            text
        };
        if let Some(ch) = text.chars().next().filter(|_| text.chars().count() == 1) {
            self.commit_suggestion_on(ch, cx);
        }
        let prev = self.prev_edit_kind;
        let operation = self.with_language(cx, |config, rope, cursors, lang| {
            cursor::type_with_language(config, rope, cursors, text, prev, lang)
        });
        self.apply(operation, cx);
        self.suggest_after_typing(text, cx);
        self.signature_help_after_typing(text, cx);
        if let Some(ch) = text.chars().last() {
            self.format_on_type(ch, cx);
        }
    }

    /// Runs `f` with the model's language context (configuration plus token
    /// types from syntax highlighting), if it has a language.
    fn with_language<R>(
        &self,
        cx: &App,
        f: impl FnOnce(
            &CursorConfig,
            &text::Rope,
            &CursorCollection,
            Option<&cursor::LanguageContext>,
        ) -> R,
    ) -> R {
        let config = self.cursor_config(cx);
        let rope = self.snapshot.rope();
        let model = self.model.read(cx);
        match model.language() {
            Some(language) => {
                let token_type = |p: PointUtf16| {
                    if p.column == 0 {
                        return cursor::StandardTokenType::Other;
                    }
                    let offset = rope.point_utf16_to_offset(PointUtf16::new(p.row, p.column - 1));
                    model.token_type_at(offset)
                };
                let context = cursor::LanguageContext::new(&language.config, &token_type)
                    .with_max_tokenization_line_length(
                        model.options().max_tokenization_line_length,
                    );
                f(&config, rope, &self.cursors, Some(&context))
            }
            None => f(&config, rope, &self.cursors, None),
        }
    }

    fn run_edit(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&CursorConfig, &text::Rope, &CursorCollection) -> EditOperation,
    ) {
        let config = self.cursor_config(cx);
        let operation = f(&config, self.snapshot.rope(), &self.cursors);
        self.apply(operation, cx);
    }

    /// Like `run_edit` for commands that may have nothing to do.
    fn run_optional_edit(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&CursorConfig, &text::Rope, &CursorCollection) -> Option<EditOperation>,
    ) {
        let config = self.cursor_config(cx);
        if let Some(operation) = f(&config, self.snapshot.rope(), &self.cursors) {
            self.apply(operation, cx);
        }
    }

    /// Steps the number or value at the cursor up or down (Monaco
    /// `editor.action.inPlaceReplace.up` / `.down`), briefly highlighting it.
    pub fn in_place_replace(&mut self, up: bool, cx: &mut Context<Self>) {
        let config = self.cursor_config(cx);
        let word_pattern = self
            .model
            .read(cx)
            .language()
            .and_then(|l| l.config.word_pattern.clone());
        let Some((operation, highlight)) = cursor::in_place_replace(
            &config,
            self.snapshot.rope(),
            &self.cursors,
            word_pattern.as_ref(),
            up,
        ) else {
            return;
        };
        self.apply(operation, cx);
        // Monaco flashes the new value for 350ms.
        let rope = self.snapshot.rope();
        let range =
            rope.point_utf16_to_offset(highlight.start)..rope.point_utf16_to_offset(highlight.end);
        let options = crate::decorations::DecorationOptions {
            border: Some(self.theme.colors.input_option_active_border),
            ..Default::default()
        };
        let owner = Some(cx.entity_id());
        let ids = self.model.update(cx, |model, cx| {
            model.delta_decorations(owner, &[], vec![(range, options)], cx)
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            this.update(cx, |this, cx| {
                this.model.update(cx, |model, cx| {
                    model.delta_decorations(None, &ids, Vec::new(), cx)
                });
            })
            .ok();
        })
        .detach();
    }

    /// Applies a case transform to each selection or word (Monaco's
    /// `editor.action.transformTo*`).
    pub fn transform_case(&mut self, transform: cursor::CaseTransform, cx: &mut Context<Self>) {
        self.run_edit(cx, |c, m, s| cursor::transform_case(c, m, s, transform));
    }

    fn delete_left(&mut self, _: &DeleteLeft, _: &mut Window, cx: &mut Context<Self>) {
        let prev = self.prev_edit_kind;
        let operation = self.with_language(cx, |config, rope, cursors, lang| {
            cursor::delete_left_with_language(config, rope, cursors, prev, lang)
        });
        self.apply(operation, cx);
        self.trigger_inline_completions(cx);
        if self.suggest.session.is_some() {
            self.refilter_suggest(cx);
        }
        if self.is_signature_help_visible() {
            self.trigger_signature_help(crate::providers::SignatureHelpTrigger::ContentChange, cx);
        }
    }

    fn delete_right(&mut self, _: &DeleteRight, _: &mut Window, cx: &mut Context<Self>) {
        let prev = self.prev_edit_kind;
        self.run_edit(cx, |c, m, s| cursor::delete_right(c, m, s, prev));
        self.trigger_inline_completions(cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_word(true, cursor::WordNavigationType::WordStart, true, cx);
    }

    /// Monaco's `DeleteWordCommand` family; to the left, a cursor between an
    /// auto-closed pair (`(|)`) deletes both characters.
    fn delete_word(
        &mut self,
        left: bool,
        nav: cursor::WordNavigationType,
        whitespace_heuristics: bool,
        cx: &mut Context<Self>,
    ) {
        let operation = self.with_language(cx, |config, rope, cursors, lang| {
            cursor::delete_word_with_language(
                config,
                rope,
                cursors,
                left,
                nav,
                whitespace_heuristics,
                lang,
            )
        });
        self.apply(operation, cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::delete_word_right);
    }

    fn delete_all_left(&mut self, _: &DeleteAllLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::delete_all_left);
    }

    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        if self.options.single_line {
            cx.propagate();
            return;
        }
        if self.is_suggest_visible() && self.accept_suggestion(cx) {
            return;
        }
        // Monaco `inlineSuggest.commit` (it outranks snippets and tab
        // completion).
        if self.tab_accepts_inline_suggestion(cx) && self.accept_inline_suggestion(cx) {
            return;
        }
        // Monaco `inlineSuggest.jump` (weighed above accepting) and
        // `inlineSuggest.commit` for inline edits.
        if self.tab_should_jump_to_inline_edit(cx) {
            self.jump_to_next_inline_edit(cx);
            return;
        }
        if self.tab_should_accept_inline_edit(cx) && self.accept_inline_suggestion(cx) {
            return;
        }
        if self.has_next_tabstop() {
            self.next_snippet_placeholder(cx);
            return;
        }
        // Monaco `tabCompletion`: `insertNextSuggestion`, then
        // `insertBestCompletion`.
        if self.options.tab_completion == crate::options::TabCompletion::On {
            if self.has_other_suggestions() {
                self.insert_other_suggestion(true, cx);
                return;
            }
            if self.insert_best_completion(cx) {
                return;
            }
        }
        self.insert_tab(cx);
        self.trigger_inline_completions(cx);
    }

    /// Monaco's `tab` command: indents or inserts a tab.
    pub(crate) fn insert_tab(&mut self, cx: &mut Context<Self>) {
        let operation = self.with_language(cx, |config, rope, cursors, lang| {
            cursor::tab_with_language(config, rope, cursors, lang)
        });
        self.apply(operation, cx);
    }

    /// Inserts a line break at the end of the document (Monaco
    /// `editor.action.insertFinalNewLine`, which inserts one even when the
    /// last line is empty). The primary selection stays where it is; other
    /// cursors are dropped, as in Monaco.
    pub fn insert_final_new_line(&mut self, cx: &mut Context<Self>) {
        if self.options.read_only {
            return;
        }
        let end = self.snapshot.rope().max_point_utf16();
        let selection = self.cursors.primary().selection();
        self.apply(
            EditOperation {
                // The model stores `\n`; `TextModel::text` joins lines with
                // its line ending.
                edits: vec![(end..end, "\n".to_string())],
                selections_after: vec![selection],
                kind: EditOperationType::Other,
                push_stack_element_before: true,
                push_stack_element_after: true,
                auto_closed: Vec::new(),
                follow_up: None,
                auto_whitespace: Vec::new(),
            },
            cx,
        );
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        if self.options.single_line {
            cx.propagate();
            return;
        }
        // Monaco `inlineSuggest.commitAlternativeAction` (Shift+Tab, weighed
        // above snippets and outdenting).
        if self.accept_inline_suggestion_alternative_action(cx) {
            return;
        }
        if self.has_prev_tabstop() {
            self.prev_snippet_placeholder(cx);
            return;
        }
        if self.options.tab_completion == crate::options::TabCompletion::On
            && self.has_other_suggestions()
        {
            self.insert_other_suggestion(false, cx);
            return;
        }
        self.run_edit(cx, cursor::outdent);
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if self.options.single_line {
            cx.propagate();
            return;
        }
        if self.is_suggest_visible() && self.accept_suggestion_on_enter(cx) {
            return;
        }
        let operation = self.with_language(cx, |config, rope, cursors, lang| {
            cursor::line_break_with_language(config, rope, cursors, lang)
        });
        self.apply(operation, cx);
        // Monaco: typing on another line closes the suggestions.
        if self.suggest.session.is_some() {
            self.refilter_suggest(cx);
        }
    }

    fn indent_lines(&mut self, _: &IndentLines, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::indent_lines);
    }

    fn outdent_lines(&mut self, _: &OutdentLines, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::outdent_lines);
    }

    fn move_lines_up(&mut self, _: &MoveLinesUp, _: &mut Window, cx: &mut Context<Self>) {
        let operation = self.with_language(cx, cursor::move_lines_up_with_language);
        self.apply(operation, cx);
    }

    fn move_lines_down(&mut self, _: &MoveLinesDown, _: &mut Window, cx: &mut Context<Self>) {
        let operation = self.with_language(cx, cursor::move_lines_down_with_language);
        self.apply(operation, cx);
    }

    fn copy_lines_up(&mut self, _: &CopyLinesUp, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::copy_lines_up);
    }

    fn copy_lines_down(&mut self, _: &CopyLinesDown, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::copy_lines_down);
    }

    /// Keyboard column selection (Monaco `cursorColumnSelect*`).
    fn column_select(&mut self, command: cursor::ColumnSelectCommand, cx: &mut Context<Self>) {
        let config = self.cursor_config(cx);
        let page = self.page_lines();
        let rope = self.snapshot.rope().clone();
        cursor::column_select(&config, &rope, &mut self.cursors, command, page);
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
    }

    /// Runs a comment command with the comment tokens of the language at
    /// each position.
    fn comment(&mut self, kind: Option<cursor::LineCommentType>, cx: &mut Context<Self>) {
        let config = self.cursor_config(cx);
        let options = self.options.comments;
        let model = self.model.read(cx);
        let rope = self.snapshot.rope();
        let comments_at = |p: PointUtf16| {
            model
                .language_at(rope.point_utf16_to_offset(p))
                .map(|language| language.config.comments().clone())
        };
        let operation = match kind {
            Some(kind) => {
                cursor::line_comment(&config, rope, &self.cursors, &comments_at, kind, options)
            }
            None => cursor::block_comment(&config, rope, &self.cursors, &comments_at, options),
        };
        self.apply(operation, cx);
    }

    fn delete_lines(&mut self, _: &DeleteLines, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::delete_lines);
    }

    fn insert_line_after(&mut self, _: &InsertLineAfter, _: &mut Window, cx: &mut Context<Self>) {
        // Single-line inputs (Monaco's `<input>`s) have no lines to add.
        if self.options.single_line {
            return;
        }
        self.run_edit(cx, cursor::insert_line_below);
    }

    fn insert_line_before(&mut self, _: &InsertLineBefore, _: &mut Window, cx: &mut Context<Self>) {
        if self.options.single_line {
            return;
        }
        self.run_edit(cx, cursor::insert_line_above);
    }

    /// Undoes the model's last undo step (Monaco `undo`), restoring the cursors it
    /// was made with.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if self.options.read_only {
            return;
        }
        let source = Some(cx.entity_id());
        self.prev_edit_kind = None;
        self.begin_update(cx);
        self.model.update(cx, |model, cx| model.undo(source, cx));
        self.end_update(cx);
    }

    /// Redoes the model's last undone step (Monaco `redo`), restoring the cursors
    /// from after it.
    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if self.options.read_only {
            return;
        }
        let source = Some(cx.entity_id());
        self.prev_edit_kind = None;
        self.begin_update(cx);
        self.model.update(cx, |model, cx| model.redo(source, cx));
        self.end_update(cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.write_selection_to_clipboard(false, cx);
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if self.write_selection_to_clipboard(true, cx) {
            self.run_edit(cx, cursor::cut);
        }
    }

    fn write_selection_to_clipboard(&mut self, is_cut: bool, cx: &mut Context<Self>) -> bool {
        let config = self.cursor_config(cx);
        let (text, is_from_empty_selection, per_cursor) =
            cursor::copy_text(&config, self.snapshot.rope(), &self.cursors);
        if text.is_empty() && !is_from_empty_selection {
            return false;
        }
        let multicursor_text = (per_cursor.len() > 1).then_some(per_cursor);
        let mode = self.model.read(cx).language_id().cloned();
        // Monaco `onWillCopy`/`onWillCut`, before the clipboard is written.
        let event = ClipboardCopyEvent {
            is_cut,
            text: text.clone(),
            is_from_empty_selection,
            multicursor_text: multicursor_text.clone(),
            language_id: mode.clone(),
        };
        cx.emit(if is_cut {
            CodeEditorEvent::WillCut(event)
        } else {
            CodeEditorEvent::WillCopy(event)
        });
        // Monaco `CopyPasteController.handleCopy`: paste providers prepare
        // data for pasting this text.
        let (id, provider_copy_mime_types) = self.prepare_document_paste(cx);
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            text,
            ClipboardMetadata {
                is_from_empty_selection,
                multicursor_text,
                mode: mode.map(|mode| mode.to_string()),
                id,
                provider_copy_mime_types,
            },
        ));
        true
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        self.paste_item(item, cx);
    }

    /// The editor metadata stored with clipboard text copied from an editor.
    pub(crate) fn clipboard_metadata(item: &ClipboardItem) -> Option<ClipboardMetadata> {
        item.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::String(string) => string.metadata_json::<ClipboardMetadata>(),
            _ => None,
        })
    }

    /// The editor's own paste, when paste edit providers leave it to it.
    pub(crate) fn paste_item_for_providers(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
        self.paste_item_inner(item, cx);
        self.trigger_inline_completions(cx);
    }

    fn paste_item(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
        self.begin_update(cx);
        self.paste_item_outer(item, cx);
        self.end_update(cx);
    }

    fn paste_item_outer(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
        // Monaco `onWillPaste`, before anything handles it.
        let metadata = Self::clipboard_metadata(&item);
        cx.emit(CodeEditorEvent::WillPaste(ClipboardPasteEvent {
            text: item.text().unwrap_or_default(),
            is_from_empty_selection: metadata
                .as_ref()
                .is_some_and(|meta| meta.is_from_empty_selection),
            language_id: metadata
                .as_ref()
                .and_then(|meta| meta.mode.clone())
                .map(Into::into),
            multicursor_text: metadata.and_then(|meta| meta.multicursor_text),
        }));
        // Monaco `CopyPasteController`: providers may handle it.
        let data = crate::drop_paste::clipboard_data(&item);
        if self.paste_with_providers(data, item.clone(), cx) {
            return;
        }
        self.paste_item_inner(item, cx);
        self.trigger_inline_completions(cx);
    }

    pub(crate) fn paste_item_inner(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
        let version = self.snapshot.version();
        let primary_start = self.cursors.primary().selection().start();
        let start = self
            .cursors
            .selections()
            .iter()
            .map(|s| s.start())
            .min()
            .unwrap_or_default();
        let language_id = Self::clipboard_metadata(&item)
            .and_then(|meta| meta.mode)
            .map(Into::into);
        self.next_selection_change.reason = CursorChangeReason::Paste;
        self.paste_clipboard_item(item, cx);
        // Nothing may have changed.
        self.next_selection_change = CursorChangeCause::default();
        if self.snapshot.version() != version {
            // Monaco `onDidPaste`: from the primary selection's start before
            // to its start after.
            cx.emit(CodeEditorEvent::DidPaste(PasteEvent {
                range: primary_start..self.cursors.primary().selection().start(),
                language_id,
            }));
            // Format from the first selection's start to the last cursor,
            // which covers every cursor's pasted text.
            let end = self
                .cursors
                .selections()
                .iter()
                .map(|s| s.head)
                .max()
                .unwrap_or(start);
            self.format_on_paste(start..end, cx);
            // Monaco `AutoIndentOnPaste`: from the primary selection's
            // start before the paste to its start after.
            if self.options.auto_indent_on_paste {
                let range = primary_start..self.cursors.primary().selection().start();
                let within_string = self.options.auto_indent_on_paste_within_string;
                let operation = self.with_language(cx, |config, rope, cursors, lang| {
                    cursor::auto_indent_on_paste(config, rope, cursors, range, lang?, within_string)
                });
                if let Some(operation) = operation {
                    self.apply(operation, cx);
                }
            }
        }
    }

    fn paste_clipboard_item(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
        let Some(mut text) = item.text() else {
            return;
        };
        if self.options.single_line {
            text = text
                .trim_end_matches(['\r', '\n'])
                .replace(['\r', '\n'], " ");
        }
        let (from_empty, multicursor) = match Self::clipboard_metadata(&item) {
            Some(meta) => (meta.is_from_empty_selection, meta.multicursor_text),
            None => (false, None),
        };
        self.run_edit(cx, |c, m, s| {
            cursor::paste(c, m, s, &text, from_empty, multicursor.as_deref())
        });
    }

    // ----- mouse ----------------------------------------------------------------

    /// The buffer position under a window point, clamped to the text.
    pub(crate) fn position_for_point(
        &self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &App,
    ) -> PointUtf16 {
        let Some(layout) = self.layout.as_ref() else {
            return PointUtf16::zero();
        };
        let line_height = layout.metrics.line_height;
        let y = position.y - layout.bounds.top() + layout.scroll_top;
        if y < px(0.) {
            return PointUtf16::zero();
        }
        let count = self.view_lines.line_count();
        if y >= layout.zones.line_y(count, line_height) {
            return self.snapshot.max_point_utf16();
        }
        let view_line = layout.zones.line_at(y, line_height, count);
        let x = position.x - layout.text_origin_x();
        match layout.view_line(view_line) {
            Some(line) => PointUtf16::new(line.row, line.column_for_x(x)),
            None => {
                let segment = row_segment(&self.view_lines, view_line);
                let tab_size = self.cursor_config(cx).tab_size;
                let line = layout_segment(
                    self.snapshot.rope(),
                    &segment,
                    tab_size,
                    &layout.metrics,
                    self.theme.colors.foreground,
                    &[],
                    &[],
                    None,
                    window,
                );
                PointUtf16::new(line.row, line.column_for_x(x))
            }
        }
    }

    /// Brings the view lines up to date with the text, wrap settings and
    /// folds, and (re)computes fold ranges in the background when stale.
    /// The mouse cursor over the text for `mouse_style` (Monaco's
    /// `mouse-default` and `mouse-copy` editor classes).
    pub(crate) fn text_mouse_cursor(&self) -> MouseCursor {
        match self.options.mouse_style {
            crate::options::MouseStyle::Text => MouseCursor::IBeam,
            crate::options::MouseStyle::Default => MouseCursor::Arrow,
            crate::options::MouseStyle::Copy => MouseCursor::DragCopy,
        }
    }

    /// The classifier for `word_wrap_break_before_characters` and
    /// `word_wrap_break_after_characters`, built when they change.
    pub(crate) fn wrap_classifier(&mut self) -> Arc<crate::view::line_breaks::WrapClassifier> {
        let before = &self.options.word_wrap_break_before_characters;
        let after = &self.options.word_wrap_break_after_characters;
        if let Some((b, a, classifier)) = &self.wrap_classifier
            && b == before
            && a == after
        {
            return classifier.clone();
        }
        let classifier = Arc::new(crate::view::line_breaks::WrapClassifier::new(before, after));
        self.wrap_classifier = Some((before.clone(), after.clone(), classifier.clone()));
        classifier
    }

    pub(crate) fn sync_view_lines(&mut self, wrap: Option<WrapSettings>, cx: &mut Context<Self>) {
        if self.view_lines.version() != self.snapshot.version() {
            // Monaco `onModelContentChanged`: the cursors' view states come
            // from their model states again.
            self.cursor_views = None;
            let model = self.model.clone();
            let buffer = model.read(cx).buffer();
            let patch = buffer.edits_since(self.view_lines.version());
            // Like Monaco, editing inside a collapsed region expands it.
            for edit in patch.edits() {
                let start = self.snapshot.offset_to_point(edit.new.start).row;
                let end = self.snapshot.offset_to_point(edit.new.end).row;
                self.folds.expand_intersecting(buffer, start..end + 1);
            }
            Arc::make_mut(&mut self.view_lines).apply_edits(
                self.snapshot.rope().clone(),
                self.snapshot.version(),
                &patch,
            );
        }
        // Only touch the view lines when something changed, so a clone held
        // elsewhere isn't copied every frame.
        if self.view_lines.wrap() != wrap.as_ref() {
            Arc::make_mut(&mut self.view_lines).set_wrap(wrap);
        }
        let mut hidden = if self.options.folding {
            self.folds.hidden_rows(self.model.read(cx).buffer())
        } else {
            Vec::new()
        };
        let row_count = self.snapshot.rope().line_count();
        hidden.extend(
            self.hidden_areas
                .iter()
                .flat_map(|(_, areas)| areas)
                .map(|r| r.start.min(row_count)..r.end.min(row_count)),
        );
        let hidden = normalize_hidden(hidden);
        if self.view_lines.hidden() != hidden {
            Arc::make_mut(&mut self.view_lines).set_hidden(hidden);
            // Monaco `HiddenAreasChangedEvent`, when the line mapping
            // changed.
            cx.emit(CodeEditorEvent::HiddenAreasChanged);
        }
        // Each new version restarts the wait (Monaco's `Delayer`).
        let version = self.snapshot.version();
        // Monaco doesn't fold models too large to tokenize.
        // `refresh_folding_ranges` asks again for the same text.
        let epoch = cx
            .try_global::<crate::providers::FoldingRangesEpoch>()
            .map_or(0, |e| e.0);
        if self.options.folding
            && !self.model.read(cx).is_too_large_for_tokenization()
            && ((self.folds.ranges_version() != Some(version)
                && self.fold_requested.as_ref() != Some(&version))
                || epoch != self.fold_epoch)
        {
            self.fold_epoch = epoch;
            self.fold_requested = Some(version);
            self.compute_fold_ranges(cx);
        }
    }

    /// Monaco's `FoldingController`: folding range providers, else the
    /// syntax tree's `folds.scm` ranges plus region markers, else
    /// indentation (`folding_strategy: Indentation` skips the first two).
    /// Once there are ranges, recomputing waits for a pause in edits, like
    /// Monaco's update scheduler.
    fn compute_fold_ranges(&mut self, cx: &mut Context<Self>) {
        // One computation at a time: huge documents take seconds each.
        if self.fold_computing {
            self.fold_rerun = true;
            return;
        }
        let delay = if self.folds.ranges_version().is_some() {
            FOLD_UPDATE_DELAY
        } else {
            Duration::ZERO
        };
        self.fold_task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok(inputs) = this.update(cx, |this, cx| {
                this.fold_computing = true;
                let strategy = this.options.folding_strategy;
                this.fold_inputs(strategy, cx)
            }) else {
                return;
            };
            let (snapshot, version, providers, compute) = inputs;
            let provided =
                collect_provided_folding_ranges(providers, snapshot.rope().line_count()).await;
            let ranges = cx
                .background_executor()
                .spawn(async move { compute(provided) })
                .await;
            this.update(cx, |this, cx| {
                let (ranges, has_types) = ranges;
                let model = this.model.clone();
                let first = this.folds.ranges_version().is_none();
                this.folds.set_has_types(has_types);
                this.folds
                    .set_ranges(ranges, version, model.read(cx).buffer());
                // Monaco `foldingImportsByDefault`: imports start folded.
                if first && this.options.folding_imports_by_default {
                    let buffer = model.read(cx).buffer().clone();
                    let imports: Vec<usize> = this
                        .folds
                        .ranges()
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| {
                            r.kind == Some(crate::providers::FoldingRangeKind::Imports)
                        })
                        .map(|(ix, _)| ix)
                        .collect();
                    this.folds.toggle(&buffer, imports);
                }
                this.fold_task = None;
                this.fold_computing = false;
                // A newer version is asked for again on the next frame.
                if std::mem::take(&mut this.fold_rerun) {
                    this.fold_requested = None;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// What computing fold ranges with `strategy` needs: the text, provider
    /// requests, and the rest of the computation for a background thread.
    /// Sticky scroll computes its own with folding off.
    #[allow(clippy::type_complexity)]
    pub(crate) fn fold_inputs(
        &mut self,
        strategy: crate::options::FoldingStrategy,
        cx: &mut Context<Self>,
    ) -> (
        BufferSnapshot,
        text::Version,
        Vec<Task<anyhow::Result<Option<Vec<crate::providers::FoldingRange>>>>>,
        Box<
            dyn FnOnce(
                    Option<Vec<FoldingRangeData>>,
                ) -> (Vec<crate::view::folding::FoldRange>, bool)
                + Send,
        >,
    ) {
        let snapshot = self.model.read(cx).snapshot();
        let version = snapshot.version();
        let model = self.model.read(cx);
        let tab_size = model.options().tab_size;
        let config = model
            .language()
            .map(|language| language.config.folding.clone());
        let auto = strategy == crate::options::FoldingStrategy::Auto;
        let limit = self.options.folding_maximum_regions;
        let context = model.provider_context();
        let syntax = if auto { model.syntax_snapshot() } else { None };
        // Monaco's `syntaxRangeProvider.ts` passes the same empty context.
        let folding_context = crate::providers::FoldingContext::default();
        let providers: Vec<_> = if auto {
            crate::providers::folding_range_providers(cx, self.provider_target(cx))
                .into_iter()
                .map(|provider| provider.provide_folding_ranges(&context, &folding_context, cx))
                .collect()
        } else {
            Vec::new()
        };
        let text = snapshot.clone();
        let compute = Box::new(move |provided: Option<Vec<FoldingRangeData>>| {
            let line_count = text.rope().line_count();
            let folding = config.flatten();
            let markers = folding.as_ref().and_then(|f| f.markers.as_ref());
            if let Some(data) = provided {
                return (sanitize_ranges(data, limit).0, true);
            }
            let syntax_ranges = syntax.and_then(|syntax| {
                syntax
                    .highlighter
                    .fold_ranges(&syntax.state, syntax.text.rope())
            });
            if let Some(folds) = syntax_ranges {
                let mut data: Vec<FoldingRangeData> = folds
                    .into_iter()
                    .filter(|(_, end)| *end < line_count)
                    .map(|(start, end)| FoldingRangeData {
                        start,
                        end,
                        rank: 0,
                        kind: None,
                        manual: false,
                    })
                    .collect();
                if let Some(markers) = markers {
                    data.extend(
                        marker_ranges(text.rope(), &markers.start, &markers.end)
                            .into_iter()
                            .map(|(start, end)| FoldingRangeData {
                                start,
                                end,
                                rank: 1,
                                kind: Some(crate::providers::FoldingRangeKind::Region),
                                manual: false,
                            }),
                    );
                }
                return (sanitize_ranges(data, limit).0, false);
            }
            let off_side = folding.as_ref().is_some_and(|f| f.off_side);
            let markers = markers.map(|m| (&m.start, &m.end));
            (
                compute_indent_ranges_limited(text.rope(), tab_size, off_side, markers, limit),
                false,
            )
        });
        (snapshot, version, providers, compute)
    }

    /// The current folding ranges, as of the last computation.
    pub fn folding_ranges(&self) -> Vec<crate::providers::FoldingRange> {
        self.folds
            .ranges()
            .iter()
            .map(|range| crate::providers::FoldingRange {
                start_row: range.start_row,
                end_row: range.end_row,
                kind: range.kind.clone(),
            })
            .collect()
    }

    /// The collapsed regions, as (header row, last hidden row).
    pub fn collapsed_folds(&self, cx: &App) -> Vec<(u32, u32)> {
        self.folds.collapsed_rows(self.model.read(cx).buffer())
    }

    fn emit_mouse(
        &self,
        position: Point<Pixels>,
        button: Option<MouseButton>,
        modifiers: gpui::Modifiers,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> crate::mouse_target::EditorMouseEvent {
        crate::mouse_target::EditorMouseEvent {
            target: self.mouse_target(position, window, cx),
            button,
            modifiers,
            click_count,
        }
    }

    /// Whether `position` is over an embedded view (a zone or widget),
    /// which handles its own clicks and focus.
    fn over_widget(&self, position: Point<Pixels>) -> bool {
        self.layout.as_ref().is_some_and(|layout| {
            layout
                .widget_bounds
                .iter()
                .any(|bounds| bounds.contains(&position))
        })
    }

    /// Whether the wheel at `position` is over an embedded view that keeps
    /// it from the editor: overlay and overflowing widgets and zone widgets
    /// (Monaco's, which aren't in the editor's scrollable element). Plain
    /// zones and content widgets scroll the editor, as in Monaco.
    fn wheel_over_widget(&self, position: Point<Pixels>) -> bool {
        self.layout.as_ref().is_some_and(|layout| {
            layout
                .widget_bounds
                .iter()
                .zip(&layout.widget_captures_wheel)
                .any(|(bounds, captures)| *captures && bounds.contains(&position))
        })
    }

    /// Monaco `scrollOnMiddleClick`: a middle click starts scrolling
    /// toward the mouse; another click ends it.
    fn on_middle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.end_middle_scroll(cx) {
            return;
        }
        // Widgets handle their own clicks: no autoscroll, focus or link
        // under them.
        if self.over_widget(event.position) {
            let mouse = self.emit_mouse(
                event.position,
                Some(event.button),
                event.modifiers,
                event.click_count,
                window,
                cx,
            );
            cx.emit(CodeEditorEvent::MouseDown(mouse));
            return;
        }
        cx.stop_propagation();
        if self.options.scroll_on_middle_click {
            self.animation.middle_scroll =
                Some(crate::view::animation::MiddleScroll::new(event.position));
            cx.notify();
            return;
        }
        window.focus(&self.focus_handle, cx);
        // Monaco `mouseMiddleClickAction`: a middle click on text opens a
        // link, or with `ctrlLeftClick` acts like a Cmd/Ctrl+click.
        if let Some(position) = self.text_position_under_mouse(event.position, window, cx) {
            use crate::options::MouseMiddleClickAction;
            let handled = match self.options.mouse_middle_click_action {
                MouseMiddleClickAction::Default => false,
                MouseMiddleClickAction::OpenLink => self.follow_link_at(position, cx),
                MouseMiddleClickAction::CtrlLeftClick => {
                    if !self.follow_link_at(position, cx) {
                        // Monaco's gesture runs go to definition there,
                        // quietly, like a Cmd/Ctrl+click.
                        self.clear_definition_link(cx);
                        self.set_selections(&[Selection::collapsed(position)], cx);
                        let open_in_peek =
                            self.options.definition_link_opens_in_peek && !self.in_peek;
                        self.navigate(
                            LocationKind::Definition,
                            crate::goto::Navigation {
                                open_in_peek,
                                mute_message: true,
                                ..Default::default()
                            },
                            cx,
                        );
                    }
                    true
                }
            };
            if handled {
                return;
            }
        }
        let position = self.position_for_point(event.position, window, cx);
        // Monaco `dispatchMouse`: with the selection clipboard (Linux) a
        // middle click is a click, and the platform pastes the primary
        // selection there; otherwise it column-selects.
        if self.selection_clipboard_on() {
            self.next_selection_change = CursorChangeCause {
                reason: CursorChangeReason::Explicit,
                source: CursorChangeSource::Mouse,
            };
            self.cursors
                .set_selections(&[Selection::collapsed(position)]);
            self.cursors_changed(false, cx);
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            if let Some(item) = cx.read_from_primary() {
                self.paste_item(item, cx);
            }
            return;
        }
        // Dragging extends it.
        let count = self.count_mouse_down(event.click_count, position, cx);
        self.mouse_selection = Some(MouseSelection {
            count,
            started_on_line_numbers: false,
            middle_button: true,
            modifiers: event.modifiers,
            on_injected_text: false,
        });
        self.dispatch_mouse(position, event.position, event.modifiers.shift, true, cx);
    }

    /// Monaco `MouseDownState.trySetCount` for a mouse down at `position`.
    fn count_mouse_down(&mut self, click_count: usize, position: PointUtf16, cx: &App) -> usize {
        let now = cx.background_executor().now();
        self.mouse_down_count
            .try_set_count(click_count.max(1), position, now)
    }

    /// Monaco `selectionClipboard` applies on Linux only.
    fn selection_clipboard_on(&self) -> bool {
        cfg!(any(target_os = "linux", target_os = "freebsd")) && self.options.selection_clipboard
    }

    /// The selection clipboard: selected text becomes the primary selection
    /// (as Monaco's textarea does on Linux).
    fn write_selection_to_primary(&mut self, cx: &mut Context<Self>) {
        if !self.selection_clipboard_on() {
            return;
        }
        let rope = self.snapshot.rope();
        let text: Vec<String> = self
            .cursors
            .selections()
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| {
                let range =
                    rope.point_utf16_to_offset(s.start())..rope.point_utf16_to_offset(s.end());
                rope.slice(range).to_string()
            })
            .collect();
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if !text.is_empty() {
            cx.write_to_primary(ClipboardItem::new_string(text.join("\n")));
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let _ = (text, cx);
    }

    /// Ends middle-click scrolling, if on.
    fn end_middle_scroll(&mut self, cx: &mut Context<Self>) -> bool {
        if self.animation.middle_scroll.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    fn on_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.end_middle_scroll(cx) {
            return;
        }
        // Monaco `_onContextMenu` ignores widgets (e.g. the find input's
        // own menu), and they keep focus: the editor doesn't take it, so
        // the quick input, rename box and menus stay open.
        let over_widget = self.over_widget(event.position);
        if !over_widget {
            window.focus(&self.focus_handle, cx);
        }
        let mouse = self.emit_mouse(
            event.position,
            Some(event.button),
            event.modifiers,
            event.click_count,
            window,
            cx,
        );
        cx.emit(CodeEditorEvent::MouseDown(mouse.clone()));
        if over_widget {
            cx.emit(CodeEditorEvent::ContextMenu(mouse));
            return;
        }
        self.long_lines_mouse_down(&mouse, cx);
        cx.emit(CodeEditorEvent::ContextMenu(mouse.clone()));
        // Monaco: only over text, and not on widgets.
        use crate::mouse_target::MouseTargetKind;
        if matches!(
            mouse.target.kind,
            MouseTargetKind::ContentText | MouseTargetKind::ContentEmpty { .. }
        ) {
            self.context_menu_mouse_down(mouse.target.position, event.position, window, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.end_middle_scroll(cx) {
            return;
        }
        let mouse = self.emit_mouse(
            event.position,
            Some(event.button),
            event.modifiers,
            event.click_count,
            window,
            cx,
        );
        cx.emit(CodeEditorEvent::MouseDown(mouse.clone()));
        self.long_lines_mouse_down(&mouse, cx);
        // Embedded views (zones and widgets) handle their own clicks and
        // focus.
        if self.over_widget(event.position) {
            return;
        }
        // Monaco `_onMouseDown`: the glyph margin and line decorations
        // (folding controls included) don't steal focus, nor do the line
        // numbers unless they select (`selectOnLineNumbers`).
        let keeps_focus = match mouse.target.kind {
            crate::mouse_target::MouseTargetKind::GutterGlyphMargin
            | crate::mouse_target::MouseTargetKind::GutterLineDecorations => true,
            crate::mouse_target::MouseTargetKind::GutterLineNumbers => {
                !self.options.select_on_line_numbers
            }
            _ => false,
        };
        if keeps_focus {
            // Also keeps GPUI from focusing the editor's root on the click.
            window.prevent_default();
        } else {
            window.focus(&self.focus_handle, cx);
        }
        let Some(layout) = self.layout.clone() else {
            return;
        };
        // The lightbulb opens its code actions (Monaco `onClick`).
        if event.button == MouseButton::Left
            && layout
                .lightbulb
                .is_some_and(|bulb| bulb.contains(&event.position))
        {
            self.show_lightbulb_menu(cx);
            return;
        }
        if let Some(minimap) = layout.minimap
            && minimap.interactive
            && minimap.bounds.contains(&event.position)
        {
            self.minimap_mouse_down(minimap, event, cx);
            return;
        }
        if let Some(ix) = layout.sticky_line_at(event.position) {
            self.sticky_mouse_down(ix, event);
            return;
        }
        for (vertical, bar) in [
            (true, layout.vertical_scrollbar),
            (false, layout.horizontal_scrollbar),
        ] {
            let Some(bar) = bar.filter(|b| b.interactive && b.bar.contains(&event.position)) else {
                continue;
            };
            // Monaco's arrows scroll a wheel step, repeating while held.
            if let Some(ix) = bar
                .arrows
                .and_then(|arrows| arrows.iter().position(|a| a.contains(&event.position)))
            {
                let sign = if ix == 0 { -1. } else { 1. };
                self.start_scrollbar_arrow(vertical, sign, cx);
                return;
            }
            let (along, thumb_start, thumb_len) = if vertical {
                (event.position.y, bar.thumb.top(), bar.thumb.size.height)
            } else {
                (event.position.x, bar.thumb.left(), bar.thumb.size.width)
            };
            let on_thumb = bar.thumb.contains(&event.position);
            // Monaco `scrollByPage`: the track pages towards the click by
            // the visible size (`getDesiredScrollPositionFromOffsetPaged`),
            // then drags from there (`_sliderPointerDown`).
            if !on_thumb && self.options.scrollbar.scroll_by_page {
                let page = if vertical {
                    layout.bounds.size.height
                } else {
                    layout.text_bounds.size.width
                };
                let delta = if along < thumb_start { -page } else { page };
                let track_start = if vertical {
                    self.scroll_top += delta;
                    bar.track.top()
                } else {
                    self.scroll_left += delta;
                    bar.track.left()
                };
                self.clamp_scroll();
                // Where the slider now is, so dragging moves it from there.
                let scrolled = if vertical {
                    self.scroll_top
                } else {
                    self.scroll_left
                };
                let thumb_start = track_start + scrolled / bar.ratio.max(f32::EPSILON);
                self.scrollbar_drag = Some(if vertical {
                    ScrollbarDrag::Vertical {
                        grab_offset: along - thumb_start,
                    }
                } else {
                    ScrollbarDrag::Horizontal {
                        grab_offset: along - thumb_start,
                    }
                });
                self.emit_scroll_changed(cx);
                cx.notify();
                return;
            }
            let grab_offset = if on_thumb {
                along - thumb_start
            } else {
                thumb_len / 2.
            };
            self.scrollbar_drag = Some(if vertical {
                ScrollbarDrag::Vertical { grab_offset }
            } else {
                ScrollbarDrag::Horizontal { grab_offset }
            });
            self.drag_scrollbar(event.position, cx);
            return;
        }

        if let Some(control) = layout
            .fold_controls
            .iter()
            .find(|control| control.bounds.contains(&event.position))
        {
            let recursive = event.modifiers.shift || event.button == MouseButton::Middle;
            self.click_fold(control.row, true, recursive, event.modifiers.alt, cx);
            return;
        }
        // The `⋯` after a collapsed line unfolds it, as does empty space
        // after the line with `unfold_on_click_after_end_of_line`.
        let recursive = event.modifiers.shift || event.button == MouseButton::Middle;
        if let Some((row, _)) = layout
            .fold_placeholders
            .iter()
            .find(|(_, bounds)| bounds.contains(&event.position))
        {
            self.click_fold(*row, false, recursive, event.modifiers.alt, cx);
            return;
        }
        // Clicking a color swatch opens the color picker.
        if event.button == MouseButton::Left
            && let Some(position) = self.swatch_at(event.position)
        {
            self.show_color_picker_hover(position, cx);
        }
        if self.options.unfold_on_click_after_end_of_line
            && matches!(
                mouse.target.kind,
                crate::mouse_target::MouseTargetKind::ContentEmpty { after_lines: false }
            )
            && let Some(position) = mouse.target.position
            && self
                .folds
                .is_collapsed(self.model.read(cx).buffer(), position.row)
        {
            self.click_fold(position.row, false, recursive, event.modifiers.alt, cx);
            return;
        }

        // Monaco: Cmd/Ctrl+click on an inlay hint part opens its location or
        // runs its command.
        if event.button == MouseButton::Left
            && self.is_definition_modifier(&event.modifiers)
            && let Some(part) = self.inlay_hint_part_at(event.position)
            && self.inlay_hint_link_click(part, cx)
        {
            return;
        }
        let position = self.position_for_point(event.position, window, cx);
        // Monaco: Cmd/Ctrl+click follows a link, else a definition.
        if self.is_definition_modifier(&event.modifiers)
            && (self.follow_link_at(position, cx)
                || self.follow_definition_link(
                    position,
                    self.is_side_by_side_modifier(&event.modifiers),
                    cx,
                ))
        {
            return;
        }
        self.hide_suggest(cx);
        let count = self.count_mouse_down(event.click_count, position, cx);
        // Monaco: a click on the selection may start dragging it.
        if event.button == MouseButton::Left
            && self.dnd_mouse_down(mouse.target.kind, position, &event.modifiers, count)
        {
            return;
        }
        let in_gutter = event.position.x < layout.text_bounds.left();
        // Monaco `_onMouseDown`: in the gutter, only the line numbers select
        // (with `selectOnLineNumbers`).
        if in_gutter
            && !(self.options.select_on_line_numbers
                && mouse.target.kind == crate::mouse_target::MouseTargetKind::GutterLineNumbers)
        {
            return;
        }
        // Monaco `MouseDownOperation.start`: the gesture, dispatched now
        // (extending the selection with Shift) and on every drag step.
        self.mouse_selection = Some(MouseSelection {
            count,
            started_on_line_numbers: in_gutter,
            middle_button: false,
            modifiers: event.modifiers,
            on_injected_text: self.inlay_hint_part_at(event.position).is_some(),
        });
        self.prev_edit_kind = None;
        self.dispatch_mouse(position, event.position, event.modifiers.shift, true, cx);
    }

    /// Monaco `_hasNonMulticursorModifier`: a modifier other than the one
    /// that adds cursors.
    fn has_non_multicursor_modifier(&self, modifiers: &gpui::Modifiers) -> bool {
        match self.options.multi_cursor_modifier {
            crate::options::MultiCursorModifier::Alt => modifiers.control || modifiers.platform,
            crate::options::MultiCursorModifier::CtrlCmd => {
                if cfg!(target_os = "macos") {
                    modifiers.control || modifiers.alt
                } else {
                    modifiers.alt || modifiers.platform
                }
            }
        }
    }

    /// Monaco `ViewController.dispatchMouse`: what the mouse gesture in
    /// [`Self::mouse_selection`] does at `position` (`mouse` in window
    /// coordinates), extending the selection when `in_selection_mode` (a
    /// Shift click, or a drag step). With `reveal`, the cursor is revealed
    /// minimally (Monaco `NavigationCommandRevealType.Minimal`; drag
    /// scrolling passes `None`).
    ///
    /// - The middle button column-selects.
    /// - From the line numbers: lines (with the multi-cursor modifier, a
    ///   new cursor on the line).
    /// - Four clicks select all; three, the line; two, the word (or with
    ///   `doubleClickSelectsBlock`, the bracket or string content). With
    ///   the multi-cursor modifier these act on the last added cursor.
    /// - One click moves the cursor (Shift: extends the selection by
    ///   characters, words or lines, as it was started). The multi-cursor
    ///   modifier adds a cursor or removes the one there, and with Shift
    ///   column-selects; so does Alt-drag when Ctrl/Cmd adds cursors, and
    ///   any drag with `columnSelection`.
    fn dispatch_mouse(
        &mut self,
        position: PointUtf16,
        mouse: Point<Pixels>,
        in_selection_mode: bool,
        reveal: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.mouse_selection.as_ref() else {
            return;
        };
        let (count, on_line_numbers, middle, modifiers, on_injected_text) = (
            state.count,
            state.started_on_line_numbers,
            state.middle_button,
            state.modifiers,
            state.on_injected_text,
        );
        let multi = self.is_add_cursor_modifier(&modifiers);
        let config = self.cursor_config(cx);
        let model: &dyn cursor::CursorModel = self.snapshot.rope();
        let mut states = self.cursors.all().to_vec();
        let last = self.cursors.last_added_cursor_index();
        // What to reveal (Monaco: the single-cursor commands reveal, the
        // multi-cursor ones don't) and which cursor stays on a wrapped
        // segment's end.
        let mut reveal_kind: Option<bool> = None;
        let mut keep_on_wrapped = None;
        let column = |this: &mut Self, extend: bool| {
            let visible_column = this.mouse_visible_column(mouse);
            cursor::column_select_to(
                &config,
                this.snapshot.rope(),
                &mut this.cursors,
                position,
                visible_column,
                extend,
            );
        };
        enum Next {
            States(Vec<SingleCursorState>),
            Column(bool),
            SelectAll,
            Nothing,
        }
        let next = if middle && !self.selection_clipboard_on() {
            Next::Column(in_selection_mode)
        } else if on_line_numbers {
            if multi {
                if in_selection_mode {
                    states[last] = cursor::line_state(model, &states[last], position, false);
                    Next::States(states)
                } else {
                    let state = cursor::line_state(model, &states[0], position, false);
                    Next::States(create_cursor(states, state))
                }
            } else {
                reveal_kind = Some(false);
                Next::States(vec![cursor::line_state(
                    model,
                    &states[0],
                    position,
                    in_selection_mode,
                )])
            }
        } else if count >= 4 {
            Next::SelectAll
        } else if count == 3 {
            if multi {
                states[last] =
                    cursor::line_state(model, &states[last], position, in_selection_mode);
                Next::States(states)
            } else {
                reveal_kind = Some(false);
                Next::States(vec![cursor::line_state(
                    model,
                    &states[0],
                    position,
                    in_selection_mode,
                )])
            }
        } else if count == 2 {
            if on_injected_text {
                Next::Nothing
            } else if multi {
                let select = states[last].has_selection();
                states[last] = cursor::word_state(&config, model, &states[last], position, select);
                Next::States(states)
            } else if in_selection_mode {
                reveal_kind = Some(true);
                Next::States(vec![cursor::word_state(
                    &config, model, &states[0], position, true,
                )])
            } else if let Some(block) = self.block_selection_at(position, cx) {
                Next::States(vec![SingleCursorState::from_selection(block)])
            } else {
                reveal_kind = Some(true);
                Next::States(vec![cursor::word_state(
                    &config, model, &states[0], position, false,
                )])
            }
        } else if multi {
            if self.has_non_multicursor_modifier(&modifiers) {
                Next::Nothing
            } else if modifiers.shift {
                Next::Column(true)
            } else if in_selection_mode {
                states[last] = cursor::move_to_state(&config, model, &states[last], position, true);
                keep_on_wrapped = Some(last);
                Next::States(states)
            } else {
                let state = cursor::move_to_state(&config, model, &states[0], position, false);
                let states = create_cursor(states, state);
                keep_on_wrapped = Some(states.len() - 1);
                Next::States(states)
            }
        } else if in_selection_mode && (modifiers.alt || self.options.column_selection) {
            Next::Column(true)
        } else {
            reveal_kind = Some(true);
            keep_on_wrapped = Some(0);
            Next::States(vec![cursor::move_to_state(
                &config,
                model,
                &states[0],
                position,
                in_selection_mode,
            )])
        };
        match next {
            Next::States(states) => self.cursors.validated_states(model, states),
            Next::Column(extend) => column(self, extend),
            Next::SelectAll => cursor::select_all(self.snapshot.rope(), &mut self.cursors),
            Next::Nothing => return,
        }
        if let Some(index) = keep_on_wrapped
            && index < self.cursors.all().len()
        {
            self.keep_mouse_on_wrapped_line(index, mouse);
        }
        self.model.update(cx, |model, _| model.push_stack_element());
        self.next_selection_change = CursorChangeCause {
            reason: CursorChangeReason::Explicit,
            source: CursorChangeSource::Mouse,
        };
        let reveal_kind = reveal_kind.filter(|_| reveal);
        self.cursors_changed(reveal_kind.is_some(), cx);
        if let Some(horizontal) = reveal_kind {
            self.autoscroll_minimal = true;
            self.autoscroll_vertical_only = !horizontal;
        }
    }

    /// Monaco's mouse targets are view positions: a click past the end of a
    /// wrapped segment puts cursor `index` at the end of that view line, not
    /// the start of the next one (the same model position).
    fn keep_mouse_on_wrapped_line(&mut self, index: usize, mouse: Point<Pixels>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let y = mouse.y - layout.bounds.top() + layout.scroll_top;
        if y < px(0.) {
            return;
        }
        let count = self.view_lines.line_count();
        let view_line = layout.zones.line_at(y, layout.metrics.line_height, count);
        let Some(line) = layout.view_line(view_line) else {
            return;
        };
        if line.is_last_segment
            || line.column_for_x(mouse.x - layout.text_origin_x()) != line.columns.end
        {
            return;
        }
        let view = PointUtf16::new(view_line, self.view_lines.line_max_column(view_line));
        let mut views = self.cursor_view_states();
        let Some(state) = views.get_mut(index) else {
            return;
        };
        if self.view_lines.to_model(view.row, view.column) != self.cursors.all()[index].position {
            return;
        }
        state.position = view;
        if self.cursors.all()[index].selection_start.is_empty()
            && self.cursors.all()[index].selection_start.start == self.cursors.all()[index].position
        {
            state.selection_start = view..view;
        }
        self.set_cursor_view_states(views.into_iter().map(Some).collect());
    }

    /// The text position under the mouse, or `None` when the mouse isn't
    /// over text (gutter, past the end of a line, scrollbars).
    fn text_position_under_mouse(
        &self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &App,
    ) -> Option<PointUtf16> {
        let layout = self.layout.as_ref()?;
        if !layout.text_bounds.contains(&position) {
            return None;
        }
        let point = self.position_for_point(position, window, cx);
        let line = layout.line_for_point(point)?;
        let x = position.x - layout.text_origin_x();
        (x <= line.width()).then_some(point)
    }

    /// Visible column under the mouse, past the end of text too (box
    /// selection).
    fn mouse_visible_column(&self, position: Point<Pixels>) -> u32 {
        self.layout.as_ref().map_or(0, |layout| {
            ((position.x - layout.text_origin_x()) / layout.metrics.em_width)
                .round()
                .max(0.) as u32
        })
    }

    /// Mouse moves over the editor or its overflowing widgets, and anywhere
    /// while dragging (see `EditorElement::paint`).
    pub(crate) fn editor_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sticky_mouse_move(Some(event.position), event.modifiers.shift, cx);
        if let Some(session) = self.animation.middle_scroll.as_mut() {
            session.mouse = event.position;
        }
        let over_thumb = |position: Option<Point<Pixels>>, layout: Option<&EditorLayout>| {
            layout.is_some_and(|l| {
                [l.vertical_scrollbar, l.horizontal_scrollbar]
                    .into_iter()
                    .flatten()
                    .any(|b| position.is_some_and(|p| b.thumb.contains(&p)))
            })
        };
        // The "Show more" chip's hover background.
        let over_chip = |position: Option<Point<Pixels>>, layout: Option<&EditorLayout>| {
            layout.is_some_and(|l| {
                position.is_some_and(|p| l.overflow_chips().any(|b| b.contains(&p)))
            })
        };
        let was_over = over_thumb(
            self.animation.scrollbars.mouse_position,
            self.layout.as_ref(),
        ) || over_chip(
            self.animation.scrollbars.mouse_position,
            self.layout.as_ref(),
        );
        self.animation.scrollbars.mouse_position = Some(event.position);
        if was_over
            != (over_thumb(Some(event.position), self.layout.as_ref())
                || over_chip(Some(event.position), self.layout.as_ref()))
        {
            cx.notify();
        }
        if self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.bounds.contains(&event.position))
        {
            let mouse = self.emit_mouse(event.position, None, event.modifiers, 0, window, cx);
            if let Some(position) = mouse.target.position {
                self.message_mouse_move(position.row, cx);
            }
            cx.emit(CodeEditorEvent::MouseMove(mouse));
        }
        let minimap_hovered = self
            .layout
            .as_ref()
            .and_then(|layout| layout.minimap)
            .is_some_and(|minimap| minimap.interactive && minimap.bounds.contains(&event.position));
        let minimap_slider_hovered = minimap_hovered
            && self
                .layout
                .as_ref()
                .and_then(|layout| layout.minimap)
                .is_some_and(|minimap| {
                    minimap.slider_needed && minimap.slider.contains(&event.position)
                });
        if minimap_hovered != self.minimap_hovered
            || minimap_slider_hovered != self.minimap_slider_hovered
        {
            self.minimap_hovered = minimap_hovered;
            self.minimap_slider_hovered = minimap_slider_hovered;
            cx.notify();
        }
        if self.minimap_mouse_move(event, cx) {
            return;
        }
        let gutter_hovered = self.layout.as_ref().is_some_and(|layout| {
            layout.bounds.contains(&event.position) && event.position.x < layout.text_bounds.left()
        });
        if gutter_hovered != self.gutter_hovered {
            self.gutter_hovered = gutter_hovered;
            cx.notify();
        }
        // The folding control under the mouse gets its tooltip on the next
        // frame.
        let fold_control_hovered = self.layout.as_ref().and_then(|layout| {
            layout
                .fold_controls
                .iter()
                .find(|control| control.bounds.contains(&event.position))
                .map(|control| control.row)
        });
        if fold_control_hovered != self.fold_control_hovered {
            self.fold_control_hovered = fold_control_hovered;
            cx.notify();
        }
        if event.pressed_button.is_none() && self.mouse_selection.is_none() {
            let over_hover = self
                .hover_card_bounds(cx)
                .is_some_and(|bounds| bounds.contains(&event.position));
            let over_widget = self.layout.as_ref().is_some_and(|layout| {
                layout
                    .widget_bounds
                    .iter()
                    .any(|bounds| bounds.contains(&event.position))
            });
            let hover_focused = self.is_hover_focused(window, cx);
            if over_hover {
                self.hover_mouse_move(
                    crate::hover::HoverTarget::None,
                    true,
                    hover_focused,
                    event.modifiers,
                    cx,
                );
            } else if !over_widget {
                let link_modifier = self.is_definition_modifier(&event.modifiers);
                // Monaco's inlay hint parts are their own mouse targets.
                let inlay_part = self.inlay_hint_part_at(event.position);
                self.inlay_hints_mouse_move(inlay_part, link_modifier, cx);
                // Over a sticky line, its own text (Monaco links its
                // definitions for Cmd/Ctrl+click, and shows no hovers).
                let over_sticky = self.layout.as_ref().and_then(|layout| {
                    layout
                        .sticky_line_at(event.position)
                        .map(|ix| layout.sticky_position(ix, event.position))
                });
                let position = match (inlay_part, over_sticky) {
                    (Some(_), _) => None,
                    (None, Some(sticky)) => sticky.filter(|_| link_modifier),
                    (None, None) => self.text_position_under_mouse(event.position, window, cx),
                };
                self.links_mouse_move(position, link_modifier, cx);
                if link_modifier && position.is_some() {
                    self.definition_mouse_move(position, cx);
                } else if self.has_definition_link() {
                    self.clear_definition_link(cx);
                }
                if let Some(part) = inlay_part {
                    self.hover_inlay_hint_part(part, cx);
                } else if over_sticky.is_some() {
                    self.hover_mouse_move(
                        crate::hover::HoverTarget::None,
                        false,
                        hover_focused,
                        event.modifiers,
                        cx,
                    );
                } else {
                    // Monaco's glyph hover: the glyph margin and line numbers.
                    let gutter_row = self.layout.as_ref().and_then(|layout| {
                        let gutter = layout.bounds.contains(&event.position)
                            && event.position.x
                                < layout.bounds.left()
                                    + layout.glyph_margin_width
                                    + layout.line_numbers_width;
                        gutter.then(|| self.position_for_point(event.position, window, cx).row)
                    });
                    let target = match (position, gutter_row) {
                        (_, Some(row)) => crate::hover::HoverTarget::Gutter(row),
                        (Some(position), None) => crate::hover::HoverTarget::Text(position),
                        (None, None) => crate::hover::HoverTarget::None,
                    };
                    self.hover_mouse_move(target, false, hover_focused, event.modifiers, cx);
                }
            }
        }
        if self.scrollbar_drag.is_some() {
            if event.pressed_button == Some(MouseButton::Left) {
                self.drag_scrollbar(event.position, cx);
            } else {
                self.scrollbar_drag = None;
            }
            return;
        }
        if self.dnd.active {
            if event.pressed_button == Some(MouseButton::Left) {
                // Monaco `onMouseDrag`.
                let mouse = self.emit_mouse(
                    event.position,
                    Some(MouseButton::Left),
                    event.modifiers,
                    1,
                    window,
                    cx,
                );
                cx.emit(CodeEditorEvent::MouseDrag(mouse));
                let position = self.position_for_point(event.position, window, cx);
                self.dnd_mouse_drag(Some(position), &event.modifiers, cx);
            } else {
                self.dnd_cancel(cx);
            }
            return;
        }
        let Some(drag) = self.mouse_selection.as_mut() else {
            return;
        };
        let button = if drag.middle_button {
            MouseButton::Middle
        } else {
            MouseButton::Left
        };
        if event.pressed_button != Some(button) {
            self.mouse_selection = None;
            self.drag_scroll = None;
            return;
        }
        // Monaco `_onMouseDownThenMove`: the modifiers follow the mouse;
        // outside the text, scroll towards the mouse every frame; inside,
        // dispatch the gesture to the mouse.
        drag.modifiers = event.modifiers;
        if let Some((outside, distance)) = self.drag_outside(event.position) {
            let row = self.position_for_point(event.position, window, cx).row;
            self.start_drag_scroll(outside, distance, event.position, row, window, cx);
            return;
        }
        self.drag_scroll = None;
        let position = self.position_for_point(event.position, window, cx);
        self.dispatch_mouse(position, event.position, true, true, cx);
    }

    /// Whether a mouse drag that started in the editor is going on, so it
    /// follows the mouse anywhere in the window (Monaco's global pointer
    /// monitor).
    pub(crate) fn is_mouse_dragging(&self) -> bool {
        self.mouse_selection.is_some()
            || self.scrollbar_drag.is_some()
            || self.minimap_drag.is_some()
            || self.dnd.active
    }

    /// Where `mouse` is outside the text while drag-selecting, and how far
    /// (Monaco `MouseDownOperation._getPositionOutsideEditor`): above or
    /// below the editor, else left of the text (over the gutter too) or
    /// right of it.
    fn drag_outside(&self, mouse: Point<Pixels>) -> Option<(DragOutside, Pixels)> {
        let layout = self.layout.as_ref()?;
        let editor = layout.bounds;
        let text = layout.text_bounds;
        if mouse.y < editor.top() {
            Some((DragOutside::Above, editor.top() - mouse.y))
        } else if mouse.y > editor.bottom() {
            Some((DragOutside::Below, mouse.y - editor.bottom()))
        } else if mouse.x <= text.left() {
            Some((DragOutside::Left, text.left() - mouse.x))
        } else if mouse.x >= text.right() {
            Some((DragOutside::Right, mouse.x - text.right()))
        } else {
            None
        }
    }

    /// Starts (or retargets) scrolling while drag-selecting outside the
    /// text (Monaco `TopBottomDragScrolling` and `LeftRightDragScrolling`):
    /// every frame it scrolls towards the mouse, faster the farther away it
    /// is, and extends the selection to the edge. `row` is the line under
    /// the mouse, for left and right.
    fn start_drag_scroll(
        &mut self,
        outside: DragOutside,
        distance: Pixels,
        mouse: Point<Pixels>,
        row: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(scroll) = self.drag_scroll.as_mut()
            && scroll.vertical() == outside.vertical()
        {
            scroll.outside = outside;
            scroll.distance = distance;
            scroll.mouse = mouse;
            scroll.row = row;
            return;
        }
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(DRAG_SCROLL_FRAME).await;
                let Ok(going) =
                    this.update_in(cx, |this, window, cx| this.drag_scroll_step(window, cx))
                else {
                    return;
                };
                if !going {
                    return;
                }
            }
        });
        self.drag_scroll = Some(DragScroll {
            outside,
            distance,
            mouse,
            row,
            last_time: cx.background_executor().now(),
            _task: task,
        });
    }

    /// One frame of drag scrolling (Monaco `DragScrollingOperation._execute`);
    /// whether it goes on.
    fn drag_scroll_step(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (Some(scroll), Some(layout)) = (self.drag_scroll.as_ref(), self.layout.as_ref()) else {
            return false;
        };
        if self.mouse_selection.is_none() {
            self.drag_scroll = None;
            return false;
        }
        let (outside, distance, mouse, row) =
            (scroll.outside, scroll.distance, scroll.mouse, scroll.row);
        // Monaco `_tick`: the time since the last frame, not the frame
        // length, so late timers don't slow it down.
        let now = cx.background_executor().now();
        let elapsed = now
            .saturating_duration_since(scroll.last_time)
            .as_secs_f32();
        // Monaco `_getScrollSpeed`: lines (or characters) per second.
        let speed = |viewport: f32, outside: f32| {
            if outside <= 1.5 {
                (viewport * (1. + outside)).max(30.)
            } else if outside <= 3. {
                (viewport * (2. + outside)).max(60.)
            } else {
                (viewport * (7. + outside)).max(200.)
            }
        };
        let text = layout.text_bounds;
        let before = (self.scroll_top, self.scroll_left);
        let position = if outside.vertical() {
            let line_height = layout.metrics.line_height;
            let lines = speed(
                layout.bounds.size.height / line_height,
                distance / line_height,
            );
            let delta = line_height * lines * elapsed;
            // The edge line, in the last frame's layout.
            let max = self.snapshot.max_point_utf16();
            if outside == DragOutside::Above {
                self.scroll_top -= delta;
                let top = self.position_for_point(point(text.left(), text.top()), window, cx);
                PointUtf16::new(top.row, 0)
            } else {
                self.scroll_top += delta;
                // At the mouse's x on the last visible line; the end of
                // the text once that's the last line.
                let edge = point(mouse.x.max(text.left()), text.bottom() - px(0.1));
                let bottom = self.position_for_point(edge, window, cx);
                if bottom.row >= max.row { max } else { bottom }
            }
        } else {
            // Monaco uses the font's full-width character width.
            let char_width = layout.metrics.em_width * 2.;
            let chars = speed(text.size.width / char_width, distance / char_width);
            let delta = char_width * chars * elapsed * 0.5;
            if outside == DragOutside::Left {
                self.scroll_left -= delta;
                PointUtf16::new(row, 0)
            } else {
                self.scroll_left += delta;
                PointUtf16::new(row, self.snapshot.rope().line_len_utf16(row))
            }
        };
        self.clamp_scroll();
        if before != (self.scroll_top, self.scroll_left) {
            self.emit_scroll_changed(cx);
        }
        if let Some(scroll) = self.drag_scroll.as_mut() {
            scroll.last_time = now;
        }
        // Monaco dispatches with `NavigationCommandRevealType.None`.
        self.dispatch_mouse(position, mouse, true, false, cx);
        cx.notify();
        true
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.scrollbar_arrow_task = None;
        self.sticky_mouse_up(event.position, event.button, event.modifiers, cx);
        let mut drop_target = None;
        if self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.bounds.contains(&event.position))
        {
            let mouse = self.emit_mouse(
                event.position,
                Some(event.button),
                event.modifiers,
                event.click_count,
                window,
                cx,
            );
            drop_target = mouse.target.position.map(|p| (mouse.target.kind, p));
            cx.emit(CodeEditorEvent::MouseUp(mouse));
        }
        if self.dnd.active && event.button == MouseButton::Left {
            // Monaco `onMouseDrop`.
            let mouse = self.emit_mouse(
                event.position,
                Some(event.button),
                event.modifiers,
                event.click_count,
                window,
                cx,
            );
            cx.emit(CodeEditorEvent::MouseDrop(mouse));
            self.dnd_mouse_drop(drop_target, &event.modifiers, cx);
        }
        // Monaco `_installDblClickGesture`: a double-click on an inlay hint
        // applies its text edits.
        if event.button == MouseButton::Left
            && event.click_count == 2
            && let Some(part) = self.inlay_hint_part_at(event.position)
        {
            self.inlay_hint_double_click(part, cx);
        }
        self.minimap_drag = None;
        self.mouse_selection = None;
        self.drag_scroll = None;
        self.scrollbar_drag = None;
    }

    /// Monaco `ScrollbarArrow`: one wheel step now, then repeating after
    /// 200ms at 24 per second until the button is released.
    fn start_scrollbar_arrow(&mut self, vertical: bool, sign: f32, cx: &mut Context<Self>) {
        let step = px(50.) * self.options.mouse_wheel_scroll_sensitivity * sign;
        let scroll = move |this: &mut Self, cx: &mut Context<Self>| {
            if vertical {
                this.scroll_top += step;
            } else {
                this.scroll_left += step;
            }
            this.clamp_scroll();
            this.emit_scroll_changed(cx);
            cx.notify();
        };
        scroll(self, cx);
        self.scrollbar_arrow_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            loop {
                if this.update(cx, |this, cx| scroll(this, cx)).is_err() {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(1000 / 24))
                    .await;
            }
        }));
    }

    fn drag_scrollbar(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (Some(drag), Some(layout)) = (self.scrollbar_drag, self.layout.as_ref()) else {
            return;
        };
        match drag {
            ScrollbarDrag::Vertical { grab_offset } => {
                if let Some(bar) = layout.vertical_scrollbar {
                    let thumb_top = position.y - grab_offset - bar.track.top();
                    self.scroll_top = (thumb_top * bar.ratio).max(px(0.));
                }
            }
            ScrollbarDrag::Horizontal { grab_offset } => {
                if let Some(bar) = layout.horizontal_scrollbar {
                    let thumb_left = position.x - grab_offset - bar.track.left();
                    self.scroll_left = (thumb_left * bar.ratio).max(px(0.));
                }
            }
        }
        self.emit_scroll_changed(cx);
        cx.notify();
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Widgets that keep the wheel: it scrolls them (GPUI's scrollable
        // elements don't stop it), never the editor.
        if self.wheel_over_widget(event.position) {
            cx.stop_propagation();
            return;
        }
        self.scroll_by_wheel(event, cx);
    }

    /// Scrolls by a wheel event, as over the text (Monaco
    /// `delegateScrollFromMouseWheelEvent`, e.g. for the diff editor's
    /// gutter).
    pub(crate) fn scroll_by_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let line_height = self
            .layout
            .as_ref()
            .map_or(self.options.resolved_line_height(), |l| {
                l.metrics.line_height
            });
        // Monaco `onMouseWheel`, before zooming or scrolling.
        cx.emit(CodeEditorEvent::MouseWheel(MouseWheelEvent {
            position: event.position,
            delta: event.delta.pixel_delta(line_height),
            lines: match event.delta {
                gpui::ScrollDelta::Lines(lines) => Some(lines),
                gpui::ScrollDelta::Pixels(_) => None,
            },
            modifiers: event.modifiers,
        }));
        if self.mouse_wheel_zoom(event, cx) {
            cx.stop_propagation();
            return;
        }
        if !self.options.scrollbar.handle_mouse_wheel {
            return;
        }
        // Monaco `fastScrollSensitivity` with Alt held.
        let sensitivity = if event.modifiers.alt {
            self.options.mouse_wheel_scroll_sensitivity * self.options.fast_scroll_sensitivity
        } else {
            self.options.mouse_wheel_scroll_sensitivity
        };
        let mut delta = event.delta.pixel_delta(line_height) * sensitivity;
        if event.modifiers.shift && delta.x == px(0.) {
            delta = point(delta.y, px(0.));
        }
        // Monaco `scrollPredominantAxis`: only the main direction.
        if self.options.scroll_predominant_axis {
            if delta.x.abs() >= delta.y.abs() {
                delta.y = px(0.);
            } else {
                delta.x = px(0.);
            }
        }
        // Physical wheels scroll smoothly (Monaco `mouseWheelSmoothScroll`),
        // trackpads don't.
        if matches!(event.delta, gpui::ScrollDelta::Lines(_)) {
            self.animation.smooth_scroll_requested = true;
        } else {
            self.animation.cancel_scroll();
        }
        self.hide_hover(cx);
        let before = (self.scroll_top, self.scroll_left);
        self.scroll_top -= delta.y;
        self.scroll_left -= delta.x;
        self.clamp_scroll();
        let moved = before != (self.scroll_top, self.scroll_left);
        // Monaco `inertialScroll`: keep scrolling, slowing down, after the
        // wheel events stop.
        if self.options.inertial_scroll && self.animation.inertial.on_wheel(delta) {
            self.run_inertial_scroll(cx);
        }
        self.emit_scroll_changed(cx);
        cx.notify();
        // Monaco `alwaysConsumeMouseWheel: false` lets the parent scroll
        // once the editor can't.
        if moved || self.options.scrollbar.always_consume_mouse_wheel {
            cx.stop_propagation();
        }
    }

    /// Monaco `AbstractScrollableElement._periodicSync`: scrolls by the
    /// inertial speed now and every 1/60s while it lasts.
    fn run_inertial_scroll(&mut self, cx: &mut Context<Self>) {
        if !self.inertial_scroll_step(cx) {
            return;
        }
        self.inertial_scroll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(crate::view::animation::INERTIAL_SCROLL_INTERVAL)
                    .await;
                let Ok(going) = this.update(cx, |this, cx| this.inertial_scroll_step(cx)) else {
                    return;
                };
                if !going {
                    break;
                }
            }
            this.update(cx, |this, _| this.inertial_scroll_task = None)
                .ok();
        }));
    }

    /// One inertial scroll step; whether another follows.
    fn inertial_scroll_step(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(delta) = self.animation.inertial.tick() else {
            return false;
        };
        let before = (self.scroll_top, self.scroll_left);
        self.scroll_top -= delta.y;
        self.scroll_left -= delta.x;
        self.clamp_scroll();
        if before != (self.scroll_top, self.scroll_left) {
            self.emit_scroll_changed(cx);
            cx.notify();
        }
        self.animation.inertial.is_moving()
    }

    fn stop_inertial_scroll(&mut self) {
        self.animation.inertial = Default::default();
        self.inertial_scroll_task = None;
    }

    // ----- UTF-16 helpers for the input handler ----------------------------------

    fn offset_from_utf16(&self, offset: usize) -> usize {
        self.snapshot.offset_utf16_to_offset(OffsetUtf16(offset))
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.snapshot.offset_to_offset_utf16(offset).0
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn primary_selection_offsets(&self) -> (Range<usize>, bool) {
        let selection = self.cursors.primary().selection();
        let start = self.snapshot.point_utf16_to_offset(selection.start());
        let end = self.snapshot.point_utf16_to_offset(selection.end());
        (start..end, selection.is_reversed())
    }

    /// Replaces `range` (bytes) with `text` and puts a single cursor after it.
    /// Monaco `ReplaceOvertypeCommandOnCompositionEnd`: a committed
    /// composition in overtype mode replaces as many characters after it as
    /// it inserted, on the same line.
    fn overtype_after_composition(&mut self, end: usize, text: &str, cx: &mut Context<Self>) {
        let rope = self.snapshot.rope();
        let line_end = rope.line_range(rope.offset_to_point(end).row).end;
        let units = text.encode_utf16().count();
        let mut overtyped = end;
        for ch in rope.chars_at(end).take(units) {
            if overtyped >= line_end {
                break;
            }
            overtyped += ch.len_utf8();
        }
        let overtyped = overtyped.min(line_end);
        if overtyped > end {
            self.replace_range(end..overtyped, "", cx);
        }
    }

    /// Switches between insert and overtype mode (Monaco
    /// `editor.action.toggleOvertypeInsertMode`, Insert).
    pub fn toggle_overtype(&mut self, cx: &mut Context<Self>) {
        self.overtype = !self.overtype;
        cx.notify();
    }

    /// Whether typing overtypes (Monaco's `overtype` input mode).
    pub fn is_overtype(&self) -> bool {
        self.overtype
    }

    fn replace_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let start = self.snapshot.offset_to_point_utf16(range.start);
        let end = self.snapshot.offset_to_point_utf16(range.end);
        let mut after = start;
        for ch in text.chars() {
            if ch == '\n' {
                after = PointUtf16::new(after.row + 1, 0);
            } else {
                after.column += ch.len_utf16() as u32;
            }
        }
        self.apply(
            EditOperation {
                edits: vec![(start..end, text.to_string())],
                selections_after: vec![Selection::collapsed(after)],
                kind: EditOperationType::Other,
                push_stack_element_before: true,
                push_stack_element_after: true,
                auto_closed: Vec::new(),
                follow_up: None,
                auto_whitespace: Vec::new(),
            },
            cx,
        );
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.snapshot.text_for_range(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let (range, reversed) = self.primary_selection_offsets();
        Some(UTF16Selection {
            range: self.range_to_utf16(&range),
            reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_selection = None;
        self.set_marked_range(None, cx);
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_item(item, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.options.read_only {
            self.read_only_edit_attempt(cx);
            return;
        }
        let from_composition = range_utf16.is_none() && self.marked_range.is_some();
        let marked = self.marked_range.take();
        self.marked_selection = None;
        let explicit = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(marked.clone());
        // Monaco commits a composition at every cursor.
        if marked.is_some()
            && self.cursors.all().len() > 1
            && let Some(range) = explicit.clone()
            && self
                .composition_type_at_cursors(range, text, 0, cx)
                .is_some()
        {
            cx.emit(CodeEditorEvent::CompositionEnded);
            return;
        }
        match explicit {
            Some(range) => {
                self.replace_range(range.clone(), text, cx);
                self.prev_edit_kind = None;
                if from_composition && self.overtype {
                    self.overtype_after_composition(range.start + text.len(), text, cx);
                }
            }
            None => self.type_text(text, cx),
        }
        // Committing the text ends the composition (Monaco
        // `compositionend`).
        if marked.is_some() {
            cx.emit(CodeEditorEvent::CompositionEnded);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.options.read_only {
            self.read_only_edit_attempt(cx);
            return;
        }
        let mut range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.primary_selection_offsets().0);
        // Monaco `compositionstart` comes before the composed text.
        let starting = self.marked_range.is_none() && !new_text.is_empty();
        if starting {
            cx.emit(CodeEditorEvent::CompositionStarted);
        }
        let was_marked = self.marked_range.is_some();
        // Monaco composes at every cursor (`compositionType`): the primary
        // cursor's text, typed at each of them. A composition starting over
        // selections first replaces them.
        let multi = self.cursors.all().len() > 1;
        if multi && starting && range == self.primary_selection_offsets().0 {
            let selections = self.cursors.selections();
            if selections.iter().any(|s| !s.is_empty()) {
                self.run_edit(cx, |c, m, s| cursor::type_text(c, m, s, "", None));
                range = self.primary_selection_offsets().0;
            }
        }
        let len_utf16 = new_text.encode_utf16().count();
        let composed = if multi {
            let delta = new_selected_range_utf16
                .as_ref()
                .map_or(0, |selected| selected.end as i64 - len_utf16 as i64);
            self.composition_type_at_cursors(range.clone(), new_text, delta as i32, cx)
        } else {
            None
        };
        let composed_at_cursors = composed.is_some();
        let composition = match composed {
            Some(composition) => composition,
            None => {
                self.replace_range(range.clone(), new_text, cx);
                range.start..range.start + new_text.len()
            }
        };
        self.marked_range = (!new_text.is_empty()).then(|| composition.clone());
        // The IME's selected part: its caret, or the clause it converts.
        let base = self.offset_to_utf16(composition.start);
        self.marked_selection = new_selected_range_utf16.as_ref().map(|selected| {
            let start = self.offset_from_utf16(base + selected.start.min(len_utf16));
            let end = self.offset_from_utf16(base + selected.end.min(len_utf16));
            start..end
        });
        // Marking nothing cancels the composition.
        if was_marked && self.marked_range.is_none() {
            cx.emit(CodeEditorEvent::CompositionEnded);
        }
        if !composed_at_cursors && let Some(selected) = self.marked_selection.clone() {
            let selection = Selection {
                anchor: self.snapshot.offset_to_point_utf16(selected.start),
                head: self.snapshot.offset_to_point_utf16(selected.end),
            };
            self.cursors.set_selections(&[selection]);
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = self.snapshot.offset_to_point_utf16(range.start);
        let line = layout.line_for_point(start)?;
        let x = layout.text_origin_x() + line.x_for_column(start.column);
        let top = layout.line_top(line.view_line);
        Some(Bounds::new(
            point(x, top),
            gpui::size(px(1.), layout.line_height_of(line.view_line)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let position = self.position_for_point(point, window, cx);
        let offset = self.snapshot.point_utf16_to_offset(position);
        Some(self.offset_to_utf16(offset))
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.snapshot.len_utf16().0)
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        !self.options.read_only
    }
}

macro_rules! move_action {
    ($div:expr, $cx:expr, $($action:ident => ($command:expr, $select:expr)),* $(,)?) => {
        $div$(.on_action($cx.listener(|this, _: &$action, _, cx| {
            this.move_cursors($command, $select, cx)
        })))*
    };
}

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.refocus) {
            window.focus(&self.focus_handle, cx);
        }
        // Monaco `accessibilitySupport`, and reading new inline suggestions.
        self.screen_reader.optimized = self.screen_reader_optimized(window);
        self.announce_inline_suggestion(cx);
        let accessibility_off =
            self.options.accessibility_support == crate::options::AccessibilitySupport::Off;
        // Focus listeners can miss changes (e.g. before the window is
        // active), so the frame keeps `has_focus` true to the window.
        let focused = self.focus_handle.is_focused(window);
        if focused != self.has_focus {
            self.has_focus = focused;
            self.update_bracket_highlights(cx);
        }
        let mut key_context = gpui::KeyContext::new_with_defaults();
        key_context.add(KEY_CONTEXT);
        if self.options.single_line {
            key_context.add("single_line");
        }
        if self.is_suggest_visible() {
            key_context.add("suggest_widget_visible");
            if self.is_suggestion_details_focused() {
                key_context.add("suggest_widget_details_focused");
            }
        }
        if self.is_signature_help_visible() {
            key_context.add("parameter_hints_visible");
            if self.has_multiple_signatures(cx) {
                key_context.add("parameter_hints_multiple_signatures");
            }
        }
        if self.sticky.focused.is_some() {
            key_context.add("sticky_scroll_focused");
        }
        if self.is_post_edit_widget_visible() {
            key_context.add("post_edit_widget");
        }
        if self.options.tab_focus_mode || crate::tab_focus_mode(cx) {
            key_context.add("tab_moves_focus");
        }
        if self.inline_suggestion_visible(cx) {
            key_context.add("inline_suggestion_visible");
            if self.cursor_before_ghost_text(cx) {
                key_context.add("cursor_before_ghost_text");
            }
        }
        // Monaco `inlineEditIsVisible`, `cursorAtInlineEdit`,
        // `tabShouldJumpToInlineEdit` and `tabShouldAcceptInlineEdit`.
        if self.inline_edit_visible(cx) {
            key_context.add("inline_edit_visible");
            if self.cursor_at_inline_edit(cx) {
                key_context.add("cursor_at_inline_edit");
            }
            if self.tab_should_jump_to_inline_edit(cx) {
                key_context.add("tab_should_jump_to_inline_edit");
            }
            if self.tab_should_accept_inline_edit(cx) {
                key_context.add("tab_should_accept_inline_edit");
            }
            if self.inline_suggestion_alternative_action_visible(cx) {
                key_context.add("inline_suggestion_alternative_action_visible");
            }
        }
        // Monaco's context keys, for keybinding rules and preconditions.
        self.render_commands(&mut key_context, window, cx);
        let root = div()
            .id("code-editor")
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            // Screen readers: a text input whose text runs are the lines
            // around the selection (Monaco's screen reader content).
            .role(if self.options.single_line {
                gpui::Role::TextInput
            } else {
                gpui::Role::MultilineTextInput
            })
            // Monaco `ariaLabelForScreenReaderContent`.
            .aria_label(if accessibility_off {
                crate::accessibility::NOT_ACCESSIBLE_LABEL.into()
            } else {
                self.options.aria_label.clone()
            })
            .map(|root| {
                if !window.is_a11y_active() {
                    return root;
                }
                // Monaco `accessibilitySupport: 'off'` gives the screen
                // reader no text.
                let content = (!accessibility_off).then(|| self.screen_reader_content());
                let announcement = self.announcement();
                let rows = self.a11y_rows.clone();
                let (read_only, required) = (self.options.read_only, self.options.aria_required);
                let editor = cx.entity().downgrade();
                let action_rows = self.a11y_rows.clone();
                root.a11y_synthetic_children(move |builder| {
                    match &content {
                        Some(content) => content.build(builder, &rows),
                        None => rows.borrow_mut().clear(),
                    }
                    if let Some((message, second)) = &announcement {
                        crate::accessibility::build_announcement(builder, message, *second);
                    }
                    let node = builder.parent_node();
                    if read_only {
                        node.set_read_only();
                    }
                    if required {
                        node.set_required();
                    }
                })
                .on_a11y_action(
                    gpui::AccessibleAction::SetTextSelection,
                    move |data, _, cx| {
                        if let Some(gpui::accesskit::ActionData::SetTextSelection(selection)) = data
                            && let Some(editor) = editor.upgrade()
                        {
                            editor.update(cx, |editor, cx| {
                                editor.a11y_set_selection(selection, &action_rows, cx)
                            });
                        }
                    },
                )
            })
            .size_full()
            .cursor(if self.dnd.target.is_some() || self.dnd.copy {
                // Monaco's `mouseStyle` while dragging the selection.
                if self.dnd.copy {
                    MouseCursor::DragCopy
                } else {
                    MouseCursor::Arrow
                }
            } else if self.has_definition_link()
                || self.has_active_link()
                || self.has_active_inlay_hint_link()
            {
                MouseCursor::PointingHand
            } else {
                self.text_mouse_cursor()
            });
        let root = move_action!(root, cx,
            CursorLeft => (MoveCommand::Left, false),
            CursorLeftSelect => (MoveCommand::Left, true),
            CursorRight => (MoveCommand::Right, false),
            CursorRightSelect => (MoveCommand::Right, true),
            CursorUp => (MoveCommand::Up, false),
            CursorUpSelect => (MoveCommand::Up, true),
            CursorDown => (MoveCommand::Down, false),
            CursorDownSelect => (MoveCommand::Down, true),
            CursorWordLeft => (MoveCommand::WordLeft, false),
            CursorWordLeftSelect => (MoveCommand::WordLeft, true),
            CursorWordRight => (MoveCommand::WordRight, false),
            CursorWordRightSelect => (MoveCommand::WordRight, true),
            CursorWordStartLeft => (MoveCommand::WordStartLeft, false),
            CursorWordStartLeftSelect => (MoveCommand::WordStartLeft, true),
            CursorWordStartRight => (MoveCommand::WordStartRight, false),
            CursorWordStartRightSelect => (MoveCommand::WordStartRight, true),
            CursorWordEndLeft => (MoveCommand::WordEndLeft, false),
            CursorWordEndLeftSelect => (MoveCommand::WordEndLeft, true),
            CursorWordEndRight => (MoveCommand::WordEndRight, false),
            CursorWordEndRightSelect => (MoveCommand::WordEndRight, true),
            CursorWordPartLeft => (MoveCommand::WordPartLeft, false),
            CursorWordPartLeftSelect => (MoveCommand::WordPartLeft, true),
            CursorWordPartRight => (MoveCommand::WordPartRight, false),
            CursorWordPartRightSelect => (MoveCommand::WordPartRight, true),
            CursorHome => (MoveCommand::Home, false),
            CursorHomeSelect => (MoveCommand::Home, true),
            CursorEnd => (MoveCommand::End, false),
            CursorEndSelect => (MoveCommand::End, true),
            CursorTop => (MoveCommand::Top, false),
            CursorTopSelect => (MoveCommand::Top, true),
            CursorBottom => (MoveCommand::Bottom, false),
            CursorBottomSelect => (MoveCommand::Bottom, true),
            CursorPageUp => (MoveCommand::PageUp, false),
            CursorPageUpSelect => (MoveCommand::PageUp, true),
            CursorPageDown => (MoveCommand::PageDown, false),
            CursorPageDownSelect => (MoveCommand::PageDown, true),
            CursorLineStart => (MoveCommand::LineStart, false),
            CursorLineStartSelect => (MoveCommand::LineStart, true),
            CursorLineEnd => (MoveCommand::LineEnd, false),
            CursorLineEndSelect => (MoveCommand::LineEnd, true),
        );
        let root = root
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::expand_line_selection))
            .on_action(cx.listener(Self::add_cursor_above))
            .on_action(cx.listener(Self::add_cursor_below))
            .on_action(cx.listener(Self::remove_secondary_cursors))
            .on_action(cx.listener(Self::delete_left))
            .on_action(cx.listener(Self::delete_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(|this, _: &DeleteWordStartLeft, _, cx| {
                this.delete_word(true, cursor::WordNavigationType::WordStart, false, cx)
            }))
            .on_action(cx.listener(|this, _: &DeleteWordEndLeft, _, cx| {
                this.delete_word(true, cursor::WordNavigationType::WordEnd, false, cx)
            }))
            .on_action(cx.listener(|this, _: &DeleteWordStartRight, _, cx| {
                this.run_edit(cx, |c, m, s| {
                    cursor::delete_word(
                        c,
                        m,
                        s,
                        false,
                        cursor::WordNavigationType::WordStart,
                        false,
                    )
                })
            }))
            .on_action(cx.listener(|this, _: &DeleteWordEndRight, _, cx| {
                this.run_edit(cx, |c, m, s| {
                    cursor::delete_word(c, m, s, false, cursor::WordNavigationType::WordEnd, false)
                })
            }))
            .on_action(cx.listener(|this, _: &DeleteWordPartLeft, _, cx| {
                let operation = this.with_language(cx, |config, rope, cursors, lang| {
                    cursor::delete_word_part_with_language(config, rope, cursors, true, lang)
                });
                this.apply(operation, cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteWordPartRight, _, cx| {
                this.run_edit(cx, |c, m, s| cursor::delete_word_part(c, m, s, false))
            }))
            .on_action(cx.listener(|this, _: &DeleteInsideWord, _, cx| {
                this.run_edit(cx, |c, m, s| cursor::delete_inside_word(c, m, s, false))
            }))
            .on_action(
                cx.listener(|this, _: &AddSelectionToPreviousFindMatch, _, cx| {
                    this.add_selection_to_previous_find_match(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &MoveSelectionToPreviousFindMatch, _, cx| {
                    this.move_selection_to_previous_find_match(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &InsertCursorAtEndOfEachLineSelected, _, cx| {
                    this.insert_cursor_at_end_of_each_line_selected(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &AddCursorsToBottom, _, cx| {
                    this.add_cursors_to_edge(true, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &AddCursorsToTop, _, cx| this.add_cursors_to_edge(false, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FocusNextCursor, _, cx| this.focus_next_cursor(true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FocusPreviousCursor, _, cx| {
                    this.focus_next_cursor(false, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CursorColumnSelectLeft, _, cx| {
                this.column_select(cursor::ColumnSelectCommand::Left, cx)
            }))
            .on_action(cx.listener(|this, _: &CursorColumnSelectRight, _, cx| {
                this.column_select(cursor::ColumnSelectCommand::Right, cx)
            }))
            .on_action(cx.listener(|this, _: &CursorColumnSelectUp, _, cx| {
                this.column_select(cursor::ColumnSelectCommand::Up, cx)
            }))
            .on_action(cx.listener(|this, _: &CursorColumnSelectDown, _, cx| {
                this.column_select(cursor::ColumnSelectCommand::Down, cx)
            }))
            .on_action(cx.listener(|this, _: &CursorColumnSelectPageUp, _, cx| {
                this.column_select(cursor::ColumnSelectCommand::PageUp, cx)
            }))
            .on_action(cx.listener(|this, _: &CursorColumnSelectPageDown, _, cx| {
                this.column_select(cursor::ColumnSelectCommand::PageDown, cx)
            }))
            .on_action(cx.listener(|this, _: &CursorUndo, _, cx| this.cursor_undo(cx)))
            .on_action(
                cx.listener(|this, _: &WordHighlightNext, _, cx| {
                    this.go_to_word_highlight(true, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &WordHighlightPrev, _, cx| {
                this.go_to_word_highlight(false, cx)
            }))
            .on_action(
                cx.listener(|this, _: &WordHighlightTrigger, _, cx| {
                    this.trigger_word_highlight(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &InPlaceReplaceUp, _, cx| this.in_place_replace(true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &InPlaceReplaceDown, _, cx| this.in_place_replace(false, cx)),
            )
            .on_action(cx.listener(|this, _: &RemoveBrackets, _, cx| this.remove_brackets(cx)))
            .on_action(cx.listener(|this, _: &ReindentLines, _, cx| {
                let operation = this.with_language(cx, |config, rope, cursors, lang| {
                    lang.and_then(|lang| cursor::reindent_lines(config, rope, cursors, lang))
                });
                if let Some(operation) = operation {
                    this.apply(operation, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ReindentSelectedLines, _, cx| {
                let operation = this.with_language(cx, |config, rope, cursors, lang| {
                    lang.and_then(|lang| {
                        cursor::reindent_selected_lines(config, rope, cursors, lang)
                    })
                });
                if let Some(operation) = operation {
                    this.apply(operation, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SortLinesAscending, _, cx| {
                this.run_optional_edit(cx, |c, m, s| cursor::sort_lines(c, m, s, false))
            }))
            .on_action(cx.listener(|this, _: &SortLinesDescending, _, cx| {
                this.run_optional_edit(cx, |c, m, s| cursor::sort_lines(c, m, s, true))
            }))
            .on_action(cx.listener(|this, _: &DeleteDuplicateLines, _, cx| {
                this.run_optional_edit(cx, cursor::delete_duplicate_lines)
            }))
            .on_action(
                cx.listener(|this, _: &ReverseLines, _, cx| {
                    this.run_edit(cx, cursor::reverse_lines)
                }),
            )
            .on_action(cx.listener(|this, _: &TrimTrailingWhitespace, _, cx| {
                this.run_edit(cx, cursor::trim_trailing_whitespace)
            }))
            .on_action(cx.listener(|this, _: &DeleteAllRight, _, cx| {
                this.run_edit(cx, cursor::delete_all_right)
            }))
            .on_action(
                cx.listener(|this, _: &JoinLines, _, cx| this.run_edit(cx, cursor::join_lines)),
            )
            .on_action(
                cx.listener(|this, _: &Transpose, _, cx| this.run_edit(cx, cursor::transpose)),
            )
            .on_action(cx.listener(|this, _: &TransposeLetters, _, cx| {
                this.run_optional_edit(cx, cursor::transpose_letters)
            }))
            .on_action(cx.listener(|this, _: &DuplicateSelection, _, cx| {
                this.run_edit(cx, cursor::duplicate_selection)
            }))
            .on_action(cx.listener(|this, _: &MoveCaretLeft, _, cx| {
                this.run_edit(cx, |c, m, s| cursor::move_caret(c, m, s, true))
            }))
            .on_action(cx.listener(|this, _: &MoveCaretRight, _, cx| {
                this.run_edit(cx, |c, m, s| cursor::move_caret(c, m, s, false))
            }))
            .on_action(cx.listener(|this, _: &TransformToUppercase, _, cx| {
                this.transform_case(cursor::CaseTransform::Upper, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformToLowercase, _, cx| {
                this.transform_case(cursor::CaseTransform::Lower, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformToTitlecase, _, cx| {
                this.transform_case(cursor::CaseTransform::Title, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformToSnakecase, _, cx| {
                this.transform_case(cursor::CaseTransform::Snake, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformToCamelcase, _, cx| {
                this.transform_case(cursor::CaseTransform::Camel, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformToPascalcase, _, cx| {
                this.transform_case(cursor::CaseTransform::Pascal, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformToKebabcase, _, cx| {
                this.transform_case(cursor::CaseTransform::Kebab, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ToggleOvertypeInsertMode, _, cx| this.toggle_overtype(cx)),
            )
            .on_action(cx.listener(|this, _: &CursorRedo, _, cx| this.cursor_redo(cx)))
            .on_action(
                cx.listener(|this, _: &SetSelectionAnchor, _, cx| this.set_selection_anchor(cx)),
            )
            .on_action(
                cx.listener(|this, _: &GoToSelectionAnchor, _, cx| this.go_to_selection_anchor(cx)),
            )
            .on_action(cx.listener(|this, _: &SelectFromAnchorToCursor, _, cx| {
                this.select_from_anchor_to_cursor(cx)
            }))
            .on_action(cx.listener(|this, _: &CancelSelectionAnchor, _, cx| {
                this.cancel_selection_anchor(cx);
            }))
            .on_action(cx.listener(|this, _: &ScrollLineUp, _, cx| this.scroll_lines(-1., cx)))
            .on_action(cx.listener(|this, _: &ScrollLineDown, _, cx| this.scroll_lines(1., cx)))
            .on_action(cx.listener(|this, _: &ScrollPageUp, _, cx| this.scroll_pages(-1., cx)))
            .on_action(cx.listener(|this, _: &ScrollPageDown, _, cx| this.scroll_pages(1., cx)))
            .on_action(
                cx.listener(|this, _: &ScrollEditorTop, _, cx| this.scroll_to_edge(false, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ScrollEditorBottom, _, cx| this.scroll_to_edge(true, cx)),
            )
            .on_action(cx.listener(|this, _: &CancelSelection, _, cx| {
                let primary = this.cursors.primary().position;
                if !this.cursors.primary().selection().is_empty() {
                    this.set_selections(&[Selection::collapsed(primary)], cx);
                }
            }))
            .on_action(cx.listener(|this, _: &LineBreakInsert, _, cx| {
                let operation = this.with_language(cx, |config, rope, cursors, lang| {
                    cursor::line_break_insert(config, rope, cursors, lang)
                });
                this.apply(operation, cx);
            }))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::delete_all_left))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::indent_lines))
            .on_action(cx.listener(Self::outdent_lines))
            .on_action(cx.listener(Self::move_lines_up))
            .on_action(cx.listener(Self::move_lines_down))
            .on_action(cx.listener(Self::copy_lines_up))
            .on_action(cx.listener(Self::copy_lines_down))
            .on_action(cx.listener(Self::delete_lines))
            .on_action(cx.listener(|this, _: &CommentLine, _, cx| {
                this.comment(Some(cursor::LineCommentType::Toggle), cx)
            }))
            .on_action(cx.listener(|this, _: &AddCommentLine, _, cx| {
                this.comment(Some(cursor::LineCommentType::ForceAdd), cx)
            }))
            .on_action(cx.listener(|this, _: &RemoveCommentLine, _, cx| {
                this.comment(Some(cursor::LineCommentType::ForceRemove), cx)
            }))
            .on_action(cx.listener(|this, _: &BlockComment, _, cx| this.comment(None, cx)))
            .on_action(cx.listener(Self::insert_line_after))
            .on_action(cx.listener(Self::insert_line_before))
            .on_action(cx.listener(|this, _: &FoldAll, _, cx| this.fold_all(cx)))
            .on_action(cx.listener(|this, _: &UnfoldAll, _, cx| this.unfold_all(cx)))
            .on_action(cx.listener(|this, _: &ToggleFold, _, cx| this.toggle_fold(cx)))
            .on_action(cx.listener(|this, _: &FoldRecursively, _, cx| this.fold_recursively(cx)))
            .on_action(
                cx.listener(|this, _: &UnfoldRecursively, _, cx| this.unfold_recursively(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFoldRecursively, _, cx| {
                this.toggle_fold_recursively(cx)
            }))
            .on_action(
                cx.listener(|this, _: &FoldAllBlockComments, _, cx| {
                    this.fold_all_block_comments(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &FoldAllMarkerRegions, _, cx| {
                this.fold_all_marker_regions(true, cx)
            }))
            .on_action(cx.listener(|this, _: &UnfoldAllMarkerRegions, _, cx| {
                this.fold_all_marker_regions(false, cx)
            }))
            .on_action(cx.listener(|this, _: &FoldAllExcept, _, cx| this.fold_all_except(true, cx)))
            .on_action(
                cx.listener(|this, _: &UnfoldAllExcept, _, cx| this.fold_all_except(false, cx)),
            )
            .on_action(cx.listener(|this, _: &FoldLevel1, _, cx| this.fold_level(1, cx)))
            .on_action(cx.listener(|this, _: &FoldLevel2, _, cx| this.fold_level(2, cx)))
            .on_action(cx.listener(|this, _: &FoldLevel3, _, cx| this.fold_level(3, cx)))
            .on_action(cx.listener(|this, _: &FoldLevel4, _, cx| this.fold_level(4, cx)))
            .on_action(cx.listener(|this, _: &FoldLevel5, _, cx| this.fold_level(5, cx)))
            .on_action(cx.listener(|this, _: &FoldLevel6, _, cx| this.fold_level(6, cx)))
            .on_action(cx.listener(|this, _: &FoldLevel7, _, cx| this.fold_level(7, cx)))
            .on_action(cx.listener(|this, _: &GoToParentFold, _, cx| this.go_to_parent_fold(cx)))
            .on_action(
                cx.listener(|this, _: &GoToPreviousFold, _, cx| this.go_to_previous_fold(cx)),
            )
            .on_action(cx.listener(|this, _: &GoToNextFold, _, cx| this.go_to_next_fold(cx)))
            .on_action(cx.listener(|this, _: &CreateFoldFromSelection, _, cx| {
                this.create_fold_from_selection(cx)
            }))
            .on_action(cx.listener(|this, _: &RemoveManualFoldingRanges, _, cx| {
                this.remove_manual_folding_ranges(cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleImportFold, _, cx| this.toggle_import_fold(cx)))
            .on_action(cx.listener(|this, _: &ExpandSelection, _, cx| this.expand_selection(cx)))
            .on_action(cx.listener(|this, _: &ShrinkSelection, _, cx| this.shrink_selection(cx)))
            .on_action(cx.listener(|this, _: &StartFind, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.open_find(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &StartFindReplace, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.open_find(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextMatch, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.find_match_action(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousMatch, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.find_match_action(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextSelectionMatch, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.find_selection_match_action(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousSelectionMatch, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.find_selection_match_action(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &StartFindWithSelection, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.start_find_with_selection(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ReplaceOne, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.replace_one(cx)
            }))
            .on_action(cx.listener(|this, _: &ReplaceAll, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.replace_all(cx)
            }))
            .on_action(cx.listener(|this, _: &SelectAllMatches, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                // Monaco `FindController.selectAllMatches`: the widget
                // stays; the editor takes the focus.
                this.select_all_matches(cx);
                window.focus(&this.focus_handle, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleCaseSensitive, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                let mut options = this.find_options().clone();
                options.match_case = !options.match_case;
                this.set_find_options(options, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleRegex, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                let mut options = this.find_options().clone();
                options.is_regex = !options.is_regex;
                this.set_find_options(options, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleWholeWord, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                let mut options = this.find_options().clone();
                options.whole_word = !options.whole_word;
                this.set_find_options(options, cx);
            }))
            .on_action(cx.listener(|this, _: &TogglePreserveCase, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                let mut options = this.find_options().clone();
                options.preserve_case = !options.preserve_case;
                this.set_find_options(options, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleFindInSelection, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                let mut options = this.find_options().clone();
                options.in_selection = !options.in_selection;
                this.set_find_options(options, cx);
            }))
            .on_action(cx.listener(|this, _: &AddSelectionToNextFindMatch, _, cx| {
                this.add_selection_to_next_find_match(cx)
            }))
            .on_action(
                cx.listener(|this, _: &MoveSelectionToNextFindMatch, _, cx| {
                    this.move_selection_to_next_find_match(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SelectAllOccurrences, _, cx| {
                    this.select_all_occurrences(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &JumpToBracket, _, cx| this.jump_to_bracket(cx)))
            .on_action(cx.listener(|this, _: &SelectToBracket, _, cx| this.select_to_bracket(cx)))
            .on_action(cx.listener(|this, _: &GoToNextMarker, _, cx| this.go_to_marker(true, cx)))
            .on_action(
                cx.listener(|this, _: &GoToNextMarkerInFiles, _, cx| this.go_to_marker(true, cx)),
            )
            .on_action(cx.listener(|this, _: &GoToPreviousMarkerInFiles, _, cx| {
                this.go_to_marker(false, cx)
            }))
            .on_action(
                cx.listener(|this, _: &GoToPreviousMarker, _, cx| this.go_to_marker(false, cx)),
            )
            .on_action(cx.listener(|this, _: &GoToDefinition, _, cx| this.go_to_definition(cx)))
            .on_action(cx.listener(|this, _: &HideHover, _, cx| this.hide_hover(cx)))
            .on_action(cx.listener(|this, _: &ShowDefinitionPreviewHover, _, cx| {
                this.show_definition_preview_hover(cx)
            }))
            .on_action(cx.listener(|this, _: &IncreaseHoverVerbosityLevel, _, cx| {
                this.update_hover_verbosity_level(true, None, cx)
            }))
            .on_action(cx.listener(|this, _: &DecreaseHoverVerbosityLevel, _, cx| {
                this.update_hover_verbosity_level(false, None, cx)
            }))
            .on_action(cx.listener(|this, _: &HideLongLineWarningHover, _, cx| {
                this.hide_long_line_warning_hover(cx)
            }))
            .on_action(cx.listener(|this, _: &OpenLink, _, cx| this.open_link(cx)))
            .on_action(cx.listener(|this, _: &GotoLine, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.open_quick_access(":", window, cx)
            }))
            .on_action(cx.listener(|this, _: &QuickOutline, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.open_quick_access("@", window, cx)
            }))
            .on_action(cx.listener(|this, _: &GotoOffset, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.open_quick_access("::", window, cx)
            }))
            .on_action(cx.listener(|this, _: &ForceRetokenize, _, cx| this.force_retokenize(cx)))
            .on_action(cx.listener(|this, _: &InspectTokens, _, cx| this.launch_inspect_tokens(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleHighContrast, _, cx| this.toggle_high_contrast(cx)),
            )
            .on_action(cx.listener(|this, _: &CancelOperation, _, cx| {
                if !this.cancel_operation(cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, args: &GoToLocations, _, cx| {
                this.go_to_locations_action(args.clone(), cx)
            }))
            .on_action(cx.listener(|this, args: &PeekLocations, _, cx| {
                this.go_to_locations_action(args.clone().into(), cx)
            }))
            .on_action(cx.listener(
                |this, _: &DisableHighlightingOfAmbiguousCharacters, _, cx| {
                    this.disable_unicode_highlighting(|o| o.ambiguous_characters = false, cx)
                },
            ))
            .on_action(cx.listener(
                |this, _: &DisableHighlightingOfInvisibleCharacters, _, cx| {
                    this.disable_unicode_highlighting(|o| o.invisible_characters = false, cx)
                },
            ))
            .on_action(cx.listener(
                |this, _: &DisableHighlightingOfNonBasicAsciiCharacters, _, cx| {
                    this.disable_unicode_highlighting(|o| o.non_basic_ascii = false, cx)
                },
            ))
            .on_action(
                cx.listener(|this, _: &ShowOrFocusStandaloneColorPicker, window, cx| {
                    if this.options.single_line {
                        return cx.propagate();
                    }
                    this.show_or_focus_standalone_color_picker(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ShowContextMenu, window, cx| {
                if !this.options.context_menu {
                    return cx.propagate();
                }
                this.show_context_menu(None, window, cx)
            }))
            .on_action(cx.listener(|this, _: &QuickCommand, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.open_quick_access(">", window, cx)
            }))
            .on_action(cx.listener(|this, action: &RunAction, window, cx| {
                if !this.run_action(&action.id, window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, action: &RunCommand, window, cx| {
                if !this.run_command(&action.id, action.args.clone(), window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &StartLinkedEditing, _, cx| {
                this.update_linked_editing(true, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowNextInlineSuggestion, _, cx| {
                this.show_next_inline_suggestion(true, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ShowPreviousInlineSuggestion, _, cx| {
                    this.show_next_inline_suggestion(false, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &AcceptInlineSuggestion, _, cx| {
                this.accept_inline_suggestion(cx);
            }))
            .on_action(cx.listener(|this, _: &AcceptNextInlineWord, _, cx| {
                this.accept_next_inline_word(cx);
            }))
            .on_action(cx.listener(|this, _: &AcceptNextInlineLine, _, cx| {
                this.accept_next_inline_line(cx);
            }))
            .on_action(
                cx.listener(|this, _: &HideInlineSuggestion, _, cx| {
                    this.hide_inline_suggestion(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ToggleAlwaysShowInlineSuggestionToolbar, _, cx| {
                    this.toggle_always_show_inline_toolbar(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &JumpToNextInlineEdit, _, cx| {
                this.jump_to_next_inline_edit(cx)
            }))
            .on_action(
                cx.listener(|this, _: &AcceptInlineSuggestionAlternativeAction, _, cx| {
                    this.accept_inline_suggestion_alternative_action(cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &ToggleInlineSuggestionShowCollapsed, _, cx| {
                    this.toggle_inline_edits_show_collapsed(cx)
                }),
            )
            .on_modifiers_changed(cx.listener(
                |this, event: &gpui::ModifiersChangedEvent, _, cx| {
                    this.inlay_hints_modifiers(&event.modifiers, cx);
                    this.hover_modifiers_changed(&event.modifiers, cx);
                    this.sticky_mouse_move(None, event.modifiers.shift, cx);
                    this.dnd_modifiers_changed(&event.modifiers, cx);
                },
            ))
            .on_action(
                cx.listener(|this, _: &ToggleStickyScroll, _, cx| this.toggle_sticky_scroll(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleTabFocusMode, _, cx| {
                let on = !crate::tab_focus_mode(cx);
                crate::set_tab_focus_mode(on, cx);
                // Monaco `ToggleTabFocusModeAction` alerts the new mode.
                this.announce(
                    if on {
                        "Pressing Tab will now move focus to the next focusable element"
                    } else {
                        "Pressing Tab will now insert the tab character"
                    },
                    cx,
                );
            }))
            .on_action(|_: &FocusNextElement, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPreviousElement, window, cx| window.focus_prev(cx))
            .on_action(cx.listener(|this, _: &PasteAsText, window, cx| {
                this.paste_as(Some("text.plain".into()), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ChangePasteType, window, cx| {
                this.show_post_edit_selector(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ChangeDropType, window, cx| {
                this.show_post_edit_selector(window, cx)
            }))
            .on_action(cx.listener(|this, _: &HidePostEditWidget, _, cx| {
                this.clear_post_edit(cx);
            }))
            // Monaco `dropIntoEditor`: a caret follows dragged files, and
            // dropping them runs the drop providers.
            .on_drag_move::<gpui::ExternalPaths>(cx.listener(
                |this, event: &gpui::DragMoveEvent<gpui::ExternalPaths>, window, cx| {
                    let inside = event.bounds.contains(&event.event.position);
                    let target = (inside && this.options.drop_into_editor.enabled)
                        .then(|| this.position_for_point(event.event.position, window, cx));
                    if this.dnd.target != target {
                        this.dnd.target = target;
                        cx.notify();
                    }
                },
            ))
            .on_drop(
                cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                    this.dnd.target = None;
                    let position = this.position_for_point(window.mouse_position(), window, cx);
                    window.focus(&this.focus_handle, cx);
                    this.drop_files(paths.paths(), position, cx);
                }),
            )
            .on_action(|_: &FontZoomIn, _, cx| {
                crate::set_editor_zoom_level(crate::editor_zoom_level(cx) + 1., cx)
            })
            .on_action(|_: &FontZoomOut, _, cx| {
                crate::set_editor_zoom_level(crate::editor_zoom_level(cx) - 1., cx)
            })
            .on_action(|_: &FontZoomReset, _, cx| crate::set_editor_zoom_level(0., cx))
            .on_action(
                cx.listener(|this, _: &FocusStickyScroll, _, cx| this.focus_sticky_scroll(cx)),
            )
            .on_action(cx.listener(|this, _: &SelectNextStickyScrollLine, _, cx| {
                this.sticky_focus_move(true, cx)
            }))
            .on_action(
                cx.listener(|this, _: &SelectPreviousStickyScrollLine, _, cx| {
                    this.sticky_focus_move(false, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &GoToFocusedStickyScrollLine, _, cx| {
                this.sticky_go_to_focused(cx)
            }))
            .on_action(cx.listener(|this, _: &SelectEditor, _, cx| this.sticky_select_editor(cx)))
            .on_action(cx.listener(|this, _: &Rename, _, cx| this.rename(cx)))
            .on_action(cx.listener(|this, _: &QuickFix, _, cx| this.quick_fix(cx)))
            .on_action(cx.listener(|this, _: &OrganizeImports, _, cx| this.organize_imports(cx)))
            .on_action(cx.listener(|this, _: &FixAll, _, cx| this.fix_all(cx)))
            .on_action(cx.listener(|this, _: &AutoFix, _, cx| this.auto_fix(cx)))
            .on_action(cx.listener(|this, action: &TriggerCodeAction, _, cx| {
                this.code_action_with_args(action, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusSuggestion, _, cx| {
                this.focus_suggestion(cx);
            }))
            .on_action(cx.listener(|this, _: &FocusAndAcceptSuggestion, _, cx| {
                if this.focus_suggestion(cx) {
                    this.accept_suggestion(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleExplainMode, _, cx| {
                this.toggle_suggest_explain_mode(cx);
            }))
            .on_action(cx.listener(|this, _: &ResetSuggestSize, _, cx| this.reset_suggest_size(cx)))
            .on_action(cx.listener(|this, _: &ToggleSuggestionFocus, _, cx| {
                this.toggle_suggestion_focus(cx);
            }))
            .on_action(cx.listener(|this, _: &SuggestWidgetCopy, _, cx| {
                this.copy_suggestion_details(cx);
            }))
            .on_action(cx.listener(|this, action: &FindWithArgs, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.find_with_args(action, window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToMatch, window, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.go_to_match(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RevealDefinitionAside, _, cx| {
                this.reveal_definition_aside(cx)
            }))
            .on_action(cx.listener(|this, _: &TogglePeekWidgetFocus, window, cx| {
                if !this.toggle_peek_widget_focus(window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(
                cx.listener(|this, _: &InsertFinalNewLine, _, cx| this.insert_final_new_line(cx)),
            )
            .on_action(
                cx.listener(|this, _: &ShowLensesInCurrentLine, window, cx| {
                    this.show_lenses_in_current_line(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &GoToDeclaration, _, cx| {
                this.go_to(LocationKind::Declaration, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToTypeDefinition, _, cx| {
                this.go_to(LocationKind::TypeDefinition, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToImplementation, _, cx| {
                this.go_to(LocationKind::Implementation, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToReferences, _, cx| {
                this.go_to(LocationKind::References, cx)
            }))
            .on_action(cx.listener(|this, _: &PeekDefinition, _, cx| {
                this.peek(LocationKind::Definition, cx)
            }))
            .on_action(cx.listener(|this, _: &PeekDeclaration, _, cx| {
                this.peek(LocationKind::Declaration, cx)
            }))
            .on_action(cx.listener(|this, _: &PeekTypeDefinition, _, cx| {
                this.peek(LocationKind::TypeDefinition, cx)
            }))
            .on_action(cx.listener(|this, _: &PeekImplementation, _, cx| {
                this.peek(LocationKind::Implementation, cx)
            }))
            .on_action(cx.listener(|this, _: &PeekReferences, _, cx| {
                this.peek(LocationKind::References, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToNextReference, _, cx| {
                // A preview editor leaves F4 to the editor around it.
                let handled = !this.in_peek
                    && (this.peek_select_relative(1, cx) || this.go_to_next_symbol_from_result(cx));
                if !handled {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &GoToPreviousReference, _, cx| {
                if this.in_peek || !this.peek_select_relative(-1, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &FormatDocument, _, cx| {
                this.format_document(cx);
            }))
            .on_action(cx.listener(|this, _: &FormatSelection, _, cx| {
                this.format_selection(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSuggestionDetails, _, cx| {
                // Monaco `focusSuggestion` shares the keys while no
                // suggestion is focused.
                if this.focus_suggestion(cx) {
                    return;
                }
                if this.selected_suggestion().is_some() {
                    this.toggle_suggestion_details(cx);
                } else {
                    this.trigger_suggest(crate::providers::CompletionTrigger::Invoke, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AcceptAlternativeSuggestion, _, cx| {
                this.accept_alternative_suggestion(cx);
            }))
            .on_action(
                cx.listener(|this, _: &IndentationToSpaces, _, cx| this.indentation_to_spaces(cx)),
            )
            .on_action(
                cx.listener(|this, _: &IndentationToTabs, _, cx| this.indentation_to_tabs(cx)),
            )
            .on_action(cx.listener(|this, _: &IndentUsingSpaces, window, cx| {
                this.change_indentation_size(Some(true), window, cx)
            }))
            .on_action(cx.listener(|this, _: &IndentUsingTabs, window, cx| {
                this.change_indentation_size(Some(false), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ChangeTabDisplaySize, window, cx| {
                this.change_indentation_size(None, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &DetectIndentation, _, cx| this.detect_indentation(cx)),
            )
            .on_action(cx.listener(|this, _: &SelectNextSuggestion, _, cx| {
                this.suggest_navigate(1, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPreviousSuggestion, _, cx| {
                this.suggest_navigate(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectNextPageSuggestion, _, cx| {
                this.suggest_navigate_page(true, cx);
            }))
            .on_action(
                cx.listener(|this, _: &SelectPreviousPageSuggestion, _, cx| {
                    this.suggest_navigate_page(false, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &SelectFirstSuggestion, _, cx| {
                this.select_first_suggestion(cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLastSuggestion, _, cx| {
                this.select_last_suggestion(cx);
            }))
            .on_action(cx.listener(|this, _: &TriggerSuggest, _, cx| {
                this.trigger_suggest(crate::providers::CompletionTrigger::Invoke, cx)
            }))
            .on_action(cx.listener(|this, _: &TriggerParameterHints, _, cx| {
                this.trigger_signature_help(crate::providers::SignatureHelpTrigger::Invoke, cx)
            }))
            .on_action(cx.listener(Self::close_parameter_hints))
            .on_action(cx.listener(|this, _: &ShowNextParameterHint, _, cx| {
                this.show_next_parameter_hint(cx)
            }))
            .on_action(cx.listener(|this, _: &ShowPrevParameterHint, _, cx| {
                this.show_previous_parameter_hint(cx)
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.redo(cx)))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(
                cx.listener(|_, _: &ShowCharacterPalette, window, _| {
                    window.show_character_palette()
                }),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_middle_mouse_down))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    // Releasing ends it only after it scrolled (Monaco).
                    if this.animation.middle_scroll.is_some_and(|s| s.did_scroll) {
                        this.end_middle_scroll(cx);
                    }
                }),
            )
            .on_key_down(cx.listener(|this, _: &gpui::KeyDownEvent, _, cx| {
                this.end_middle_scroll(cx);
            }))
            // Monaco `onKeyUp`, for the text (not the editor's widgets).
            .on_key_up(cx.listener(|this, event: &gpui::KeyUpEvent, window, cx| {
                if this.focus_handle.is_focused(window) {
                    cx.emit(CodeEditorEvent::KeyUp(event.keystroke.clone()));
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                // Monaco reveals `auto` scrollbars while the mouse is over.
                let scrollbars = &mut this.animation.scrollbars;
                scrollbars.mouse_over = *hovered;
                if !*hovered {
                    scrollbars.left_at = Some(web_time::Instant::now());
                    scrollbars.mouse_position = None;
                    this.minimap_hovered = false;
                }
                cx.notify();
                if !*hovered {
                    this.sticky_mouse_leave(cx);
                    cx.emit(CodeEditorEvent::MouseLeave);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            // Mouse moves are `EditorElement`'s, to follow drags out of the
            // editor.
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel));
        let root = crate::editor_commands::register_handlers(root, cx);
        // Monaco `setBanner`: the banner takes its height off the top.
        match self.banner.clone() {
            Some((banner, height)) => root
                .flex()
                .flex_col()
                .child(div().flex_none().w_full().h(height).child(banner))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(EditorElement::new(cx.entity())),
                ),
            None => root.child(EditorElement::new(cx.entity())),
        }
    }
}

/// Options for an input in one of the editor's widgets (find, replace,
/// quick input, rename): [`EditorOptions::single_line_input`] without the
/// language features, since Monaco's are plain `<input>`s (no suggestions,
/// ghost text, hovers, highlights, links, lenses or lightbulb), in the UI
/// font at 13px (Monaco's `findWidget.css` and `quickInput.css`).
pub(crate) fn widget_input_options() -> EditorOptions {
    use crate::options::*;
    EditorOptions {
        font_family: ".SystemUIFont".into(),
        font_size: px(13.),
        quick_suggestions: QuickSuggestions::OFF,
        suggest_on_trigger_characters: false,
        word_based_suggestions: WordBasedSuggestions::Off,
        snippet_suggestions: None,
        tab_completion: TabCompletion::Off,
        parameter_hints: ParameterHintsOptions {
            enabled: false,
            ..Default::default()
        },
        inline_suggest: InlineSuggestOptions {
            enabled: false,
            ..Default::default()
        },
        hover: HoverOptions {
            enabled: HoverEnabled::Off,
            ..Default::default()
        },
        inlay_hints: InlayHintsOptions {
            enabled: InlayHintsMode::Off,
            ..Default::default()
        },
        lightbulb: LightbulbMode::Off,
        code_lens: false,
        links: false,
        occurrences_highlight: OccurrencesHighlight::Off,
        selection_highlight: false,
        format_on_type: false,
        linked_editing: false,
        semantic_highlighting: SemanticHighlighting::Off,
        bracket_pair_colorization: BracketPairColorizationOptions {
            enabled: false,
            ..Default::default()
        },
        unicode_highlight: UnicodeHighlightOptions {
            non_basic_ascii: false,
            invisible_characters: false,
            ambiguous_characters: false,
            ..Default::default()
        },
        drop_into_editor: DropIntoEditorOptions {
            enabled: false,
            ..Default::default()
        },
        paste_as: PasteAsOptions {
            enabled: false,
            ..Default::default()
        },
        drag_and_drop: false,
        ..EditorOptions::single_line_input()
    }
}

/// Applies `f` to every position in a cursor state.
fn map_state(state: &SingleCursorState, f: impl Fn(PointUtf16) -> PointUtf16) -> SingleCursorState {
    SingleCursorState {
        selection_start: f(state.selection_start.start)..f(state.selection_start.end),
        position: f(state.position),
        ..state.clone()
    }
}

/// The cursors' view states and the model states they're for (see
/// [`CodeEditor::cursor_view_states`]).
struct CursorViewStates {
    model: Vec<SingleCursorState>,
    view: Vec<SingleCursorState>,
}

/// The cursor core's view of the view lines (Monaco's view model lines).
struct ViewCursorModel<'a>(&'a ViewLines);

impl cursor::CursorModel for ViewCursorModel<'_> {
    fn line_count(&self) -> u32 {
        self.0.line_count().max(1)
    }

    fn line_content(&self, row: u32) -> String {
        self.0.line_content(row)
    }

    fn line_max_column(&self, row: u32) -> u32 {
        self.0.line_max_column(row)
    }
}

/// The overview ruler as laid out, for tests: (bounds, color) of each
/// mark.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct OverviewRulerForTests {
    pub bounds: Bounds<Pixels>,
    pub background: Option<Hsla>,
    pub marks: Vec<(Bounds<Pixels>, Hsla)>,
    pub cursors: Vec<(Bounds<Pixels>, Hsla)>,
    /// The left and top edges and their color.
    pub border: Option<([Bounds<Pixels>; 2], Hsla)>,
}
