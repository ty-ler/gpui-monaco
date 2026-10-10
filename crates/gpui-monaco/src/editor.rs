//! [`CodeEditor`]: the editor view entity.

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
use crate::events::{CursorChangeReason, CursorChangeSource, SelectionChange};
use crate::model::{EditKind, TextModel, TextModelEvent, metadata};
use crate::options::EditorOptions;
use crate::theme::{EditorColors, Theme};
use crate::view::folding::{FoldState, compute_indent_ranges};
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
    /// The editor gained focus (Monaco `onDidFocusEditorText`).
    Focused,
    /// The editor lost focus (Monaco `onDidBlurEditorText`).
    Blurred,
    /// Go to definition led to another model: the host should open it.
    OpenLocation(crate::providers::Location),
    /// A mouse button went down over the editor (Monaco `onMouseDown`).
    MouseDown(crate::mouse_target::EditorMouseEvent),
    /// A mouse button went up (Monaco `onMouseUp`).
    MouseUp(crate::mouse_target::EditorMouseEvent),
    /// The mouse moved over the editor (Monaco `onMouseMove`).
    MouseMove(crate::mouse_target::EditorMouseEvent),
    /// The mouse left the editor (Monaco `onMouseLeave`).
    MouseLeave,
    /// The context menu was requested with a right click (Monaco
    /// `onContextMenu`); the editor shows none itself.
    ContextMenu(crate::mouse_target::EditorMouseEvent),
}

/// Saved per-model view state (Monaco `saveViewState`/`restoreViewState`).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    /// The selections; the first is the primary one.
    pub selections: Vec<Selection>,
    /// Vertical scroll offset in pixels.
    pub scroll_top: Pixels,
    /// Horizontal scroll offset in pixels.
    pub scroll_left: Pixels,
}

/// Clipboard metadata so pastes can reproduce Monaco's empty-selection and
/// multi-cursor behavior.
#[derive(Serialize, Deserialize)]
struct ClipboardMetadata {
    is_from_empty_selection: bool,
    multicursor_text: Option<Vec<String>>,
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
enum ScrollbarDrag {
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
    pub(crate) options: EditorOptions,
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
    pub(crate) pending_reveal: Option<(Range<PointUtf16>, crate::view::reveal::RevealType)>,
    /// Where the next prepaint scrolls horizontally to (after a reveal).
    pub(crate) pending_reveal_x: Option<PointUtf16>,
    /// Monaco's `overtype` input mode (Insert key).
    pub(crate) overtype: bool,
    pub(crate) hover: crate::hover::HoverState,
    pub(crate) marker_navigation: crate::markers::MarkerNavigation,
    pub(crate) definition: crate::definition::DefinitionLink,
    pub(crate) signature_help: crate::signature_help::SignatureHelpState,
    pub(crate) suggest: crate::completion::SuggestState,
    pub(crate) minimap_cache: crate::element::MinimapCache,
    pub(crate) minimap_hovered: bool,
    minimap_drag: Option<Pixels>,
    fold_task: Option<Task<()>>,
    pub(crate) autoscroll_requested: bool,
    pub(crate) cursor_visible: bool,
    /// IME composition range, in byte offsets.
    marked_range: Option<Range<usize>>,
    pub(crate) prev_edit_kind: Option<EditOperationType>,
    /// The reason and source the next `SelectionsChanged` reports.
    pub(crate) next_selection_change: SelectionChange,
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
    scrollbar_drag: Option<ScrollbarDrag>,
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
        let focus_handle = cx.focus_handle();
        let subscriptions = vec![
            cx.subscribe(&model, Self::on_model_event),
            cx.on_focus(&focus_handle, window, |this, _, cx| {
                this.restart_blink(cx);
                cx.emit(CodeEditorEvent::Focused);
                cx.notify();
            }),
            cx.on_blur(&focus_handle, window, |this, _, cx| {
                this.mouse_selection = None;
                this.hide_hover(cx);
                cx.emit(CodeEditorEvent::Blurred);
                cx.notify();
            }),
        ];
        model.update(cx, |model, cx| model.ensure_syntax(cx));
        let snapshot = model.read(cx).snapshot();
        Self {
            model,
            snapshot: snapshot.clone(),
            focus_handle,
            cursors: CursorCollection::new(SingleCursorState::from_position(PointUtf16::zero())),
            options: EditorOptions::default(),
            theme: Arc::new(Theme::vs_dark()),
            scroll_top: px(0.),
            scroll_left: px(0.),
            layout: None,
            autoscroll_requested: false,
            cursor_visible: true,
            marked_range: None,
            prev_edit_kind: None,
            next_selection_change: SelectionChange::default(),
            format_task: None,
            format_on_type_task: None,
            last_cursor_offset: None,
            pending_cursor_offset: None,
            mouse_selection: None,
            scrollbar_drag: None,
            blink_epoch: 0,
            view_lines: Arc::new(ViewLines::new(snapshot.rope().clone(), snapshot.version())),
            folds: FoldState::default(),
            gutter_hovered: false,
            widgets: Widgets::default(),
            find: Default::default(),
            multi_cursor: None,
            bracket_decorations: Vec::new(),
            overtype: false,
            occurrences: Default::default(),
            bracket_colors: Default::default(),
            pending_reveal: None,
            pending_reveal_x: None,
            navigation: crate::navigation_history::NavigationHistory::starting_at(
                vec![Selection::collapsed(PointUtf16::zero())],
                snapshot.version(),
            ),
            hover: Default::default(),
            marker_navigation: Default::default(),
            definition: Default::default(),
            signature_help: Default::default(),
            suggest: Default::default(),
            minimap_cache: Default::default(),
            minimap_hovered: false,
            minimap_drag: None,
            fold_task: None,
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
        // Popups and decorations this editor owns refer to the old model: its
        // decoration ids would name unrelated decorations in the new one.
        self.hide_hover(cx);
        self.hide_suggest(cx);
        self.hide_signature_help(cx);
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
        self.snapshot = model.read(cx).snapshot();
        self.view_lines = Arc::new(ViewLines::new(
            self.snapshot.rope().clone(),
            self.snapshot.version(),
        ));
        self.folds = FoldState::default();
        self.fold_task = None;
        self.model = model;
        self.cursors = CursorCollection::new(SingleCursorState::from_position(PointUtf16::zero()));
        self.cursors
            .set_merge_overlapping(self.options.multi_cursor_merge_overlapping);
        self.scroll_top = px(0.);
        self.scroll_left = px(0.);
        self.marked_range = None;
        self.prev_edit_kind = None;
        self.record_cursor_change();
        self.refresh_find_matches(cx);
        cx.emit(CodeEditorEvent::SelectionsChanged(SelectionChange {
            reason: CursorChangeReason::ContentFlush,
            source: CursorChangeSource::Model,
        }));
        cx.emit(CodeEditorEvent::ScrollChanged);
        cx.notify();
    }

    /// Captures the selections and scroll position (Monaco `saveViewState`).
    pub fn save_view_state(&self) -> ViewState {
        ViewState {
            selections: self.cursors.selections(),
            scroll_top: self.scroll_top,
            scroll_left: self.scroll_left,
        }
    }

    /// Restores a [`ViewState`] from [`CodeEditor::save_view_state`], clamping
    /// selections to the current text (Monaco `restoreViewState`). Emits
    /// [`CodeEditorEvent::SelectionsChanged`].
    pub fn restore_view_state(&mut self, state: &ViewState, cx: &mut Context<Self>) {
        self.cursors.set_selections(&state.selections);
        self.cursors.validate(self.snapshot.rope());
        self.scroll_top = state.scroll_top;
        self.scroll_left = state.scroll_left;
        cx.emit(CodeEditorEvent::SelectionsChanged(SelectionChange {
            reason: CursorChangeReason::NotSet,
            source: CursorChangeSource::RestoreState,
        }));
        cx.emit(CodeEditorEvent::ScrollChanged);
        cx.notify();
    }

    /// The editor options.
    pub fn options(&self) -> &EditorOptions {
        &self.options
    }

    /// Replaces the editor options (Monaco `updateOptions`, but with a full set).
    pub fn update_options(&mut self, options: EditorOptions, cx: &mut Context<Self>) {
        self.options = options;
        self.cursors
            .set_merge_overlapping(self.options.multi_cursor_merge_overlapping);
        self.bracket_colors = Default::default();
        self.refresh_bracket_colors(cx);
        cx.notify();
    }

    /// The current theme's chrome colors.
    pub fn colors(&self) -> &EditorColors {
        &self.theme.colors
    }

    /// The current theme. Defaults to [`Theme::vs_dark`].
    pub fn theme(&self) -> &Arc<Theme> {
        &self.theme
    }

    /// Sets the theme (Monaco `editor.setTheme`).
    pub fn set_theme(&mut self, theme: Arc<Theme>, cx: &mut Context<Self>) {
        self.theme = theme;
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

    /// Scrolls to `position` in pixels, clamped to the scroll range, and emits
    /// [`CodeEditorEvent::ScrollChanged`].
    pub fn set_scroll_position(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.scroll_left = position.x;
        self.scroll_top = position.y;
        self.clamp_scroll();
        cx.emit(CodeEditorEvent::ScrollChanged);
        cx.notify();
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

    /// Inserts a host view between lines (Monaco `changeViewZones` +
    /// `addZone`).
    pub fn add_view_zone(&mut self, zone: ViewZone, cx: &mut Context<Self>) -> WidgetId {
        let id = self.widgets.next_id();
        let buffer = self.model.read(cx).buffer();
        let anchor = (zone.after_line > 0).then(|| {
            let row = (zone.after_line - 1).min(buffer.max_point().row);
            buffer.anchor_before(buffer.point_to_offset(text::Point::new(row, u32::MAX)))
        });
        self.widgets.zones.push(ZoneState {
            id,
            anchor,
            height: zone.height,
            view: zone.view,
        });
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
                layout.sticky_lines.iter().map(|(row, _)| *row).collect(),
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

    pub(crate) fn cursor_config(&self, cx: &App) -> CursorConfig {
        let model_options = self.model.read(cx).options();
        CursorConfig {
            tab_size: model_options.tab_size,
            indent_size: model_options.indent_size,
            insert_spaces: model_options.insert_spaces,
            word_separators: self.options.word_separators.to_string(),
            sticky_tab_stops: self.options.sticky_tab_stops,
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
            ..CursorConfig::default()
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
                ..
            } => {
                let own = *source == Some(cx.entity_id());
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
                self.refresh_find_matches(cx);
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
                cx.notify();
            }
            TextModelEvent::OptionsChanged
            | TextModelEvent::LanguageChanged
            | TextModelEvent::LineEndingChanged
            | TextModelEvent::DecorationsChanged => cx.notify(),
        }
    }

    /// Applies an edit computed by the cursor core: converts it to byte
    /// ranges, pushes it to the model with undo grouping and cursor metadata,
    /// then moves the cursors to the operation's resulting selections.
    pub(crate) fn apply(&mut self, operation: EditOperation, cx: &mut Context<Self>) {
        self.hide_hover(cx);
        if operation.edits.is_empty() {
            if operation.selections_after != self.cursors.selections() {
                self.cursors.set_selections(&operation.selections_after);
                self.cursors_changed(true, cx);
            }
            return;
        }
        if self.options.read_only {
            return;
        }
        let snapshot = self.snapshot.clone();
        let edits: Vec<(Range<usize>, String)> = operation
            .edits
            .iter()
            .map(|(range, text)| {
                (
                    snapshot.point_utf16_to_offset(range.start)
                        ..snapshot.point_utf16_to_offset(range.end),
                    text.clone(),
                )
            })
            .collect();
        let before = metadata(self.cursors.selections());
        let after = metadata(operation.selections_after.clone());
        let source = cx.entity_id();
        let group = undo_group(operation.kind);
        let result = self.model.update(cx, |model, cx| {
            if operation.push_stack_element_before {
                model.push_stack_element();
            }
            let result = model.edit_from(edits, group, Some(source), cx);
            if result.is_ok() {
                model.set_undo_metadata(Some(before), Some(after));
            }
            if operation.push_stack_element_after {
                model.push_stack_element();
            }
            result
        });
        if result.is_err() {
            return;
        }
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
    fn is_definition_modifier(&self, modifiers: &gpui::Modifiers) -> bool {
        match self.options.multi_cursor_modifier {
            crate::options::MultiCursorModifier::Alt => modifiers.secondary(),
            crate::options::MultiCursorModifier::CtrlCmd => modifiers.alt,
        }
    }

    fn cursors_changed(&mut self, autoscroll: bool, cx: &mut Context<Self>) {
        // Monaco `multiCursorLimit`: extra cursors are dropped.
        let limit = self.options.multi_cursor_limit.max(1);
        if self.cursors.all().len() > limit {
            let kept = self.cursors.all()[..limit].to_vec();
            self.cursors.set_states(kept);
        }
        if autoscroll {
            self.autoscroll_requested = true;
        }
        self.record_cursor_change();
        self.update_bracket_highlights(cx);
        self.update_occurrence_highlights(cx);
        self.restart_blink(cx);
        let change = std::mem::take(&mut self.next_selection_change);
        // Monaco `ParameterHintsModel.onCursorChange`: clicks close parameter
        // hints, other changes re-query them while they're shown.
        if change.source == CursorChangeSource::Mouse {
            self.hide_signature_help(cx);
        } else if self.is_signature_help_visible() {
            self.trigger_signature_help(crate::providers::SignatureHelpTrigger::ContentChange, cx);
        }
        cx.emit(CodeEditorEvent::SelectionsChanged(change));
        cx.notify();
    }

    fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_visible = true;
        self.blink_epoch += 1;
        if !self.options.cursor_blinking {
            self._blink_task = None;
            return;
        }
        let epoch = self.blink_epoch;
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
            let page = crate::suggest_widget::page_size() as isize;
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
        if !select
            && matches!(command, MoveCommand::Up | MoveCommand::Down)
            && self.cycle_signature(if command == MoveCommand::Up { -1 } else { 1 }, cx)
        {
            return;
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

    fn remove_secondary_cursors(
        &mut self,
        _: &RemoveSecondaryCursors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.options.single_line {
            cx.propagate();
            return;
        }
        if self.is_suggest_visible() {
            self.hide_suggest(cx);
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

    /// Types `text` at every cursor, as if from the keyboard.
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
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
                let context = cursor::LanguageContext::new(&language.config, &token_type);
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
        let operation = self.with_language(cx, |config, rope, cursors, lang| {
            cursor::tab_with_language(config, rope, cursors, lang)
        });
        self.apply(operation, cx);
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        if self.options.single_line {
            cx.propagate();
            return;
        }
        self.run_edit(cx, cursor::outdent);
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if self.options.single_line {
            cx.propagate();
            return;
        }
        if self.is_suggest_visible() && self.accept_suggestion(cx) {
            return;
        }
        let operation = self.with_language(cx, |config, rope, cursors, lang| {
            cursor::line_break_with_language(config, rope, cursors, lang)
        });
        self.apply(operation, cx);
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
        self.write_selection_to_clipboard(cx);
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if self.write_selection_to_clipboard(cx) {
            self.run_edit(cx, cursor::cut);
        }
    }

    fn write_selection_to_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        let config = self.cursor_config(cx);
        let (text, is_from_empty_selection, per_cursor) =
            cursor::copy_text(&config, self.snapshot.rope(), &self.cursors);
        if text.is_empty() && !is_from_empty_selection {
            return false;
        }
        let multicursor_text = (per_cursor.len() > 1).then_some(per_cursor);
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            text,
            ClipboardMetadata {
                is_from_empty_selection,
                multicursor_text,
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

    fn paste_item(&mut self, item: ClipboardItem, cx: &mut Context<Self>) {
        let version = self.snapshot.version();
        let start = self
            .cursors
            .selections()
            .iter()
            .map(|s| s.start())
            .min()
            .unwrap_or_default();
        self.next_selection_change.reason = CursorChangeReason::Paste;
        self.paste_clipboard_item(item, cx);
        // Nothing may have changed.
        self.next_selection_change = SelectionChange::default();
        if self.snapshot.version() != version {
            // Monaco's `onDidPaste` range: from the first selection's start
            // to the last cursor.
            let end = self
                .cursors
                .selections()
                .iter()
                .map(|s| s.head)
                .max()
                .unwrap_or(start);
            self.format_on_paste(start..end, cx);
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
        let metadata = item.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::String(string) => string.metadata_json::<ClipboardMetadata>(),
            _ => None,
        });
        let (from_empty, multicursor) = match metadata {
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
        let hidden = if self.options.folding {
            normalize_hidden(self.folds.hidden_rows(self.model.read(cx).buffer()))
        } else {
            Vec::new()
        };
        if self.view_lines.hidden() != hidden {
            Arc::make_mut(&mut self.view_lines).set_hidden(hidden);
        }
        if self.options.folding
            && self.folds.ranges_version() != Some(self.snapshot.version())
            && self.fold_task.is_none()
        {
            self.compute_fold_ranges(cx);
        }
    }

    fn compute_fold_ranges(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.snapshot.clone();
        let model = self.model.read(cx);
        let tab_size = model.options().tab_size;
        let config = model
            .language()
            .map(|language| language.config.folding.clone());
        self.fold_task = Some(cx.spawn(async move |this, cx| {
            let version = snapshot.version();
            let ranges = cx
                .background_executor()
                .spawn(async move {
                    let folding = config.flatten();
                    let off_side = folding.as_ref().is_some_and(|f| f.off_side);
                    let markers = folding
                        .as_ref()
                        .and_then(|f| f.markers.as_ref())
                        .map(|m| (&m.start, &m.end));
                    compute_indent_ranges(snapshot.rope(), tab_size, off_side, markers)
                })
                .await;
            this.update(cx, |this, cx| {
                this.folds.set_ranges(ranges, version);
                this.fold_task = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Collapses the innermost expanded region containing each cursor
    /// (Monaco `editor.fold`).
    pub fn fold(&mut self, cx: &mut Context<Self>) {
        let buffer = self.model.read(cx).buffer().clone();
        let rows: Vec<u32> = self
            .cursors
            .selections()
            .iter()
            .map(|s| s.head.row)
            .collect();
        let collapsed = self.folds.collapsed_rows(&buffer);
        for row in rows {
            let candidate = self
                .folds
                .ranges()
                .iter()
                .filter(|r| r.start_row <= row && row <= r.end_row)
                .filter(|r| !collapsed.iter().any(|(start, _)| *start == r.start_row))
                .max_by_key(|r| r.start_row)
                .cloned();
            if let Some(range) = candidate {
                self.folds.collapse(&buffer, range.start_row, range.end_row);
            }
        }
        self.after_fold_change(cx);
    }

    /// Expands collapsed regions at or containing each cursor's row (Monaco
    /// `editor.unfold`).
    pub fn unfold(&mut self, cx: &mut Context<Self>) {
        let buffer = self.model.read(cx).buffer().clone();
        let rows: Vec<u32> = self
            .cursors
            .selections()
            .iter()
            .map(|s| s.head.row)
            .collect();
        for (start, end) in self.folds.collapsed_rows(&buffer) {
            if rows.iter().any(|row| start <= *row && *row <= end) {
                self.folds.expand(&buffer, start);
            }
        }
        self.after_fold_change(cx);
    }

    /// Toggles the region starting at `row` (gutter chevron click).
    pub fn toggle_fold_at(&mut self, row: u32, cx: &mut Context<Self>) {
        let buffer = self.model.read(cx).buffer().clone();
        if !self.folds.expand(&buffer, row)
            && let Some(range) = self.folds.range_starting_at(row).cloned()
        {
            self.folds.collapse(&buffer, range.start_row, range.end_row);
        }
        self.after_fold_change(cx);
    }

    /// Collapses every foldable region (Monaco `editor.foldAll`).
    pub fn fold_all(&mut self, cx: &mut Context<Self>) {
        let buffer = self.model.read(cx).buffer().clone();
        self.folds.collapse_all(&buffer);
        self.after_fold_change(cx);
    }

    /// Expands every collapsed region (Monaco `editor.unfoldAll`).
    pub fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.folds.expand_all();
        self.after_fold_change(cx);
    }

    /// Moves cursors out of hidden rows to the end of their fold's start row.
    fn after_fold_change(&mut self, cx: &mut Context<Self>) {
        let buffer = self.model.read(cx).buffer().clone();
        let collapsed = self.folds.collapsed_rows(&buffer);
        let fix = |p: PointUtf16| match collapsed
            .iter()
            .find(|(start, end)| *start < p.row && p.row <= *end)
        {
            Some((start, _)) => PointUtf16::new(*start, self.snapshot.line_len_utf16(*start)),
            None => p,
        };
        let selections: Vec<Selection> = self
            .cursors
            .selections()
            .into_iter()
            .map(|s| Selection {
                anchor: fix(s.anchor),
                head: fix(s.head),
            })
            .collect();
        self.cursors.set_selections(&selections);
        self.cursors_changed(true, cx);
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

    fn on_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let Some(layout) = self.layout.clone() else {
            return;
        };
        if let Some(minimap) = layout.minimap
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
        if let Some((row, _)) = layout
            .sticky_lines
            .iter()
            .find(|(_, bounds)| bounds.contains(&event.position))
        {
            // Monaco reveals the clicked scope header at the top.
            let row = *row;
            let ix = layout
                .sticky_lines
                .iter()
                .position(|(r, _)| *r == row)
                .unwrap_or(0);
            let line = self.view_lines.first_view_line_of_row(row);
            self.scroll_top = (layout.zones.line_y(line, layout.metrics.line_height)
                - layout.metrics.line_height * ix as f32)
                .max(px(0.));
            self.set_selections(&[Selection::collapsed(PointUtf16::new(row, 0))], cx);
            self.autoscroll_requested = false;
            return;
        }
        if let Some(vbar) = layout.vertical_scrollbar
            && vbar.track.contains(&event.position)
        {
            let grab_offset = if vbar.thumb.contains(&event.position) {
                event.position.y - vbar.thumb.top()
            } else {
                vbar.thumb.size.height / 2.
            };
            self.scrollbar_drag = Some(ScrollbarDrag::Vertical { grab_offset });
            self.drag_scrollbar(event.position, cx);
            return;
        }
        if let Some(hbar) = layout.horizontal_scrollbar
            && hbar.track.contains(&event.position)
        {
            let grab_offset = if hbar.thumb.contains(&event.position) {
                event.position.x - hbar.thumb.left()
            } else {
                hbar.thumb.size.width / 2.
            };
            self.scrollbar_drag = Some(ScrollbarDrag::Horizontal { grab_offset });
            self.drag_scrollbar(event.position, cx);
            return;
        }

        if layout
            .widget_bounds
            .iter()
            .any(|bounds| bounds.contains(&event.position))
        {
            return;
        }
        if let Some(control) = layout
            .fold_controls
            .iter()
            .find(|control| control.bounds.contains(&event.position))
        {
            self.toggle_fold_at(control.row, cx);
            return;
        }

        let position = self.position_for_point(event.position, window, cx);
        if self.is_definition_modifier(&event.modifiers)
            && self.follow_definition_link(position, cx)
        {
            return;
        }
        self.hide_suggest(cx);
        let in_gutter = event.position.x < layout.text_bounds.left();
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
            MouseSelectMode::Word => cursor::word_at(&config, self.snapshot.rope(), position)
                .map(|range| Selection {
                    anchor: range.start,
                    head: range.end,
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
        if self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.bounds.contains(&event.position))
        {
            let mouse = self.emit_mouse(event.position, None, event.modifiers, 0, window, cx);
            cx.emit(CodeEditorEvent::MouseMove(mouse));
        }
        let minimap_hovered = self
            .layout
            .as_ref()
            .and_then(|layout| layout.minimap)
            .is_some_and(|minimap| minimap.bounds.contains(&event.position));
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
            let over_widget = self.layout.as_ref().is_some_and(|layout| {
                layout
                    .widget_bounds
                    .iter()
                    .any(|bounds| bounds.contains(&event.position))
            });
            if !over_widget {
                let position = self.text_position_under_mouse(event.position, window, cx);
                if self.is_definition_modifier(&event.modifiers) {
                    self.definition_mouse_move(position, cx);
                } else if self.has_definition_link() {
                    self.clear_definition_link(cx);
                }
                self.hover_mouse_move(position, cx);
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
        let Some(drag) = self.mouse_selection.as_ref() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
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
            cx.emit(CodeEditorEvent::MouseUp(mouse));
        }
        self.minimap_drag = None;
        self.mouse_selection = None;
        self.scrollbar_drag = None;
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
        let mut delta =
            event.delta.pixel_delta(line_height) * self.options.mouse_wheel_scroll_sensitivity;
        if event.modifiers.shift && delta.x == px(0.) {
            delta = point(delta.y, px(0.));
        }
        self.hide_hover(cx);
        self.scroll_top -= delta.y;
        self.scroll_left -= delta.x;
        self.clamp_scroll();
        cx.emit(CodeEditorEvent::ScrollChanged);
        cx.notify();
        cx.stop_propagation();
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
            gpui::size(px(1.), layout.metrics.line_height),
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut key_context = gpui::KeyContext::new_with_defaults();
        key_context.add(KEY_CONTEXT);
        if self.options.single_line {
            key_context.add("single_line");
        }
        let root = div()
            .id("code-editor")
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .size_full()
            .cursor(if self.has_definition_link() {
                MouseCursor::PointingHand
            } else {
                MouseCursor::IBeam
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
            .on_action(cx.listener(|this, _: &ToggleFold, _, cx| {
                let row = this.cursors.primary().position.row;
                let buffer = this.model.read(cx).buffer().clone();
                let collapsed = this.folds.collapsed_rows(&buffer);
                if let Some((start, _)) = collapsed.iter().find(|(s, e)| *s <= row && row <= *e) {
                    this.toggle_fold_at(*start, cx);
                } else {
                    this.fold(cx);
                }
            }))
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
            .on_action(cx.listener(|this, _: &NextMatch, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.find_next(cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousMatch, _, cx| {
                if this.options.single_line {
                    return cx.propagate();
                }
                this.find_previous(cx)
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
                cx.listener(|this, _: &GoToPreviousMarker, _, cx| this.go_to_marker(false, cx)),
            )
            .on_action(cx.listener(|this, _: &GoToDefinition, _, cx| this.go_to_definition(cx)))
            .on_action(cx.listener(|this, _: &ShowHover, _, cx| this.show_hover_at_cursor(cx)))
            .on_action(cx.listener(|this, _: &FormatDocument, _, cx| {
                this.format_document(cx);
            }))
            .on_action(cx.listener(|this, _: &FormatSelection, _, cx| {
                this.format_selection(cx);
            }))
            .on_action(cx.listener(|this, _: &TriggerSuggest, _, cx| {
                this.trigger_suggest(crate::providers::CompletionTrigger::Invoke, cx)
            }))
            .on_action(cx.listener(|this, _: &TriggerParameterHints, _, cx| {
                this.trigger_signature_help(crate::providers::SignatureHelpTrigger::Invoke, cx)
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
            .on_hover(cx.listener(|_, hovered: &bool, _, cx| {
                if !*hovered {
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
