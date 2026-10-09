//! [`CodeEditor`]: the editor view entity.

use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, CursorStyle as MouseCursor, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, ScrollWheelEvent, Styled, Subscription, Task, UTF16Selection, Window, div, point, px,
};
use serde::{Deserialize, Serialize};
use text::{Bias, BufferSnapshot, OffsetUtf16, PointUtf16};

use crate::actions::*;
use crate::cursor::{
    self, CursorCollection, CursorConfig, EditOperation, EditOperationType, MoveCommand, Selection,
    SingleCursorState,
};
use crate::element::{EditorElement, EditorLayout, TextMetrics, layout_row};
use crate::model::{EditKind, TextModel, TextModelEvent, metadata};
use crate::options::EditorOptions;
use crate::theme::EditorColors;

/// Events emitted by [`CodeEditor`].
#[derive(Clone, Debug, PartialEq)]
pub enum CodeEditorEvent {
    /// The cursors or selections changed (Monaco `onDidChangeCursorSelection`).
    SelectionsChanged,
    /// The scroll position changed (Monaco `onDidScrollChange`).
    ScrollChanged,
    Focused,
    Blurred,
}

/// Saved per-model view state (Monaco `saveViewState`/`restoreViewState`).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    pub selections: Vec<Selection>,
    pub scroll_top: Pixels,
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
    pub(crate) colors: EditorColors,
    pub(crate) scroll_top: Pixels,
    pub(crate) scroll_left: Pixels,
    pub(crate) layout: Option<EditorLayout>,
    pub(crate) autoscroll_requested: bool,
    pub(crate) cursor_visible: bool,
    /// IME composition range, in byte offsets.
    marked_range: Option<Range<usize>>,
    prev_edit_kind: Option<EditOperationType>,
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
                cx.emit(CodeEditorEvent::Blurred);
                cx.notify();
            }),
        ];
        let snapshot = model.read(cx).snapshot();
        Self {
            model,
            snapshot,
            focus_handle,
            cursors: CursorCollection::new(SingleCursorState::from_position(PointUtf16::zero())),
            options: EditorOptions::default(),
            colors: EditorColors::default(),
            scroll_top: px(0.),
            scroll_left: px(0.),
            layout: None,
            autoscroll_requested: false,
            cursor_visible: true,
            marked_range: None,
            prev_edit_kind: None,
            mouse_selection: None,
            scrollbar_drag: None,
            blink_epoch: 0,
            _blink_task: None,
            _subscriptions: subscriptions,
        }
    }

    /// Creates an editor with a new model holding `text`.
    pub fn with_text(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = cx.new(|_| TextModel::new(text));
        Self::new(model, window, cx)
    }

    pub fn model(&self) -> &Entity<TextModel> {
        &self.model
    }

    /// Shows a different model, resetting cursors and scroll (Monaco
    /// `setModel`). Save and restore [`ViewState`] to keep them per model.
    pub fn set_model(&mut self, model: Entity<TextModel>, cx: &mut Context<Self>) {
        self._subscriptions[0] = cx.subscribe(&model, Self::on_model_event);
        self.snapshot = model.read(cx).snapshot();
        self.model = model;
        self.cursors = CursorCollection::new(SingleCursorState::from_position(PointUtf16::zero()));
        self.scroll_top = px(0.);
        self.scroll_left = px(0.);
        self.marked_range = None;
        self.prev_edit_kind = None;
        cx.emit(CodeEditorEvent::SelectionsChanged);
        cx.notify();
    }

    pub fn save_view_state(&self) -> ViewState {
        ViewState {
            selections: self.cursors.selections(),
            scroll_top: self.scroll_top,
            scroll_left: self.scroll_left,
        }
    }

    pub fn restore_view_state(&mut self, state: &ViewState, cx: &mut Context<Self>) {
        self.cursors.set_selections(&state.selections);
        self.cursors.validate(self.snapshot.rope());
        self.scroll_top = state.scroll_top;
        self.scroll_left = state.scroll_left;
        cx.emit(CodeEditorEvent::SelectionsChanged);
        cx.notify();
    }

    pub fn options(&self) -> &EditorOptions {
        &self.options
    }

    pub fn update_options(&mut self, options: EditorOptions, cx: &mut Context<Self>) {
        self.options = options;
        cx.notify();
    }

    pub fn colors(&self) -> &EditorColors {
        &self.colors
    }

    pub fn set_colors(&mut self, colors: EditorColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    /// All selections; the first is the primary one.
    pub fn selections(&self) -> Vec<Selection> {
        self.cursors.selections()
    }

    pub fn set_selections(&mut self, selections: &[Selection], cx: &mut Context<Self>) {
        if selections.is_empty() {
            return;
        }
        self.cursors.set_selections(selections);
        self.cursors.validate(self.snapshot.rope());
        self.cursors_changed(true, cx);
    }

    pub fn scroll_position(&self) -> Point<Pixels> {
        point(self.scroll_left, self.scroll_top)
    }

    pub fn set_scroll_position(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.scroll_left = position.x.max(px(0.));
        self.scroll_top = position.y.max(px(0.));
        cx.emit(CodeEditorEvent::ScrollChanged);
        cx.notify();
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
                self.snapshot = latest;
                self.cursors.validate(self.snapshot.rope());
                if *kind == EditKind::SetValue {
                    self.marked_range = None;
                }
                if !own {
                    self.prev_edit_kind = None;
                }
                if !own || matches!(kind, EditKind::Undo | EditKind::Redo) {
                    self.cursors_changed(own, cx);
                }
                cx.notify();
            }
            TextModelEvent::OptionsChanged | TextModelEvent::LanguageChanged => cx.notify(),
        }
    }

    /// Applies an edit computed by the cursor core: converts it to byte
    /// ranges, pushes it to the model with undo grouping and cursor metadata,
    /// then moves the cursors to the operation's resulting selections.
    pub(crate) fn apply(&mut self, operation: EditOperation, cx: &mut Context<Self>) {
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
        self.snapshot = self.model.read(cx).snapshot();
        self.cursors.set_selections(&operation.selections_after);
        self.cursors.validate(self.snapshot.rope());
        self.prev_edit_kind = Some(operation.kind);
        self.cursors_changed(true, cx);
    }

    fn cursors_changed(&mut self, autoscroll: bool, cx: &mut Context<Self>) {
        if autoscroll {
            self.autoscroll_requested = true;
        }
        self.restart_blink(cx);
        cx.emit(CodeEditorEvent::SelectionsChanged);
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
        let config = self.cursor_config(cx);
        let page = self.page_lines();
        let before = self.cursors.selections();
        cursor::move_cursors(
            &config,
            self.snapshot.rope(),
            &mut self.cursors,
            command,
            select,
            page,
        );
        self.prev_edit_kind = None;
        if self.cursors.selections() != before {
            self.model.update(cx, |model, _| model.push_stack_element());
        }
        self.cursors_changed(true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        cursor::select_all(self.snapshot.rope(), &mut self.cursors);
        self.cursors_changed(false, cx);
    }

    fn expand_line_selection(
        &mut self,
        _: &ExpandLineSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cursor::expand_line_selection(self.snapshot.rope(), &mut self.cursors);
        self.cursors_changed(true, cx);
    }

    fn add_cursor_above(&mut self, _: &AddCursorAbove, _: &mut Window, cx: &mut Context<Self>) {
        let config = self.cursor_config(cx);
        cursor::add_cursor_above(&config, self.snapshot.rope(), &mut self.cursors);
        self.cursors_changed(true, cx);
    }

    fn add_cursor_below(&mut self, _: &AddCursorBelow, _: &mut Window, cx: &mut Context<Self>) {
        let config = self.cursor_config(cx);
        cursor::add_cursor_below(&config, self.snapshot.rope(), &mut self.cursors);
        self.cursors_changed(true, cx);
    }

    fn remove_secondary_cursors(
        &mut self,
        _: &RemoveSecondaryCursors,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.cursors_changed(true, cx);
    }

    // ----- editing ------------------------------------------------------------

    /// Types `text` at every cursor, as if from the keyboard.
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let config = self.cursor_config(cx);
        let operation = cursor::type_text(
            &config,
            self.snapshot.rope(),
            &self.cursors,
            text,
            self.prev_edit_kind,
        );
        self.apply(operation, cx);
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

    fn delete_left(&mut self, _: &DeleteLeft, _: &mut Window, cx: &mut Context<Self>) {
        let prev = self.prev_edit_kind;
        self.run_edit(cx, |c, m, s| cursor::delete_left(c, m, s, prev));
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
        self.run_edit(cx, cursor::tab);
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::outdent);
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::line_break);
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

    fn delete_lines(&mut self, _: &DeleteLines, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::delete_lines);
    }

    fn insert_line_after(&mut self, _: &InsertLineAfter, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::insert_line_below);
    }

    fn insert_line_before(&mut self, _: &InsertLineBefore, _: &mut Window, cx: &mut Context<Self>) {
        self.run_edit(cx, cursor::insert_line_above);
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let source = Some(cx.entity_id());
        self.prev_edit_kind = None;
        self.model.update(cx, |model, cx| model.undo(source, cx));
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
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
        let Some(text) = item.text() else {
            return;
        };
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
    fn position_for_point(
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
        let max_row = self.snapshot.max_point_utf16().row;
        if y < px(0.) {
            return PointUtf16::zero();
        }
        let row = (y / line_height).floor() as u32;
        if row > max_row {
            return self.snapshot.max_point_utf16();
        }
        let x = position.x - layout.text_origin_x();
        let column = match layout.line(row) {
            Some(line) => line.column_for_x(x),
            None => {
                let metrics: &TextMetrics = &layout.metrics;
                let tab_size = self.cursor_config(cx).tab_size;
                layout_row(
                    self.snapshot.rope(),
                    row,
                    tab_size,
                    metrics,
                    self.colors.foreground,
                    None,
                    window,
                )
                .column_for_x(x)
            }
        };
        PointUtf16::new(row, column)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(layout) = self.layout.clone() else {
            return;
        };
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

        let position = self.position_for_point(event.position, window, cx);
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
            MouseSelectMode::Character => Selection::collapsed(position),
            MouseSelectMode::Word => cursor::word_at(&config, self.snapshot.rope(), position)
                .map(|range| Selection {
                    anchor: range.start,
                    head: range.end,
                })
                .unwrap_or_else(|| Selection::collapsed(position)),
            MouseSelectMode::Line => self.line_selection(position.row),
        };

        let add_cursor = event.modifiers.alt;
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
        self.cursors_changed(false, cx);
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
        self.cursors_changed(false, cx);
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
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
        self.scroll_top = (self.scroll_top - delta.y).max(px(0.));
        self.scroll_left = (self.scroll_left - delta.x).max(px(0.));
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
        let explicit = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked_range.take());
        match explicit {
            Some(range) => {
                self.replace_range(range, text, cx);
                self.prev_edit_kind = None;
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
        let line = layout.line(start.row)?;
        let x = layout.text_origin_x() + line.x_for_column(start.column);
        let top = layout.row_top(start.row);
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
        let root = div()
            .id("code-editor")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .cursor(MouseCursor::IBeam);
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
        );
        root.on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::expand_line_selection))
            .on_action(cx.listener(Self::add_cursor_above))
            .on_action(cx.listener(Self::add_cursor_below))
            .on_action(cx.listener(Self::remove_secondary_cursors))
            .on_action(cx.listener(Self::delete_left))
            .on_action(cx.listener(Self::delete_right))
            .on_action(cx.listener(Self::delete_word_left))
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
            .on_action(cx.listener(Self::insert_line_after))
            .on_action(cx.listener(Self::insert_line_before))
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
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(EditorElement::new(cx.entity()))
    }
}
