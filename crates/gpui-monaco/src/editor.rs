//! [`CodeEditor`]: the editor view entity.

use gpui::prelude::FluentBuilder as _;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, CursorStyle as MouseCursor, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, ScrollWheelEvent, StatefulInteractiveElement as _, Styled, Subscription, Task,
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
    CursorChangeReason, CursorChangeSource, ModelChange, MouseWheelEvent, PasteEvent,
    SelectionChange,
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
    ContentWidget, ContentWidgetState, OverlayWidget, OverlayWidgetState, ViewZone, WidgetId,
    Widgets, ZoneHeight, ZoneState,
};

/// Events emitted by [`CodeEditor`].
#[derive(Clone, Debug, PartialEq)]
pub enum CodeEditorEvent {
    /// The cursors or selections changed, and why (Monaco
    /// `onDidChangeCursorSelection`).
    SelectionsChanged(SelectionChange),
    /// The scroll position changed (Monaco `onDidScrollChange`).
    ScrollChanged,
    /// An edit was refused because the editor is read-only (Monaco
    /// `onDidAttemptReadOnlyEdit`).
    ReadOnlyEditAttempt,
    /// The editor gained focus (Monaco `onDidFocusEditorText`).
    Focused,
    /// The editor lost focus (Monaco `onDidBlurEditorText`).
    Blurred,
    /// Go to definition led to another model: the host should open it.
    OpenLocation(crate::providers::Location),
    /// Monaco's "Open Definition to the Side" (`revealDefinitionAside`,
    /// or Cmd/Ctrl+Alt+click on a symbol) found this location, in any
    /// model: the host should open it in an editor beside this one.
    OpenLocationToSide(crate::providers::Location),
    /// A link was followed (Cmd/Ctrl+click or `editor.action.openLink`):
    /// the host opens it. Relative `file://./` links are resolved against
    /// the model's URI.
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
}

/// Monaco's folding update scheduler waits at least this long.
const FOLD_UPDATE_DELAY: Duration = Duration::from_millis(200);

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
struct ClipboardMetadata {
    is_from_empty_selection: bool,
    multicursor_text: Option<Vec<String>>,
    /// The language id of the editor it was copied from (Monaco `mode`).
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MouseSelectMode {
    Character,
    Word,
    Line,
    /// Shift+Alt drag: box (column) selection.
    Column,
}

struct MouseSelection {
    mode: MouseSelectMode,
    /// The selection made by the initial click (a word or line for
    /// double/triple click).
    origin: Selection,
    /// Index of the cursor being dragged.
    cursor_index: usize,
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
    pub(crate) snapshot: BufferSnapshot,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) cursors: CursorCollection,
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
    pub(crate) minimap_cache: crate::element::MinimapCache,
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
    minimap_drag: Option<Pixels>,
    fold_task: Option<Task<()>>,
    /// The editor has focus (Monaco `hasWidgetFocus`, for bracket matching).
    pub(crate) has_focus: bool,
    /// Rows hidden by [`CodeEditor::set_hidden_areas`], besides folds.
    hidden_areas: Vec<Range<u32>>,
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
    /// Animations of the view (smooth scrolling, the caret, blinking).
    pub(crate) animation: crate::view::animation::ViewAnimation,
    pub(crate) cursor_visible: bool,
    /// IME composition range, in byte offsets.
    pub(crate) marked_range: Option<Range<usize>>,
    pub(crate) prev_edit_kind: Option<EditOperationType>,
    /// The reason and source the next `SelectionsChanged` reports.
    pub(crate) next_selection_change: SelectionChange,
    /// The inline message, if one is showing.
    pub(crate) message: crate::message::InlineMessage,
    /// Navigation commands in flight and the jump highlight.
    pub(crate) goto: crate::goto::GotoState,
    /// The peek view, if open.
    pub(crate) peek: crate::peek::PeekState,
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
        crate::model::register_model(cx, &model);
        crate::registry::track_editor(cx);
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
            cx.on_blur(&focus_handle, window, |this, _, cx| {
                this.has_focus = false;
                this.update_bracket_highlights(cx);
                this.mouse_selection = None;
                this.dnd_cancel(cx);
                this.hide_hover(cx);
                this.close_message(cx);
                this.inline_blurred(cx);
                cx.emit(CodeEditorEvent::Blurred);
                cx.notify();
            }),
            // Monaco's widget focus: the text or a widget inside the editor.
            cx.on_focus_in(&focus_handle, window, |_, _, cx| {
                cx.emit(CodeEditorEvent::WidgetFocused);
            }),
            cx.on_focus_out(&focus_handle, window, |_, _, _, cx| {
                cx.emit(CodeEditorEvent::WidgetBlurred);
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
        subscriptions.push(
            cx.observe_global::<crate::font_zoom::EditorZoom>(|this, cx| this.on_zoom_changed(cx)),
        );
        model.update(cx, |model, cx| model.ensure_syntax(cx));
        let snapshot = model.read(cx).snapshot();
        Self {
            model,
            snapshot: snapshot.clone(),
            focus_handle,
            cursors: CursorCollection::new(SingleCursorState::from_position(PointUtf16::zero())),
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
            animation: Default::default(),
            cursor_visible: true,
            marked_range: None,
            prev_edit_kind: None,
            next_selection_change: SelectionChange::default(),
            message: Default::default(),
            goto: Default::default(),
            peek: Default::default(),
            rename: Default::default(),
            code_actions: Default::default(),
            inlay: Default::default(),
            smart_select: Default::default(),
            in_peek: false,
            refocus: false,
            format_task: None,
            format_on_type_task: None,
            last_cursor_offset: None,
            pending_cursor_offset: None,
            mouse_selection: None,
            scrollbar_drag: None,
            scrollbar_arrow_task: None,
            blink_epoch: 0,
            view_lines: Arc::new(ViewLines::new(snapshot.rope().clone(), snapshot.version())),
            folds: FoldState::default(),
            gutter_hovered: false,
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
            sticky: Default::default(),
            colors: Default::default(),
            section_headers: Default::default(),
            color_picker: Default::default(),
            context_menu: Default::default(),
            minimap_cache: Default::default(),
            char_widths: None,
            wrap_classifier: None,
            minimap_hovered: false,
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
        if model == self.model {
            return;
        }
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
        self.minimap_cache = Default::default();
        self.cancel_selection_anchor(cx);
        self.reset_navigation_history();
        self.clear_occurrence_highlights(cx);
        self.bracket_colors = Default::default();

        self._subscriptions[0] = cx.subscribe(&model, Self::on_model_event);
        model.update(cx, |model, cx| model.ensure_syntax(cx));
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
        self.model = model;
        self.cursors = CursorCollection::new(SingleCursorState::from_position(PointUtf16::zero()));
        self.cursors
            .set_merge_overlapping(self.options.multi_cursor_merge_overlapping);
        self.scroll_top = px(0.);
        self.scroll_left = px(0.);
        self.pending_reveal = None;
        self.pending_reveal_x = None;
        self.pending_scroll_anchor = None;
        self.marked_range = None;
        self.prev_edit_kind = None;
        self.record_cursor_change();
        self.reattach_find_model(cx);
        cx.emit(CodeEditorEvent::SelectionsChanged(SelectionChange {
            reason: CursorChangeReason::ContentFlush,
            source: CursorChangeSource::Model,
        }));
        cx.emit(CodeEditorEvent::ScrollChanged);
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
        cx.emit(CodeEditorEvent::SelectionsChanged(SelectionChange {
            reason: CursorChangeReason::NotSet,
            source: CursorChangeSource::RestoreState,
        }));
        cx.emit(CodeEditorEvent::ScrollChanged);
        cx.notify();
    }

    /// The editor options.
    #[expect(
        clippy::misnamed_getters,
        reason = "`options` holds the zoomed values; hosts get theirs back"
    )]
    pub fn options(&self) -> &EditorOptions {
        &self.user_options
    }

    /// Replaces the editor options (Monaco `updateOptions`, but with a full set).
    pub fn update_options(&mut self, options: EditorOptions, cx: &mut Context<Self>) {
        let old_from_all_providers = self.options.occurrences_highlight_from_all_providers;
        let old = std::mem::replace(
            &mut self.options,
            crate::font_zoom::zoomed_options(&options, crate::font_zoom::editor_zoom_level(cx)),
        );
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
        self.theme = theme;
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
        cx.emit(CodeEditorEvent::ScrollChanged);
        cx.notify();
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
    /// (Monaco `getContentHeight`), not counting `scroll_beyond_last_line`
    /// (Monaco adds it; see [`CodeEditor::scroll_height`]).
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

    /// Hides whole rows besides folds (Monaco `setHiddenAreas`): each range
    /// is 0-based rows, end exclusive. Replaces the previous areas.
    pub fn set_hidden_areas(&mut self, areas: Vec<Range<u32>>, cx: &mut Context<Self>) {
        if self.hidden_areas != areas {
            self.hidden_areas = areas;
            cx.notify();
        }
    }

    /// The rows hidden by [`CodeEditor::set_hidden_areas`].
    pub fn hidden_areas(&self) -> &[Range<u32>] {
        &self.hidden_areas
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
        });
        self.widgets.zones_generation += 1;
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
        if let Some(zone) = self.widgets.zones.iter_mut().find(|z| z.id == id) {
            zone.height = height;
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
        });
        cx.notify();
        id
    }

    /// Removes a view zone, content widget or overlay widget.
    pub fn remove_widget(&mut self, id: WidgetId, cx: &mut Context<Self>) {
        if self.widgets.remove(id) {
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
    /// out (Monaco `getScrolledVisiblePosition`).
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

    /// The cursors of the last frame and whether they're outlined, for
    /// tests.
    #[doc(hidden)]
    pub fn cursor_shapes_for_tests(&self) -> (Vec<Bounds<Pixels>>, bool) {
        self.layout
            .as_ref()
            .map_or_else(|| (Vec::new(), false), |l| l.cursors.clone())
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
                ..
            } => {
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
                self.inline_completions_after_edit(cx);
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
                    self.marked_range = None;
                }
                self.find_content_changed(*kind == EditKind::SetValue, cx);
                if !own {
                    self.prev_edit_kind = None;
                    self.occurrences_after_edit(cx);
                }
                if !own || matches!(kind, EditKind::Undo | EditKind::Redo) {
                    self.next_selection_change = SelectionChange {
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
                cx.notify();
            }
            TextModelEvent::LanguageChanged {
                old_language,
                new_language,
            } => {
                // Monaco closes the peek view.
                self.close_peek(cx);
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
                cx.notify();
            }
            TextModelEvent::OptionsChanged
            | TextModelEvent::LineEndingChanged
            | TextModelEvent::DecorationsChanged => cx.notify(),
        }
    }

    /// Applies an edit computed by the cursor core: converts it to byte
    /// ranges, pushes it to the model with undo grouping and cursor metadata,
    /// then moves the cursors to the operation's resulting selections.
    /// Returns where the text of each of the operation's edits ended up, in
    /// byte offsets of the new text (empty if nothing was applied).
    pub(crate) fn apply(
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
        let group = undo_group(operation.kind);
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
        self.suggest_cursor_changed();
        self.write_selection_to_primary(cx);
        self.snippet_selections_changed(cx);
        // Monaco `multiCursorLimit`: extra cursors are dropped.
        let limit = self.options.multi_cursor_limit.max(1);
        if self.cursors.all().len() > limit {
            let kept = self.cursors.all()[..limit].to_vec();
            self.cursors.set_states(kept);
        }
        if autoscroll {
            self.autoscroll_requested = true;
            self.autoscroll_from_mouse =
                self.next_selection_change.source == CursorChangeSource::Mouse;
            self.animation.smooth_scroll_requested = true;
        }
        self.record_cursor_change();
        self.close_message(cx);
        self.check_symbol_navigation(cx);
        self.check_rename(cx);
        self.dismiss_keyboard_definition_link(cx);
        self.smart_select_cursor_changed();
        self.schedule_auto_code_actions(cx);
        self.update_bracket_highlights(cx);
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
        cx.emit(CodeEditorEvent::SelectionsChanged(change));
        cx.notify();
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
                // run 20 half-second iterations; repaint while they run.
                self._blink_task = Some(cx.spawn(async move |this, cx| {
                    let end = crate::view::animation::BLINK_ANIMATION_END;
                    let start = web_time::Instant::now();
                    while start.elapsed() < end {
                        cx.background_executor()
                            .timer(Duration::from_millis(16))
                            .await;
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

    // ----- navigation ---------------------------------------------------------

    fn move_cursors(&mut self, command: MoveCommand, select: bool, cx: &mut Context<Self>) {
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
            let page = self.suggest_page_size(cx) as isize;
            let delta = match command {
                MoveCommand::Up => Some(-1),
                MoveCommand::Down => Some(1),
                MoveCommand::PageUp => Some(-page),
                MoveCommand::PageDown => Some(page),
                _ => None,
            };
            if let Some(delta) = delta {
                if self.suggest_navigate(delta, cx) {
                    return;
                }
            } else {
                self.hide_suggest(cx);
            }
        }
        let config = self.cursor_config(cx);
        let page = self.page_lines();
        let before = self.cursors.selections();
        // Like Monaco, movement runs on view lines (wrapped segments, folded
        // rows skipped); edits run on the model.
        let lines = self.view_lines.clone();
        let to_view = |p: PointUtf16| {
            let (line, column) = lines.to_view(p, false);
            PointUtf16::new(line, column)
        };
        let to_model = |p: PointUtf16| lines.to_model(p.row, p.column);
        let mut view_cursors = self.cursors.clone();
        view_cursors.set_states(
            self.cursors
                .all()
                .iter()
                .map(|state| map_state(state, to_view))
                .collect(),
        );
        let model = ViewCursorModel(&lines);
        cursor::move_cursors(&config, &model, &mut view_cursors, command, select, page);
        self.cursors.set_states(
            view_cursors
                .all()
                .iter()
                .map(|state| map_state(state, to_model))
                .collect(),
        );
        self.cursors.validate(self.snapshot.rope());
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
        let config = self.cursor_config(cx);
        cursor::add_cursor_above(&config, self.snapshot.rope(), &mut self.cursors);
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
    }

    fn add_cursor_below(&mut self, _: &AddCursorBelow, _: &mut Window, cx: &mut Context<Self>) {
        let config = self.cursor_config(cx);
        cursor::add_cursor_below(&config, self.snapshot.rope(), &mut self.cursors);
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(true, cx);
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
        if self.inline_suggestion_visible(cx) || self.inline_edit_visible(cx) {
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
        if self.is_hover_visible() {
            self.hide_hover(cx);
            return;
        }
        if self.is_message_visible() {
            self.close_message(cx);
            return;
        }
        // Monaco's `closeReferenceSearch` outranks closing find.
        if self.close_peek(cx) {
            return;
        }
        if self.cancel_symbol_navigation(cx) {
            return;
        }
        // Monaco's `closeFindWidget` and `cancelSelectionAnchor` outrank
        // `removeSecondaryCursors`.
        if self.find.visible {
            self.close_find(window, cx);
            return;
        }
        if self.cancel_selection_anchor(cx) {
            return;
        }
        let primary = self.cursors.primary().clone();
        if self.cursors.all().len() > 1 {
            self.cursors.set_states(vec![primary]);
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
        cx.emit(CodeEditorEvent::WillType(typed.clone()));
        self.type_text_inner(text, cx);
        cx.emit(CodeEditorEvent::DidType(typed));
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
        self.run_edit(cx, cursor::delete_word_left);
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
        self.run_edit(cx, cursor::move_lines_up);
    }

    fn move_lines_down(&mut self, _: &MoveLinesDown, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::move_lines_down);
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
                .map(|language| language.config.comments.clone())
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
        self.run_edit(cx, cursor::insert_line_below);
    }

    fn insert_line_before(&mut self, _: &InsertLineBefore, _: &mut Window, cx: &mut Context<Self>) {
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
        self.model.update(cx, |model, cx| model.undo(source, cx));
    }

    /// Redoes the model's last undone step (Monaco `redo`), restoring the cursors
    /// from after it.
    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if self.options.read_only {
            return;
        }
        let source = Some(cx.entity_id());
        self.prev_edit_kind = None;
        self.model.update(cx, |model, cx| model.redo(source, cx));
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
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            text,
            ClipboardMetadata {
                is_from_empty_selection,
                multicursor_text,
                mode: mode.map(|mode| mode.to_string()),
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
    fn clipboard_metadata(item: &ClipboardItem) -> Option<ClipboardMetadata> {
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

    fn paste_item_inner(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
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
        self.next_selection_change = SelectionChange::default();
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
                .map(|r| r.start.min(row_count)..r.end.min(row_count)),
        );
        let hidden = normalize_hidden(hidden);
        if self.view_lines.hidden() != hidden {
            Arc::make_mut(&mut self.view_lines).set_hidden(hidden);
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
                this.fold_inputs(cx)
            }) else {
                return;
            };
            let (snapshot, version, providers, compute) = inputs;
            let line_count = snapshot.rope().line_count();
            // Monaco `collectSyntaxRanges`: valid ranges from every provider,
            // ranked by registration order.
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

    /// What computing fold ranges needs: the text, provider requests, and
    /// the rest of the computation for a background thread.
    #[allow(clippy::type_complexity)]
    fn fold_inputs(
        &mut self,
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
        let auto = self.options.folding_strategy == crate::options::FoldingStrategy::Auto;
        let limit = self.options.folding_maximum_regions;
        let context = model.provider_context();
        let syntax = if auto { model.syntax_snapshot() } else { None };
        let providers: Vec<_> = if auto {
            crate::providers::folding_range_providers(cx, self.provider_target(cx))
                .into_iter()
                .map(|provider| provider.provide_folding_ranges(&context, cx))
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
            self.next_selection_change = SelectionChange {
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
        let config = self.cursor_config(cx);
        let visible_column = self.mouse_visible_column(event.position);
        cursor::column_select_to(
            &config,
            self.snapshot.rope(),
            &mut self.cursors,
            position,
            visible_column,
            false,
        );
        // Dragging extends it.
        self.mouse_selection = Some(MouseSelection {
            mode: MouseSelectMode::Column,
            origin: Selection::collapsed(position),
            cursor_index: 0,
        });
        self.cursors_changed(false, cx);
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
        window.focus(&self.focus_handle, cx);
        let mouse = self.emit_mouse(
            event.position,
            Some(event.button),
            event.modifiers,
            event.click_count,
            window,
            cx,
        );
        cx.emit(CodeEditorEvent::MouseDown(mouse));
        cx.emit(CodeEditorEvent::ContextMenu(mouse));
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
        cx.emit(CodeEditorEvent::MouseDown(mouse));
        // Embedded views (zones and widgets) handle their own clicks and
        // focus.
        if self.layout.as_ref().is_some_and(|layout| {
            layout
                .widget_bounds
                .iter()
                .any(|bounds| bounds.contains(&event.position))
        }) {
            return;
        }
        window.focus(&self.focus_handle, cx);
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
            // Clicking outside the slider centers the clicked line; then drag.
            if !minimap.slider.contains(&event.position) {
                let line = ((event.position.y - minimap.bounds.top() + minimap.scroll)
                    / minimap.line_height)
                    .floor()
                    .max(0.);
                let visible = layout.text_bounds.size.height;
                self.scroll_top = (layout.metrics.line_height * line - visible / 2.).max(px(0.));
                self.minimap_drag = Some(minimap.slider.size.height / 2.);
            } else {
                self.minimap_drag = Some(event.position.y - minimap.slider.top());
            }
            cx.emit(CodeEditorEvent::ScrollChanged);
            cx.notify();
            return;
        }
        if let Some(ix) = layout
            .sticky_lines
            .iter()
            .position(|line| line.bounds.contains(&event.position))
        {
            self.sticky_click(ix, event, cx);
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
            // Monaco `scrollByPage`: the track pages towards the click.
            if !on_thumb && self.options.scrollbar.scroll_by_page {
                let page = if vertical {
                    layout.text_bounds.size.height
                } else {
                    layout.text_bounds.size.width
                };
                let delta = if along < thumb_start { -page } else { page };
                if vertical {
                    self.scroll_top += delta;
                } else {
                    self.scroll_left += delta;
                }
                self.clamp_scroll();
                cx.emit(CodeEditorEvent::ScrollChanged);
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
        // Monaco: a click on the selection may start dragging it.
        if event.button == MouseButton::Left
            && self.dnd_mouse_down(
                mouse.target.kind,
                position,
                &event.modifiers,
                event.click_count,
            )
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
        let config = self.cursor_config(cx);
        let mode = if in_gutter {
            MouseSelectMode::Line
        } else {
            match event.click_count {
                0 | 1 => MouseSelectMode::Character,
                2 => MouseSelectMode::Word,
                _ => MouseSelectMode::Line,
            }
        };
        let clicked = match mode {
            MouseSelectMode::Character | MouseSelectMode::Column => Selection::collapsed(position),
            MouseSelectMode::Word => self
                .block_selection_at(position, cx)
                .or_else(|| {
                    cursor::word_at(&config, self.snapshot.rope(), position).map(|range| {
                        Selection {
                            anchor: range.start,
                            head: range.end,
                        }
                    })
                })
                .unwrap_or_else(|| Selection::collapsed(position)),
            MouseSelectMode::Line => self.line_selection(position.row),
        };

        if ((event.modifiers.shift && event.modifiers.alt) || self.options.column_selection)
            && !in_gutter
        {
            let visible_column = self.mouse_visible_column(event.position);
            cursor::column_select_to(
                &config,
                self.snapshot.rope(),
                &mut self.cursors,
                position,
                visible_column,
                false,
            );
            self.mouse_selection = Some(MouseSelection {
                mode: MouseSelectMode::Column,
                origin: Selection::collapsed(position),
                cursor_index: 0,
            });
            self.next_selection_change = SelectionChange {
                reason: CursorChangeReason::Explicit,
                source: CursorChangeSource::Mouse,
            };
            self.cursors_changed(false, cx);
            return;
        }
        let add_cursor = self.is_add_cursor_modifier(&event.modifiers);
        let mut states: Vec<SingleCursorState> = self.cursors.all().to_vec();
        let cursor_index;
        if event.modifiers.shift && mode == MouseSelectMode::Character {
            let primary = &states[0];
            let anchor = primary.selection().anchor;
            states = vec![SingleCursorState::from_selection(Selection {
                anchor,
                head: position,
            })];
            cursor_index = 0;
        } else if add_cursor {
            states.push(SingleCursorState::from_selection(clicked));
            cursor_index = states.len() - 1;
        } else {
            states = vec![SingleCursorState::from_selection(clicked)];
            cursor_index = 0;
        }
        let origin = states[cursor_index].selection();
        self.mouse_selection = Some(MouseSelection {
            mode,
            origin,
            cursor_index,
        });
        self.cursors.set_states(states);
        self.prev_edit_kind = None;
        self.model.update(cx, |model, _| model.push_stack_element());
        self.next_selection_change = SelectionChange {
            reason: CursorChangeReason::Explicit,
            source: CursorChangeSource::Mouse,
        };
        self.cursors_changed(false, cx);
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

    fn line_selection(&self, row: u32) -> Selection {
        let max = self.snapshot.max_point_utf16();
        let end = if row < max.row {
            PointUtf16::new(row + 1, 0)
        } else {
            max
        };
        Selection {
            anchor: PointUtf16::new(row, 0),
            head: end,
        }
    }

    fn on_mouse_move(
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
        let was_over = over_thumb(
            self.animation.scrollbars.mouse_position,
            self.layout.as_ref(),
        );
        self.animation.scrollbars.mouse_position = Some(event.position);
        if was_over != over_thumb(Some(event.position), self.layout.as_ref()) {
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
        if minimap_hovered != self.minimap_hovered {
            self.minimap_hovered = minimap_hovered;
            cx.notify();
        }
        if let Some(grab) = self.minimap_drag {
            if event.pressed_button == Some(MouseButton::Left) {
                if let Some(minimap) = self.layout.as_ref().and_then(|layout| layout.minimap) {
                    let slider_top = event.position.y - grab - minimap.bounds.top();
                    self.scroll_top = (slider_top * minimap.drag_ratio).max(px(0.));
                    cx.emit(CodeEditorEvent::ScrollChanged);
                    cx.notify();
                }
                return;
            }
            self.minimap_drag = None;
        }
        let gutter_hovered = self.layout.as_ref().is_some_and(|layout| {
            layout.bounds.contains(&event.position) && event.position.x < layout.text_bounds.left()
        });
        if gutter_hovered != self.gutter_hovered {
            self.gutter_hovered = gutter_hovered;
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
                let position = match inlay_part {
                    Some(_) => None,
                    None => self.text_position_under_mouse(event.position, window, cx),
                };
                self.links_mouse_move(position, link_modifier, cx);
                if link_modifier && position.is_some() {
                    self.definition_mouse_move(position, cx);
                } else if self.has_definition_link() {
                    self.clear_definition_link(cx);
                }
                if let Some(part) = inlay_part {
                    self.hover_inlay_hint_part(part, cx);
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
        let Some(drag) = self.mouse_selection.as_ref() else {
            return;
        };
        // Middle drags column-select (Monaco).
        let middle_column = drag.mode == MouseSelectMode::Column
            && event.pressed_button == Some(MouseButton::Middle);
        if event.pressed_button != Some(MouseButton::Left) && !middle_column {
            self.mouse_selection = None;
            return;
        }
        let (mode, origin, index) = (drag.mode, drag.origin, drag.cursor_index);

        // Drag past the edges scrolls.
        if let Some(layout) = self.layout.as_ref() {
            let text = layout.text_bounds;
            let line_height = layout.metrics.line_height;
            if event.position.y < text.top() {
                self.scroll_top = (self.scroll_top - line_height).max(px(0.));
            } else if event.position.y > text.bottom() {
                self.scroll_top += line_height;
            }
        }

        let position = self.position_for_point(event.position, window, cx);
        let config = self.cursor_config(cx);
        if mode == MouseSelectMode::Column {
            let visible_column = self.mouse_visible_column(event.position);
            cursor::column_select_to(
                &config,
                self.snapshot.rope(),
                &mut self.cursors,
                position,
                visible_column,
                true,
            );
            self.next_selection_change = SelectionChange {
                reason: CursorChangeReason::Explicit,
                source: CursorChangeSource::Mouse,
            };
            self.cursors_changed(false, cx);
            return;
        }
        let selection = match mode {
            MouseSelectMode::Character => Selection {
                anchor: origin.anchor,
                head: position,
            },
            MouseSelectMode::Word => {
                let word = cursor::word_at(&config, self.snapshot.rope(), position)
                    .unwrap_or(position..position);
                if position < origin.start() {
                    Selection {
                        anchor: origin.end(),
                        head: word.start,
                    }
                } else {
                    Selection {
                        anchor: origin.start(),
                        head: word.end.max(origin.end()),
                    }
                }
            }
            MouseSelectMode::Column => return,
            MouseSelectMode::Line => {
                let line = self.line_selection(position.row);
                if position.row < origin.start().row {
                    Selection {
                        anchor: origin.end(),
                        head: line.anchor,
                    }
                } else {
                    Selection {
                        anchor: origin.start(),
                        head: line.head,
                    }
                }
            }
        };
        let mut states = self.cursors.all().to_vec();
        if let Some(state) = states.get_mut(index) {
            *state = SingleCursorState::from_selection(selection);
        }
        self.cursors.set_states(states);
        self.next_selection_change = SelectionChange {
            reason: CursorChangeReason::Explicit,
            source: CursorChangeSource::Mouse,
        };
        self.cursors_changed(false, cx);
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.scrollbar_arrow_task = None;
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
            cx.emit(CodeEditorEvent::ScrollChanged);
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
        cx.emit(CodeEditorEvent::ScrollChanged);
        cx.notify();
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        cx.emit(CodeEditorEvent::ScrollChanged);
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
            cx.emit(CodeEditorEvent::ScrollChanged);
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
        self.marked_range = None;
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
        let explicit = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked_range.take());
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
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.primary_selection_offsets().0);
        self.replace_range(range.clone(), new_text, cx);
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        if let Some(selected) = new_selected_range_utf16 {
            let base = self.offset_to_utf16(range.start);
            let start = self.offset_from_utf16(base + selected.start);
            let end = self.offset_from_utf16(base + selected.end);
            let selection = Selection {
                anchor: self.snapshot.offset_to_point_utf16(start),
                head: self.snapshot.offset_to_point_utf16(end),
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
        }
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
        root.on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::expand_line_selection))
            .on_action(cx.listener(Self::add_cursor_above))
            .on_action(cx.listener(Self::add_cursor_below))
            .on_action(cx.listener(Self::remove_secondary_cursors))
            .on_action(cx.listener(Self::delete_left))
            .on_action(cx.listener(Self::delete_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(|this, _: &DeleteWordStartLeft, _, cx| {
                this.run_edit(cx, |c, m, s| {
                    cursor::delete_word(c, m, s, true, cursor::WordNavigationType::WordStart, false)
                })
            }))
            .on_action(cx.listener(|this, _: &DeleteWordEndLeft, _, cx| {
                this.run_edit(cx, |c, m, s| {
                    cursor::delete_word(c, m, s, true, cursor::WordNavigationType::WordEnd, false)
                })
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
                this.run_edit(cx, |c, m, s| cursor::delete_word_part(c, m, s, true))
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
            .on_action(cx.listener(|this, _: &Fold, _, cx| this.fold(cx)))
            .on_action(cx.listener(|this, _: &Unfold, _, cx| this.unfold(cx)))
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
                this.select_all_matches(cx);
                this.close_find(window, cx);
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
            .on_action(
                cx.listener(|this, _: &ShowHover, window, cx| this.show_or_focus_hover(window, cx)),
            )
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
            .on_action(
                cx.listener(|this, _: &TriggerInlineSuggest, _, cx| {
                    this.trigger_inline_suggest(cx)
                }),
            )
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
            .on_action(cx.listener(|this, _: &PasteAs, window, cx| this.paste_as(None, window, cx)))
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
            .on_action(cx.listener(|this, _: &Refactor, _, cx| this.refactor(cx)))
            .on_action(cx.listener(|this, _: &SourceAction, _, cx| this.source_action(cx)))
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
                let page = this.suggest_page_size(cx) as isize;
                this.suggest_navigate(page, cx);
            }))
            .on_action(
                cx.listener(|this, _: &SelectPreviousPageSuggestion, _, cx| {
                    let page = this.suggest_page_size(cx) as isize;
                    this.suggest_navigate(-page, cx);
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
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(EditorElement::new(cx.entity()))
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
        self.0.line_content(row).encode_utf16().count() as u32
    }
}
