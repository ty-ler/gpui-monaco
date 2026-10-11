//! Monaco's commands that take arguments or have no other home: the core
//! commands (`cursorMove`, `editorScroll`, `revealLine`, `setSelection`,
//! `createCursor`, `columnSelect`, the mouse commands), the handler
//! commands (`type`, `paste`, `cut`, `compositionType`, …), the
//! contributions' commands without a binding of their own
//! (`acceptSelectedSuggestion`, `leaveSnippet`, `leaveEditorMessage`, …),
//! and Monaco's `ICodeEditor.executeCommand`/`executeCommands` with an
//! [`EditorCommand`].

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    ClipboardItem, Context, InteractiveElement, KeyContext, SharedString, Stateful, Window, px,
};
use text::PointUtf16;

use crate::actions::*;
use crate::code_action::{CodeActionAutoApply, CodeActionFilter};
use crate::context_keys::{ContextKeyValue, FocusKeys};
use crate::cursor::{self, CursorModel, Selection, SingleCursorState};
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::events::CursorChangeReason;
use crate::providers::CodeActionKind;
use crate::registry::Disposable;
use crate::view::reveal::RevealType;

/// A command from [`CodeEditor::add_command`].
pub(crate) type CommandHandler = Rc<dyn Fn(&mut CodeEditor, &mut Window, &mut Context<CodeEditor>)>;

/// An editor's context keys, commands and keybindings.
pub(crate) struct EditorCommandsState {
    /// Names the editor in its key context, for its own keybindings.
    pub(crate) id: u64,
    /// Keys from [`CodeEditor::create_context_key`].
    pub(crate) host_keys: HashMap<SharedString, ContextKeyValue>,
    /// What the context keys needed a window for, last frame.
    pub(crate) focus: FocusKeys,
    /// Between `compositionStart` and `compositionEnd`.
    pub(crate) composing: bool,
    /// Commands from [`CodeEditor::add_command`].
    pub(crate) handlers: HashMap<SharedString, CommandHandler>,
    /// The keybindings of each action from [`CodeEditor::add_action`].
    pub(crate) action_bindings: HashMap<SharedString, Disposable>,
    /// Keybindings to remove on the next frame.
    pub(crate) pending_dispose: Vec<Disposable>,
    /// Focus the hover once it shows (`editor.action.showHover` with
    /// `autoFocusImmediately`).
    pub(crate) focus_hover_when_shown: bool,
}

impl Default for EditorCommandsState {
    fn default() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            host_keys: HashMap::new(),
            focus: FocusKeys::default(),
            composing: false,
            handlers: HashMap::new(),
            action_bindings: HashMap::new(),
            pending_dispose: Vec::new(),
            focus_hover_when_shown: false,
        }
    }
}

/// A new id for [`CodeEditor::add_command`] (Monaco's `DYNAMIC_` ids).
pub(crate) fn next_dynamic_command_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("DYNAMIC_{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Builds the edits a command makes, for [`EditorCommand`] (Monaco
/// `IEditOperationBuilder`).
pub struct EditOperationBuilder<'a, 'b> {
    builder: &'a mut cursor::EditBuilder<'b>,
}

/// A selection [`EditOperationBuilder::track_selection`] follows through
/// the edits.
#[derive(Clone, Copy, Debug)]
pub struct TrackedSelection(cursor::TrackedId);

impl EditOperationBuilder<'_, '_> {
    /// Replaces `range` with `text` (Monaco `addEditOperation`).
    pub fn add_edit_operation(&mut self, range: Range<PointUtf16>, text: &str) {
        self.builder.add_edit(range, text);
    }

    /// Replaces `range` with `text`, its end moving cursors and decorations
    /// at it along (Monaco `addTrackedEditOperation` with
    /// `forceMoveMarkers`).
    pub fn add_edit_operation_forcing(&mut self, range: Range<PointUtf16>, text: &str) {
        self.builder.add_edit_with(range, text, true);
    }

    /// Follows `selection` through the edits (Monaco `trackSelection`);
    /// `track_previous_on_empty` says which side an empty selection sticks
    /// to (by default the text before it at a line end, else after).
    pub fn track_selection(
        &mut self,
        selection: Selection,
        track_previous_on_empty: Option<bool>,
    ) -> TrackedSelection {
        TrackedSelection(
            self.builder
                .track_selection(selection, track_previous_on_empty),
        )
    }
}

/// What a command's cursor state is computed from (Monaco
/// `ICursorStateComputerData`).
pub struct CursorStateComputerData<'a, 'b> {
    helper: &'a cursor::CursorStateHelper<'b>,
}

impl CursorStateComputerData<'_, '_> {
    /// Where each of the command's edits ended up, in the order they were
    /// added (Monaco `getInverseEditOperations`).
    pub fn inverse_edit_ranges(&self) -> &[Range<PointUtf16>] {
        self.helper.inverse_edit_ranges()
    }

    /// Where a tracked selection ended up (Monaco `getTrackedSelection`).
    pub fn tracked_selection(&self, id: TrackedSelection) -> Selection {
        self.helper.tracked_selection(id.0)
    }
}

/// An edit command (Monaco `ICommand`) for
/// [`CodeEditor::execute_command`]: it adds its edits for the text as it
/// is, then says where its cursor goes once they're applied.
pub trait EditorCommand {
    /// Adds the command's edits (Monaco `getEditOperations`).
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditOperationBuilder);

    /// The cursor's selection after the edits (Monaco
    /// `computeCursorState`).
    fn compute_cursor_state(&self, data: &CursorStateComputerData) -> Selection;
}

/// An [`EditorCommand`] as the cursor core runs commands.
struct HostCommand(Box<dyn EditorCommand>);

impl cursor::Command for HostCommand {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut cursor::EditBuilder) {
        self.0
            .edit_operations(model, &mut EditOperationBuilder { builder });
    }

    fn compute_cursor_state(&self, helper: &cursor::CursorStateHelper) -> Selection {
        self.0
            .compute_cursor_state(&CursorStateComputerData { helper })
    }
}

/// Declares listeners for actions whose handlers take the action.
macro_rules! listen {
    ($root:expr, $cx:expr, $($action:ty => $handler:expr),* $(,)?) => {
        $root$(.on_action($cx.listener($handler as fn(&mut CodeEditor, &$action, &mut Window, &mut Context<CodeEditor>))))*
    };
}

/// The editor's listeners for this module's actions.
pub(crate) fn register_handlers(
    root: Stateful<gpui::Div>,
    cx: &mut Context<CodeEditor>,
) -> Stateful<gpui::Div> {
    listen!(root, cx,
        SecondaryEscape => |this, _, window, cx| this.escape(true, window, cx),
        CursorMove => |this, args, _, cx| this.cursor_move(args, cx),
        EditorScroll => |this, args, _, cx| this.editor_scroll(args, cx),
        RevealLine => |this, args, _, cx| this.reveal_line_command(args, cx),
        SetSelection => |this, args, _, cx| {
            if let Some(selection) = args.selection {
                this.set_cursor_states(vec![SingleCursorState::from_selection(selection.to_selection())], true, cx);
            }
        },
        CreateCursor => |this, args, _, cx| this.create_cursor(args, cx),
        ColumnSelect => |this, args, _, cx| {
            let (Some(position), Some(column)) = (args.position, args.mouse_column) else {
                return;
            };
            let config = this.cursor_config(cx);
            let rope = this.snapshot.rope().clone();
            this.model().update(cx, |model, _| model.push_stack_element());
            cursor::column_select_to(&config, &rope, &mut this.cursors, position.to_point(), column.saturating_sub(1), args.do_column_select);
            this.next_selection_change.reason = CursorChangeReason::Explicit;
            this.cursors_changed(true, cx);
        },
        MoveTo => |this, args, _, cx| this.mouse_command(MouseCommand::MoveTo(false), args.position, args.reveal_type, cx),
        MoveToSelect => |this, args, _, cx| this.mouse_command(MouseCommand::MoveTo(true), args.position, args.reveal_type, cx),
        WordSelect => |this, args, _, cx| this.mouse_command(MouseCommand::Word(false), args.position, args.reveal_type, cx),
        WordSelectDrag => |this, args, _, cx| this.mouse_command(MouseCommand::Word(true), args.position, args.reveal_type, cx),
        LineSelect => |this, args, _, cx| this.mouse_command(MouseCommand::Line(false), args.position, args.reveal_type, cx),
        LineSelectDrag => |this, args, _, cx| this.mouse_command(MouseCommand::Line(true), args.position, args.reveal_type, cx),
        LastCursorMoveToSelect => |this, args, _, cx| this.mouse_command(MouseCommand::LastMoveTo, args.position, Some(2), cx),
        LastCursorWordSelect => |this, args, _, cx| this.mouse_command(MouseCommand::LastWord, args.position, Some(2), cx),
        LastCursorLineSelect => |this, args, _, cx| this.mouse_command(MouseCommand::LastLine(false), args.position, Some(2), cx),
        LastCursorLineSelectDrag => |this, args, _, cx| this.mouse_command(MouseCommand::LastLine(true), args.position, Some(2), cx),
        Type => |this, args, _, cx| this.type_text(&args.text, cx),
        ReplacePreviousChar => |this, args, _, cx| this.composition_type(&args.text, args.replace_char_cnt, 0, 0, cx),
        CompositionType => |this, args, _, cx| this.composition_type(&args.text, args.replace_prev_char_cnt, args.replace_next_char_cnt, args.position_delta, cx),
        CompositionStart => |this, _, _, cx| {
            if !this.commands.composing {
                this.commands.composing = true;
                cx.emit(CodeEditorEvent::CompositionStarted);
                cx.notify();
            }
        },
        CompositionEnd => |this, _, _, cx| {
            if this.commands.composing {
                this.commands.composing = false;
                cx.emit(CodeEditorEvent::CompositionEnded);
                cx.notify();
            }
        },
        PasteText => |this, args, window, cx| this.paste_text(args, window, cx),
        CutText => |this, _, _, cx| {
            let config = this.cursor_config(cx);
            let operation = cursor::cut(&config, this.snapshot.rope(), &this.cursors);
            this.apply(operation, cx);
        },
        FindReferences => |this, args, _, cx| {
            let uri = this.model().read(cx).uri().cloned();
            if args.uri.as_deref().is_some_and(|wanted| uri.as_deref() != Some(wanted)) {
                return;
            }
            if let Some(position) = args.position {
                this.set_selections(&[Selection::collapsed(position.to_point())], cx);
            }
            this.peek(crate::options::LocationKind::References, cx);
        },
        Fold => |this, args, _, cx| this.fold_with_args(args.levels, args.direction.as_deref(), args.selection_lines.clone(), true, cx),
        Unfold => |this, args, _, cx| this.fold_with_args(args.levels, args.direction.as_deref(), args.selection_lines.clone(), false, cx),
        Refactor => |this, args, _, cx| this.code_action_kind_command(CodeActionKind::REFACTOR, args.kind.clone(), args.apply.clone(), args.preferred, cx),
        SourceAction => |this, args, _, cx| this.code_action_kind_command(CodeActionKind::SOURCE, args.kind.clone(), args.apply.clone(), args.preferred, cx),
        PasteAs => |this, args, window, cx| {
            let only = args.kind.clone().or_else(|| args.preferences.as_ref()?.first().cloned());
            this.paste_as(only.map(Into::into), window, cx)
        },
        ShowHover => |this, args, window, cx| this.show_hover_command(args, window, cx),
        TriggerInlineSuggest => |this, args, _, cx| {
            if args.explicit == Some(false) {
                this.trigger_inline_completions(cx);
            } else {
                this.trigger_inline_suggest(cx);
            }
        },
        AcceptSelectedSuggestion => |this, _, _, cx| {
            this.accept_suggestion(cx);
        },
        HideSuggestWidget => |this, _, _, cx| this.hide_suggest(cx),
        InsertBestCompletion => |this, _, _, cx| {
            this.insert_best_completion(cx);
        },
        InsertNextSuggestion => |this, _, _, cx| this.insert_other_suggestion(true, cx),
        InsertPrevSuggestion => |this, _, _, cx| this.insert_other_suggestion(false, cx),
        JumpToNextSnippetPlaceholder => |this, _, _, cx| this.next_snippet_placeholder(cx),
        JumpToPrevSnippetPlaceholder => |this, _, _, cx| this.prev_snippet_placeholder(cx),
        LeaveSnippet => |this, _, _, cx| this.cancel_snippet(true, cx),
        AcceptSnippet => |this, _, _, cx| this.finish_snippet(cx),
        CancelLinkedEditingInput => |this, _, _, cx| this.clear_linked_editing(cx),
        LeaveEditorMessage => |this, _, _, cx| this.close_message(cx),
        ShowUnicodeHighlightExcludeOptions => |this, _, window, cx| {
            let offset = this.snapshot.point_utf16_to_offset(this.cursors.primary().position);
            let hover = this
                .unicode_hovers(offset, cx)
                .into_iter()
                .next()
                .or_else(|| this.unicode_hovers(offset.saturating_sub(1), cx).into_iter().next());
            if let Some(hover) = hover {
                this.show_unicode_exclude_options(hover, window, cx);
            }
        },
        CursorWordAccessibilityLeft => |this, _, _, cx| this.accessibility_word_move(true, false, cx),
        CursorWordAccessibilityLeftSelect => |this, _, _, cx| this.accessibility_word_move(true, true, cx),
        CursorWordAccessibilityRight => |this, _, _, cx| this.accessibility_word_move(false, false, cx),
        CursorWordAccessibilityRightSelect => |this, _, _, cx| this.accessibility_word_move(false, true, cx),
        CloseFindWidget => |this, _, window, cx| this.close_find(window, cx),
        CloseMarkersNavigation => |this, _, _, cx| {
            this.close_marker_zone(cx);
        },
        GoToNextSymbolFromResult => |this, _, _, cx| {
            this.go_to_next_symbol_from_result(cx);
        },
        CancelSymbolNavigation => |this, _, _, cx| {
            this.cancel_symbol_navigation(cx);
        },
        FormatSelectionOrDocument => |this, _, _, cx| {
            // Monaco runs the format commands, with their preconditions.
            if this.cursors.primary().selection().is_empty() {
                if this.action_enabled("editor.action.formatDocument", cx) {
                    this.format_document(cx);
                }
            } else if this.action_enabled("editor.action.formatSelection", cx) {
                this.format_selection(cx);
            }
        },
        ScrollLeft => |this, _, _, cx| this.scroll_columns(-2., cx),
        ScrollRight => |this, _, _, cx| this.scroll_columns(2., cx),
        // The rename box's commands, run by id (its input's keys reach the
        // box first; the input itself passes them on).
        AcceptRenameInput => |this, _, _, cx| {
            if !this.is_rename_visible() {
                return cx.propagate();
            }
            this.accept_rename_input(false, cx)
        },
        AcceptRenameInputWithPreview => |this, _, _, cx| {
            if !this.is_rename_visible() {
                return cx.propagate();
            }
            this.accept_rename_input(true, cx)
        },
        CancelRenameInput => |this, _, _, cx| {
            if !this.cancel_rename(true, cx) {
                cx.propagate();
            }
        },
        // `closeReferenceSearch` by id (the peek view's own keys reach it
        // first).
        ClosePeek => |this, _, _, cx| {
            // Monaco's key is off with `stablePeek`.
            if this.options.stable_peek || !this.close_peek(cx) {
                cx.propagate();
            }
        },
        FocusNextRenameSuggestion => |this, _, _, cx| {
            if !this.is_rename_visible() {
                return cx.propagate();
            }
            this.focus_next_rename_suggestion(cx)
        },
        FocusPreviousRenameSuggestion => |this, _, _, cx| {
            if !this.is_rename_visible() {
                return cx.propagate();
            }
            this.focus_previous_rename_suggestion(cx)
        },
    )
}

/// Monaco's mouse commands.
#[derive(Clone, Copy)]
enum MouseCommand {
    /// `_moveTo`, `_moveToSelect`.
    MoveTo(bool),
    /// `_wordSelect`, `_wordSelectDrag`.
    Word(bool),
    /// `_lineSelect`, `_lineSelectDrag`.
    Line(bool),
    /// `_lastCursorMoveToSelect`.
    LastMoveTo,
    /// `lastCursorWordSelect`.
    LastWord,
    /// `lastCursorLineSelect`, `lastCursorLineSelectDrag`.
    LastLine(bool),
}

impl CodeEditor {
    /// What each frame does for the context keys and commands: notes the
    /// focus, adds the keys to `context`, drops removed keybindings and
    /// focuses a hover that was asked to take focus.
    pub(crate) fn render_commands(
        &mut self,
        context: &mut KeyContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.commands.focus;
        self.note_focus_keys(window, cx);
        // Monaco `WordHighlighter`: `onDidFocusEditorText` runs it,
        // `onDidBlurEditorWidget` stops it. (Focus events can be missed
        // before the window is active, so the frame compares.)
        let after = self.commands.focus;
        if after.text && !before.text {
            self.word_highlights_focus_changed(true, cx);
        } else if before.widget && !after.widget {
            self.word_highlights_focus_changed(false, cx);
        }
        self.add_context_keys(context, cx);
        for disposable in std::mem::take(&mut self.commands.pending_dispose) {
            cx.defer(move |cx| disposable.dispose(cx));
        }
        if self.commands.focus_hover_when_shown && self.focus_hover(window, cx) {
            self.commands.focus_hover_when_shown = false;
        }
    }

    /// Sets the cursors (Monaco `setCursorStates` with `Explicit`), with an
    /// undo stop before as Monaco's core commands push.
    fn set_cursor_states(
        &mut self,
        states: Vec<SingleCursorState>,
        reveal: bool,
        cx: &mut Context<Self>,
    ) {
        if states.is_empty() {
            return;
        }
        self.model()
            .update(cx, |model, _| model.push_stack_element());
        let rope = self.snapshot.rope().clone();
        self.cursors.validated_states(&rope, states);
        self.next_selection_change.reason = CursorChangeReason::Explicit;
        self.cursors_changed(reveal, cx);
    }

    /// Monaco `cursorMove` (`CursorMoveCommands.simpleMove` and
    /// `viewportMove`).
    fn cursor_move(&mut self, args: &CursorMove, cx: &mut Context<Self>) {
        let value = args.value.unwrap_or(1).max(1);
        let select = args.select;
        let by = args.by.as_deref();
        let config = self.cursor_config(cx);
        let rope = self.snapshot.rope().clone();
        let states: Vec<SingleCursorState> = self.cursors.all().to_vec();
        let model_move = |command: cursor::MoveCommand, this: &mut Self| {
            let mut cursors = this.cursors.clone();
            for _ in 0..value {
                cursor::move_cursors(&config, &rope, &mut cursors, command, select, 1);
            }
            cursors.all().to_vec()
        };
        let moved: Vec<SingleCursorState> = match args.to.as_str() {
            "left" | "right" => {
                let left = args.to == "left";
                states
                    .iter()
                    .map(|state| {
                        let count = if by == Some("halfLine") {
                            (rope.line_len_utf16(state.position.row) as f32 / 2.).round() as u32
                        } else {
                            value
                        };
                        if left {
                            cursor::move_left(&config, &rope, state, select, count.max(1))
                        } else {
                            cursor::move_right(&config, &rope, state, select, count.max(1))
                        }
                    })
                    .collect()
            }
            "up" | "down" => {
                let up = args.to == "up";
                let command = if up {
                    cursor::MoveCommand::Up
                } else {
                    cursor::MoveCommand::Down
                };
                match by {
                    // Monaco moves by view lines here.
                    Some("wrappedLine") => {
                        for _ in 0..value {
                            self.move_cursors(command, select, cx);
                        }
                        return;
                    }
                    Some("foldedLine") => states
                        .iter()
                        .map(|state| {
                            let from = if state.has_selection() && !select {
                                if up {
                                    state.selection().start().row
                                } else {
                                    state.selection().end().row
                                }
                            } else {
                                state.position.row
                            };
                            let target = self.folded_line_target(from, value, up);
                            let mut cursors = cursor::CursorCollection::new(state.clone());
                            let steps = target.abs_diff(from);
                            if steps == 0 {
                                return state.clone();
                            }
                            for _ in 0..steps {
                                cursor::move_cursors(
                                    &config,
                                    &rope,
                                    &mut cursors,
                                    command,
                                    select,
                                    1,
                                );
                            }
                            cursors.primary().clone()
                        })
                        .collect(),
                    _ => model_move(command, self),
                }
            }
            "prevBlankLine" => model_move(cursor::MoveCommand::PrevBlankLine, self),
            "nextBlankLine" => model_move(cursor::MoveCommand::NextBlankLine, self),
            "wrappedLineStart"
            | "wrappedLineEnd"
            | "wrappedLineColumnCenter"
            | "wrappedLineFirstNonWhitespaceCharacter"
            | "wrappedLineLastNonWhitespaceCharacter" => states
                .iter()
                .map(|state| {
                    let column = self.view_line_column(state.position, &args.to);
                    state.move_to(select, column, 0)
                })
                .collect(),
            "viewPortTop" | "viewPortBottom" | "viewPortCenter" => {
                let Some((first, last)) = self.visible_rows(cx) else {
                    return;
                };
                let row = match args.to.as_str() {
                    "viewPortTop" => (first + value - 1).min(last),
                    "viewPortBottom" => last.saturating_sub(value - 1).max(first),
                    _ => (first + last).div_ceil(2),
                };
                let column = cursor::first_non_whitespace_column(&rope, row).unwrap_or(0);
                vec![
                    self.cursors
                        .primary()
                        .move_to(select, PointUtf16::new(row, column), 0),
                ]
            }
            "viewPortIfOutside" => {
                let Some((first, last)) = self.visible_rows(cx) else {
                    return;
                };
                let last = last.saturating_sub(1).max(first);
                states
                    .iter()
                    .map(|state| {
                        let row = state.position.row;
                        if (first..=last).contains(&row) {
                            return state.clone();
                        }
                        let target = row.clamp(first, last);
                        let mut cursors = cursor::CursorCollection::new(state.clone());
                        let command = if target < row {
                            cursor::MoveCommand::Up
                        } else {
                            cursor::MoveCommand::Down
                        };
                        for _ in 0..target.abs_diff(row) {
                            cursor::move_cursors(&config, &rope, &mut cursors, command, select, 1);
                        }
                        cursors.primary().clone()
                    })
                    .collect()
            }
            // Monaco ignores other values.
            _ => return,
        };
        self.set_cursor_states(moved, true, cx);
    }

    /// The model row `count` folded lines up or down from `row`, a folded
    /// region counting as one (Monaco `_targetFoldedDown`/`Up`).
    fn folded_line_target(&self, row: u32, count: u32, up: bool) -> u32 {
        let lines = self.view_lines.clone();
        let last = self.snapshot.rope().max_point().row;
        let mut row = row;
        for _ in 0..count {
            if up {
                if row == 0 {
                    break;
                }
                row -= 1;
                while row > 0 && lines.is_row_hidden(row) {
                    row -= 1;
                }
            } else {
                if row >= last {
                    break;
                }
                row += 1;
                while row < last && lines.is_row_hidden(row) {
                    row += 1;
                }
            }
        }
        row
    }

    /// The model position `to` names on the view line (wrapped segment)
    /// `position` is on.
    fn view_line_column(&self, position: PointUtf16, to: &str) -> PointUtf16 {
        let lines = self.view_lines.clone();
        let (line, _) = lines.to_view(position, false);
        let content = lines.line_content(line);
        let max = lines.line_max_column(line);
        let units: Vec<u16> = content.encode_utf16().collect();
        let is_space = |unit: &u16| *unit == b' ' as u16 || *unit == b'\t' as u16;
        let column = match to {
            "wrappedLineStart" => 0,
            "wrappedLineEnd" => max,
            "wrappedLineColumnCenter" => (max as f32 / 2.).round() as u32,
            "wrappedLineFirstNonWhitespaceCharacter" => {
                units.iter().position(|u| !is_space(u)).unwrap_or(0) as u32
            }
            _ => units
                .iter()
                .rposition(|u| !is_space(u))
                .map_or(0, |i| i as u32 + 1),
        };
        lines.to_model(line, column.min(max))
    }

    /// The first and last model rows that show.
    fn visible_rows(&self, cx: &gpui::App) -> Option<(u32, u32)> {
        let ranges = self.visible_ranges(cx);
        Some((ranges.first()?.start.row, ranges.last()?.end.row))
    }

    /// Monaco `editorScroll`.
    fn editor_scroll(&mut self, args: &EditorScroll, cx: &mut Context<Self>) {
        let value = args.value.unwrap_or(1.).floor().max(1.) as f32;
        let page = self.layout.as_ref().map_or(20, |l| l.page_rows()) as f32;
        match (args.to.as_str(), args.by.as_deref()) {
            ("left" | "right", Some("column")) => {
                let sign = if args.to == "left" { -1. } else { 1. };
                self.scroll_columns(sign * value, cx);
                return;
            }
            ("up" | "down", by) => {
                let up = args.to == "up";
                match by {
                    Some("line") => {
                        // By model lines, from the first visible one.
                        let Some((first, _)) = self.visible_rows(cx) else {
                            return;
                        };
                        let last = self.snapshot.rope().max_point().row;
                        let row = if up {
                            first.saturating_sub(value as u32)
                        } else {
                            (first + value as u32).min(last)
                        };
                        if let Some(top) = self.top_for_line_number(row, false) {
                            self.set_scroll_top(top, crate::ScrollType::Smooth, cx);
                        }
                    }
                    Some("editor") => self.scroll_to_edge(!up, cx),
                    _ => {
                        let lines = match by {
                            Some("page") => page * value,
                            Some("halfPage") => (page / 2.).round() * value,
                            Some("wrappedLine") | None => value,
                            // Monaco ignores other units.
                            Some(_) => return,
                        };
                        self.scroll_lines(if up { -lines } else { lines }, cx);
                    }
                }
            }
            _ => return,
        }
        if args.reveal_cursor {
            // Monaco keeps the primary cursor in the new viewport.
            self.cursor_move(
                &CursorMove {
                    to: "viewPortIfOutside".into(),
                    select: args.select,
                    ..Default::default()
                },
                cx,
            );
        }
    }

    /// Scrolls by `columns` typical character widths.
    fn scroll_columns(&mut self, columns: f32, cx: &mut Context<Self>) {
        let width = self.layout.as_ref().map_or(px(7.), |l| l.metrics.em_width);
        self.scroll_by(px(0.), width * columns, cx);
    }

    /// Monaco `revealLine`: the 0-based line, where `at` says.
    fn reveal_line_command(&mut self, args: &RevealLine, cx: &mut Context<Self>) {
        let line = match &args.line_number {
            Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.) as i64,
            Some(serde_json::Value::String(s)) => s.trim().parse::<i64>().unwrap_or(0),
            _ => 0,
        };
        let last = self.snapshot.rope().max_point().row;
        let row = line.clamp(0, last as i64) as u32;
        let reveal = match args.at.as_deref() {
            Some("top") => RevealType::Top,
            Some("center") => RevealType::Center,
            Some("bottom") => RevealType::Bottom,
            _ => RevealType::Simple,
        };
        let end = self.snapshot.rope().line_len_utf16(row);
        self.reveal_range(
            PointUtf16::new(row, 0)..PointUtf16::new(row, end),
            reveal,
            cx,
        );
    }

    /// Monaco `createCursor`: adds a cursor at `position`, or removes the
    /// one there.
    fn create_cursor(&mut self, args: &CreateCursor, cx: &mut Context<Self>) {
        let Some(position) = args.position else {
            return;
        };
        let config = self.cursor_config(cx);
        let rope = self.snapshot.rope().clone();
        let p = position.to_point();
        let mut temp = cursor::CursorCollection::new(self.cursors.primary().clone());
        if args.whole_line {
            cursor::select_line(&rope, &mut temp, p, false);
        } else {
            cursor::move_to(&config, &rope, &mut temp, p, false);
        }
        let new = temp.primary().clone();
        let mut states = self.cursors.all().to_vec();
        if states.len() > 1
            && let Some(index) = states.iter().position(|state| {
                let selection = state.selection();
                selection.start() <= new.position && new.position <= selection.end()
            })
        {
            states.remove(index);
        } else {
            states.push(new);
        }
        self.set_cursor_states(states, false, cx);
    }

    /// Monaco's mouse commands: `_moveTo`, `_wordSelect`,
    /// `lastCursorLineSelect` and the rest.
    fn mouse_command(
        &mut self,
        command: MouseCommand,
        position: Option<CommandPosition>,
        reveal_type: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = position else {
            return;
        };
        let p = position.to_point();
        let config = self.cursor_config(cx);
        let rope = self.snapshot.rope().clone();
        let single = |state: SingleCursorState, f: &dyn Fn(&mut cursor::CursorCollection)| {
            let mut temp = cursor::CursorCollection::new(state);
            f(&mut temp);
            temp.primary().clone()
        };
        let states = match command {
            MouseCommand::MoveTo(select) => {
                let mut cursors = self.cursors.clone();
                cursor::move_to(&config, &rope, &mut cursors, p, select);
                cursors.all().to_vec()
            }
            MouseCommand::Word(select) => {
                let mut cursors = self.cursors.clone();
                cursor::select_word(&config, &rope, &mut cursors, p, select);
                cursors.all().to_vec()
            }
            MouseCommand::Line(select) => {
                let mut cursors = self.cursors.clone();
                cursor::select_line(&rope, &mut cursors, p, select);
                cursors.all().to_vec()
            }
            MouseCommand::LastMoveTo | MouseCommand::LastWord | MouseCommand::LastLine(_) => {
                let mut states = self.cursors.all().to_vec();
                let index = self.cursors.last_added_cursor_index().min(states.len() - 1);
                let last = states[index].clone();
                states[index] = match command {
                    MouseCommand::LastMoveTo => last.move_to(true, p, 0),
                    MouseCommand::LastWord => {
                        let select = last.has_selection();
                        single(last, &|c| cursor::select_word(&config, &rope, c, p, select))
                    }
                    MouseCommand::LastLine(select) => {
                        single(last, &|c| cursor::select_line(&rope, c, p, select))
                    }
                    _ => unreachable!(),
                };
                states
            }
        };
        // Monaco `NavigationCommandRevealType.None` is 2.
        self.set_cursor_states(states, reveal_type != Some(2), cx);
    }

    /// Monaco's `compositionType` handler.
    fn composition_type(
        &mut self,
        text: &str,
        replace_prev_char_cnt: u32,
        replace_next_char_cnt: u32,
        position_delta: i32,
        cx: &mut Context<Self>,
    ) {
        let operation = cursor::composition_type(
            self.snapshot.rope(),
            &self.cursors,
            text,
            replace_prev_char_cnt,
            replace_next_char_cnt,
            position_delta,
            self.prev_edit_kind,
        );
        self.apply(operation, cx);
    }

    /// Monaco's `paste` handler: pastes `text` as if copied with the
    /// arguments' metadata, without paste providers.
    fn paste_text(&mut self, args: &PasteText, window: &mut Window, cx: &mut Context<Self>) {
        if args.text.is_empty() && args.multicursor_text.is_none() {
            window.dispatch_action(Box::new(Paste), cx);
            return;
        }
        let metadata = serde_json::json!({
            "is_from_empty_selection": args.paste_on_new_line,
            "multicursor_text": args.multicursor_text,
            "mode": args.mode,
        });
        let item = ClipboardItem::new_string_with_json_metadata(args.text.clone(), metadata);
        self.begin_update(cx);
        self.paste_item_inner(item, cx);
        self.end_update(cx);
    }

    /// Monaco `cursorWordAccessibilityLeft`/`Right` (and `Select`).
    fn accessibility_word_move(&mut self, left: bool, select: bool, cx: &mut Context<Self>) {
        let config = self.cursor_config(cx);
        let rope = self.snapshot.rope().clone();
        let mut cursors = self.cursors.clone();
        cursor::move_word(
            &config,
            &rope,
            &mut cursors,
            left,
            cursor::WordNavigationType::WordAccessibility,
            select,
        );
        self.set_cursor_states(cursors.all().to_vec(), true, cx);
    }

    /// Monaco `editor.action.refactor` and `editor.action.sourceAction`
    /// with their `CodeActionCommandArgs`.
    fn code_action_kind_command(
        &mut self,
        base: &'static str,
        kind: Option<String>,
        apply: Option<String>,
        preferred: bool,
        cx: &mut Context<Self>,
    ) {
        let base_kind = CodeActionKind::new(base);
        let include = match &kind {
            None => base_kind.clone(),
            Some(kind) if base_kind.contains(&CodeActionKind::new(kind.clone())) => {
                CodeActionKind::new(kind.clone())
            }
            // Monaco `HierarchicalKind.None`: matches nothing.
            Some(_) => CodeActionKind::new("@@none@@"),
        };
        let apply = match apply.as_deref().map(str::to_lowercase).as_deref() {
            Some("first") => CodeActionAutoApply::First,
            Some("ifsingle") => CodeActionAutoApply::IfSingle,
            _ => CodeActionAutoApply::Never,
        };
        let noun = if base == CodeActionKind::REFACTOR {
            "refactorings"
        } else {
            "source actions"
        };
        let message = match (&kind, preferred) {
            (Some(kind), true) => format!("No preferred {noun} for '{kind}' available"),
            (Some(kind), false) => format!("No {noun} for '{kind}' available"),
            (None, true) => format!("No preferred {noun} available"),
            (None, false) => format!("No {noun} available"),
        };
        self.trigger_code_actions(
            CodeActionFilter {
                include: Some(include),
                include_source_actions: base == CodeActionKind::SOURCE,
                only_preferred: preferred,
                ..Default::default()
            },
            apply,
            message,
            cx,
        );
    }

    /// Monaco `editor.action.showHover` with its `focus` argument.
    fn show_hover_command(
        &mut self,
        args: &ShowHover,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[derive(PartialEq)]
        enum Focus {
            Never,
            IfVisible,
            Immediately,
        }
        let focus = match &args.focus {
            Some(serde_json::Value::String(s)) if s == "noAutoFocus" => Focus::Never,
            Some(serde_json::Value::String(s)) if s == "autoFocusImmediately" => Focus::Immediately,
            Some(serde_json::Value::Bool(true)) => Focus::Immediately,
            _ => Focus::IfVisible,
        };
        // Monaco: the computed `accessibilitySupport` is `Enabled` (on, or
        // auto with a screen reader).
        let accessibility = self.is_screen_reader_optimized();
        if self.is_hover_visible() {
            if focus != Focus::Never {
                self.show_or_focus_hover(window, cx);
                return;
            }
            self.show_hover_at_cursor(cx);
            self.commands.focus_hover_when_shown = accessibility;
        } else {
            self.show_hover_at_cursor(cx);
            self.commands.focus_hover_when_shown = accessibility || focus == Focus::Immediately;
        }
    }

    /// Runs `command` at the primary selection (Monaco `executeCommand`):
    /// its edits, then its cursor state, as one undo step joined to the
    /// open one. `source` is the change's source (Monaco's `source`).
    /// Read-only editors refuse it.
    pub fn execute_command(
        &mut self,
        source: Option<&str>,
        command: impl EditorCommand + 'static,
        cx: &mut Context<Self>,
    ) {
        self.execute_commands(source, vec![Some(Box::new(command))], cx);
    }

    /// Runs `commands`, one per cursor in order (Monaco
    /// `executeCommands`); `None` leaves a cursor alone, and cursors past
    /// the commands follow the edits. Extra commands run at the last
    /// cursor.
    pub fn execute_commands(
        &mut self,
        source: Option<&str>,
        commands: Vec<Option<Box<dyn EditorCommand>>>,
        cx: &mut Context<Self>,
    ) {
        if commands.is_empty() {
            return;
        }
        let mut selections = self.cursors.selections();
        while selections.len() < commands.len() {
            let last = *selections.last().expect("a cursor");
            selections.push(last);
        }
        let commands: Vec<Option<Box<dyn cursor::Command>>> = commands
            .into_iter()
            .map(|command| {
                command.map(|command| Box::new(HostCommand(command)) as Box<dyn cursor::Command>)
            })
            .collect();
        let operation = cursor::execute_commands(
            self.snapshot.rope(),
            &selections,
            commands,
            cursor::EditOperationType::Other,
            false,
            false,
        );
        // Monaco's sources are free strings; others count as the API's.
        self.next_selection_change.source = match source {
            Some("keyboard") => crate::events::CursorChangeSource::Keyboard,
            Some("mouse") => crate::events::CursorChangeSource::Mouse,
            _ => crate::events::CursorChangeSource::Api,
        };
        self.apply(operation, cx);
        self.next_selection_change = Default::default();
    }
}

/// The editor's subscriptions for this module: its own keybindings go
/// with it.
pub(crate) fn subscriptions(cx: &mut Context<CodeEditor>) -> Vec<gpui::Subscription> {
    vec![cx.on_release(|this: &mut CodeEditor, cx| {
        let bindings: Vec<Disposable> = this
            .commands
            .action_bindings
            .drain()
            .map(|(_, bindings)| bindings)
            .chain(this.commands.pending_dispose.drain(..))
            .collect();
        for bindings in bindings {
            bindings.dispose(cx);
        }
    })]
}
