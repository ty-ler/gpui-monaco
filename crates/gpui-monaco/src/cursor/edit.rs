//! Executing per-cursor edit commands as one batch.
//!
//! Ports Monaco's `CommandExecutor` from
//! `src/vs/editor/common/cursor/cursor.ts`, the inverse-range computation of
//! `PieceTreeTextBuffer._getInverseEditRanges`
//! (`src/vs/editor/common/model/pieceTreeTextBuffer/pieceTreeTextBuffer.ts`),
//! tracked-range adjustment from `nodeAcceptEdit` in
//! `src/vs/editor/common/model/intervalTree.ts`, `EditOperationType` and
//! `EditOperationResult` from `src/vs/editor/common/cursorCommon.ts`, and the
//! commands of `src/vs/editor/common/commands/replaceCommand.ts` (MIT).
//!
//! Each cursor contributes a command that emits edits in pre-edit
//! coordinates and later computes its selection from where its edits ended
//! up ("inverse edit ranges") or from selections it asked to be tracked.

use std::ops::Range;

use super::auto_closed::AutoClosedPair;
use super::{CursorModel, Position, Selection};

/// The kind of an edit, used to decide where undo stops go (Monaco
/// `EditOperationType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOperationType {
    /// Any other edit (paste, cut, word deletion, line operations, ...).
    Other,
    /// Backspace ([`super::delete_left`]).
    DeletingLeft,
    /// Delete ([`super::delete_right`]).
    DeletingRight,
    /// Typing anything other than a single space.
    TypingOther,
    /// Typing a space after something other than a space.
    TypingFirstSpace,
    /// Typing a space right after another typed space.
    TypingConsecutiveSpace,
}

/// The result of an editing function.
#[derive(Clone, Debug, PartialEq)]
pub struct EditOperation {
    /// Non-overlapping edits in PRE-edit coordinates, to be applied
    /// atomically as one batch. Sorted by position.
    pub edits: Vec<(Range<Position>, String)>,
    /// One selection per cursor, in POST-edit coordinates (after all edits
    /// applied), in cursor order. Like Monaco, cursors whose edits conflicted
    /// with an earlier cursor's are dropped, and some operations (cut,
    /// distributed paste) report selections sorted by position. After
    /// applying `edits`, pass these to
    /// [`super::CursorCollection::set_selections`] (which also merges
    /// overlapping cursors) and then [`super::CursorCollection::validate`].
    pub selections_after: Vec<Selection>,
    /// Record this as the previous operation type for the next call.
    pub kind: EditOperationType,
    /// Push an undo stop before applying the edits.
    pub push_stack_element_before: bool,
    /// Push an undo stop after applying the edits.
    pub push_stack_element_after: bool,
    /// Closing characters this operation auto-inserted, in POST-edit
    /// coordinates. Pass them to
    /// [`super::CursorCollection::track_auto_closed`] (see
    /// [`super::AutoClosedRanges`]).
    pub auto_closed: Vec<AutoClosedPair>,
    /// A second operation to apply right after this one (after its undo
    /// stop), in coordinates of the text after this one's edits. Monaco uses
    /// this to auto-close multi-character pairs such as `begin`/`end` so
    /// that undo first removes only the auto-inserted closing text.
    pub follow_up: Option<Box<EditOperation>>,
    /// Indices into `edits` of the edits that insert auto whitespace (Monaco
    /// `isAutoWhitespaceEdit`, from a command's `insertsAutoWhitespace`):
    /// indentation from Enter and Tab that the model removes again on the
    /// next edit if it is all that's left on its line
    /// ([`crate::ModelOptions::trim_auto_whitespace`]).
    pub auto_whitespace: Vec<usize>,
}

impl EditOperation {
    /// A batch of `edits` (in current coordinates) with undo stops around
    /// it; the selections follow the edits (Monaco `executeEdits` without an
    /// end cursor state).
    pub fn from_edits(
        model: &impl CursorModel,
        selections: &[Selection],
        edits: Vec<(Range<Position>, String)>,
    ) -> Self {
        execute_edits(model, selections, edits, None)
    }

    /// An operation that changes nothing.
    pub(crate) fn none(selections: Vec<Selection>, kind: EditOperationType) -> Self {
        Self {
            edits: Vec::new(),
            selections_after: selections,
            kind,
            push_stack_element_before: false,
            push_stack_element_after: false,
            auto_closed: Vec::new(),
            follow_up: None,
            auto_whitespace: Vec::new(),
        }
    }
}

/// Monaco `getTypingOperation`.
pub(crate) fn typing_operation(typed_text: &str, previous: EditOperationType) -> EditOperationType {
    if typed_text == " " {
        if matches!(
            previous,
            EditOperationType::TypingFirstSpace | EditOperationType::TypingConsecutiveSpace
        ) {
            EditOperationType::TypingConsecutiveSpace
        } else {
            EditOperationType::TypingFirstSpace
        }
    } else {
        EditOperationType::TypingOther
    }
}

fn is_typing_operation(t: EditOperationType) -> bool {
    matches!(
        t,
        EditOperationType::TypingOther
            | EditOperationType::TypingFirstSpace
            | EditOperationType::TypingConsecutiveSpace
    )
}

/// Monaco `shouldPushStackElementBetween`.
pub(crate) fn should_push_stack_element_between(
    previous: EditOperationType,
    current: EditOperationType,
) -> bool {
    if is_typing_operation(previous) && !is_typing_operation(current) {
        // Always set an undo stop before non-type operations.
        return true;
    }
    if previous == EditOperationType::TypingFirstSpace {
        // `abc |d`: no undo stop; `abc  |d`: undo stop.
        return false;
    }
    let normalize = |t| match t {
        EditOperationType::TypingFirstSpace | EditOperationType::TypingConsecutiveSpace => None,
        t => Some(t),
    };
    normalize(previous) != normalize(current)
}

// ---------------------------------------------------------------------------
// Commands

/// Monaco `TrackedRangeStickiness`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stickiness {
    #[allow(dead_code)]
    AlwaysGrowsWhenTypingAtEdges,
    NeverGrowsWhenTypingAtEdges,
    GrowsOnlyWhenTypingBefore,
    GrowsOnlyWhenTypingAfter,
}

impl Stickiness {
    fn start_sticks_to_previous(self) -> bool {
        matches!(
            self,
            Self::AlwaysGrowsWhenTypingAtEdges | Self::GrowsOnlyWhenTypingBefore
        )
    }

    fn end_sticks_to_previous(self) -> bool {
        matches!(
            self,
            Self::NeverGrowsWhenTypingAtEdges | Self::GrowsOnlyWhenTypingBefore
        )
    }
}

struct RawOp {
    major: usize,
    minor: usize,
    range: Range<Position>,
    text: String,
    force_move_markers: bool,
    /// Monaco commands sometimes add a no-op edit just so their cursor state
    /// gets computed; such edits are not reported.
    is_noop: bool,
    auto_whitespace: bool,
}

struct Tracked {
    range: Range<Position>,
    reversed: bool,
    stickiness: Stickiness,
}

/// Monaco `IEditOperationBuilder`.
pub(crate) struct EditBuilder<'a> {
    model: &'a dyn CursorModel,
    major: usize,
    minor: usize,
    auto_whitespace: bool,
    ops: Vec<RawOp>,
    tracked: Vec<Tracked>,
}

/// Handle for a selection tracked through the edits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TrackedId(usize);

impl EditBuilder<'_> {
    pub(crate) fn add_edit(&mut self, range: Range<Position>, text: &str) {
        self.add_edit_with(range, text, false);
    }

    pub(crate) fn add_edit_with(
        &mut self,
        range: Range<Position>,
        text: &str,
        force_move_markers: bool,
    ) {
        if range.start == range.end && text.is_empty() {
            // A no-op edit.
            return;
        }
        let text = text::normalize_line_endings(text).into_owned();
        self.push_op(range, text, force_move_markers, false);
    }

    /// Monaco's `addEditOperation(range, null)` on an empty range: makes the
    /// command count as having edits without changing the text.
    pub(crate) fn add_noop(&mut self, at: Position) {
        self.push_op(at..at, String::new(), false, true);
    }

    fn push_op(
        &mut self,
        range: Range<Position>,
        text: String,
        force_move_markers: bool,
        is_noop: bool,
    ) {
        self.ops.push(RawOp {
            major: self.major,
            minor: self.minor,
            range,
            text,
            force_move_markers,
            is_noop,
            auto_whitespace: self.auto_whitespace,
        });
        self.minor += 1;
    }

    /// Monaco `trackSelection`.
    pub(crate) fn track_selection(
        &mut self,
        selection: Selection,
        track_previous_on_empty: Option<bool>,
    ) -> TrackedId {
        let stickiness = default_stickiness(self.model, selection, track_previous_on_empty);
        self.tracked.push(Tracked {
            range: selection.range(),
            reversed: selection.is_reversed(),
            stickiness,
        });
        TrackedId(self.tracked.len() - 1)
    }
}

fn default_stickiness(
    model: &dyn CursorModel,
    selection: Selection,
    track_previous_on_empty: Option<bool>,
) -> Stickiness {
    if !selection.is_empty() {
        return Stickiness::NeverGrowsWhenTypingAtEdges;
    }
    let previous = track_previous_on_empty.unwrap_or_else(|| {
        // Try to lock it with surrounding text.
        selection.start().column == model.line_max_column(selection.start().row)
    });
    if previous {
        Stickiness::GrowsOnlyWhenTypingBefore
    } else {
        Stickiness::GrowsOnlyWhenTypingAfter
    }
}

/// Monaco `ICursorStateComputerData`.
pub(crate) struct CursorStateHelper<'a> {
    inverse: Vec<Range<Position>>,
    tracked: &'a [Selection],
}

impl CursorStateHelper<'_> {
    /// Where this command's edits ended up, in the order they were added.
    pub(crate) fn inverse_edit_ranges(&self) -> &[Range<Position>] {
        &self.inverse
    }

    pub(crate) fn tracked_selection(&self, id: TrackedId) -> Selection {
        self.tracked[id.0]
    }
}

/// Monaco `ICommand`.
pub(crate) trait Command {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder);
    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection;
    /// The closing character this command auto-inserted (Monaco
    /// `BaseTypeWithAutoClosingCommand.closeCharacterRange`/`enclosingRange`).
    fn auto_closed(&self, _helper: &CursorStateHelper) -> Option<AutoClosedPair> {
        None
    }
    /// Monaco `ICommand.insertsAutoWhitespace`.
    fn inserts_auto_whitespace(&self) -> bool {
        false
    }
}

/// A command whose edits insert auto whitespace (Monaco's
/// `insertsAutoWhitespace` constructor argument of the replace commands).
struct AutoWhitespace(Box<dyn Command>);

impl Command for AutoWhitespace {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        self.0.edit_operations(model, builder);
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        self.0.compute_cursor_state(helper)
    }

    fn auto_closed(&self, helper: &CursorStateHelper) -> Option<AutoClosedPair> {
        self.0.auto_closed(helper)
    }

    fn inserts_auto_whitespace(&self) -> bool {
        true
    }
}

/// Marks `command` as inserting auto whitespace.
pub(crate) fn auto_whitespace(command: Option<Box<dyn Command>>) -> Option<Box<dyn Command>> {
    command.map(|command| Box::new(AutoWhitespace(command)) as Box<dyn Command>)
}

/// Monaco `ReplaceCommand`: replaces `range`, cursor after the new text.
pub(crate) struct ReplaceCommand {
    pub range: Range<Position>,
    pub text: String,
}

impl ReplaceCommand {
    pub(crate) fn boxed(
        range: Range<Position>,
        text: impl Into<String>,
    ) -> Option<Box<dyn Command>> {
        Some(Box::new(Self {
            range,
            text: text.into(),
        }))
    }
}

impl Command for ReplaceCommand {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit(self.range.clone(), &self.text);
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        Selection::collapsed(helper.inverse_edit_ranges()[0].end)
    }
}

/// Monaco `ReplaceOvertypeCommand`: like [`ReplaceCommand`], but also
/// replaces as many characters after the range as the text adds (never past
/// the end of the line). Used in overtype mode.
pub(crate) struct ReplaceOvertypeCommand {
    pub range: Range<Position>,
    pub text: String,
}

impl ReplaceOvertypeCommand {
    pub(crate) fn boxed(
        range: Range<Position>,
        text: impl Into<String>,
    ) -> Option<Box<dyn Command>> {
        Some(Box::new(Self {
            range,
            text: text.into(),
        }))
    }
}

impl Command for ReplaceOvertypeCommand {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let start = self.range.start;
        let end = self.range.end;
        let delta = self.text.encode_utf16().count() as i64 - if start == end { 0 } else { 1 };
        // Monaco `addPositiveOffsetToModelPosition`, clamped to the line.
        let column = (end.column as i64 + delta)
            .clamp(end.column as i64, model.line_max_column(end.row) as i64);
        builder.add_edit(start..Position::new(end.row, column as u32), &self.text);
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        Selection::collapsed(helper.inverse_edit_ranges()[0].end)
    }
}

/// Monaco `ReplaceCommandWithoutChangingPosition`: cursor before the new
/// text.
pub(crate) struct ReplaceCommandWithoutChangingPosition {
    pub range: Range<Position>,
    pub text: String,
}

impl Command for ReplaceCommandWithoutChangingPosition {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit(self.range.clone(), &self.text);
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        Selection::collapsed(helper.inverse_edit_ranges()[0].start)
    }
}

/// Monaco `ReplaceCommandWithOffsetCursorState`: cursor after the new text,
/// shifted by the given deltas.
pub(crate) struct ReplaceCommandWithOffsetCursorState {
    pub range: Range<Position>,
    pub text: String,
    pub line_delta: i32,
    pub column_delta: i32,
}

impl Command for ReplaceCommandWithOffsetCursorState {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit(self.range.clone(), &self.text);
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let end = helper.inverse_edit_ranges()[0].end;
        Selection::collapsed(Position::new(
            (end.row as i64 + self.line_delta as i64).max(0) as u32,
            (end.column as i64 + self.column_delta as i64).max(0) as u32,
        ))
    }
}

/// Monaco `ReplaceCommandThatPreservesSelection`: keeps `selection` where it
/// is relative to the surrounding text.
pub(crate) struct ReplaceCommandThatPreservesSelection {
    pub range: Range<Position>,
    pub text: String,
    pub selection: Selection,
    pub force_move_markers: bool,
    pub tracked: Option<TrackedId>,
}

impl Command for ReplaceCommandThatPreservesSelection {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit_with(self.range.clone(), &self.text, self.force_move_markers);
        self.tracked = Some(builder.track_selection(self.selection, None));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        helper.tracked_selection(self.tracked.expect("tracked in edit_operations"))
    }
}

// ---------------------------------------------------------------------------
// Executor

/// Monaco `Range.compareRangesUsingEnds`.
fn cmp_using_ends(a: &Range<Position>, b: &Range<Position>) -> std::cmp::Ordering {
    (a.end, a.start).cmp(&(b.end, b.start))
}

/// Monaco `CommandExecutor._getLoserCursorMap`: when edits of two cursors
/// overlap, the later cursor loses all its edits.
fn loser_cursors(ops: &[RawOp]) -> Vec<usize> {
    let mut ops: Vec<(Range<Position>, usize)> =
        ops.iter().map(|op| (op.range.clone(), op.major)).collect();
    ops.sort_by(|a, b| cmp_using_ends(&b.0, &a.0));
    let mut losers = Vec::new();
    let mut i: isize = 1;
    while (i as usize) < ops.len() {
        let previous = &ops[i as usize - 1];
        let current = &ops[i as usize];
        if previous.0.start < current.0.end {
            let loser_major = previous.1.max(current.1);
            losers.push(loser_major);
            let mut j: isize = 0;
            while (j as usize) < ops.len() {
                if ops[j as usize].1 == loser_major {
                    ops.remove(j as usize);
                    if j < i {
                        i -= 1;
                    }
                    j -= 1;
                }
                j += 1;
            }
            if i > 0 {
                i -= 1;
            }
        }
        i += 1;
    }
    losers
}

/// The extent of `text`: (line breaks, UTF-16 length of the last line).
fn text_extent(text: &str) -> (u32, u32) {
    match text.rfind('\n') {
        Some(i) => (
            text.matches('\n').count() as u32,
            super::strings::utf16_len(&text[i + 1..]),
        ),
        None => (0, super::strings::utf16_len(text)),
    }
}

fn advance(start: Position, text: &str) -> Position {
    let (lines, last) = text_extent(text);
    if lines == 0 {
        Position::new(start.row, start.column + last)
    } else {
        Position::new(start.row + lines, last)
    }
}

/// UTF-16 offset distance from `a` to `b` (`a <= b`), counting each line
/// break as one unit.
fn offset_between(model: &dyn CursorModel, a: Position, b: Position) -> u64 {
    if a.row == b.row {
        return b.column.saturating_sub(a.column) as u64;
    }
    let mut result =
        (model.line_max_column(a.row) - a.column.min(model.line_max_column(a.row))) as u64 + 1;
    for row in a.row + 1..b.row {
        result += model.line_max_column(row) as u64 + 1;
    }
    result + b.column as u64
}

/// Position `units` UTF-16 code units into `text` inserted at `start`.
fn position_in_text(start: Position, text: &str, units: u64) -> Position {
    let mut count = 0u64;
    let mut end = text.len();
    for (i, c) in text.char_indices() {
        if count >= units {
            end = i;
            break;
        }
        count += c.len_utf16() as u64;
    }
    advance(start, &text[..end])
}

struct AppliedOp<'a> {
    start: Position,
    end: Position,
    text: &'a str,
    deleting: u64,
    inserting: u64,
    force_move_markers: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MoveSemantics {
    MarkerDefined,
    ForceMove,
    ForceStay,
}

/// Monaco `adjustMarkerBeforeColumn`: whether the marker stays put.
fn marker_stays(
    marker: u64,
    stick_to_previous: bool,
    check: u64,
    semantics: MoveSemantics,
) -> bool {
    if marker < check {
        return true;
    }
    if marker > check {
        return false;
    }
    match semantics {
        MoveSemantics::ForceMove => false,
        MoveSemantics::ForceStay => true,
        MoveSemantics::MarkerDefined => stick_to_previous,
    }
}

/// Monaco `nodeAcceptEdit` for a single marker, in position space.
fn adjust_marker(
    model: &dyn CursorModel,
    marker: Position,
    stick_to_previous: bool,
    op: &AppliedOp,
) -> Position {
    if marker < op.start {
        return marker;
    }
    if marker > op.end {
        // Shift by the edit's delta.
        let inserted_end = advance(op.start, op.text);
        return if marker.row == op.end.row {
            Position::new(
                inserted_end.row,
                inserted_end.column + (marker.column - op.end.column),
            )
        } else {
            Position::new(marker.row - op.end.row + inserted_end.row, marker.column)
        };
    }
    let m = offset_between(model, op.start, marker);
    let common = op.deleting.min(op.inserting);
    let semantics = if op.force_move_markers {
        MoveSemantics::ForceMove
    } else if op.deleting > 0 {
        MoveSemantics::ForceStay
    } else {
        MoveSemantics::MarkerDefined
    };
    if marker_stays(m, stick_to_previous, 0, semantics) {
        return op.start;
    }
    if common > 0 && !op.force_move_markers {
        let semantics = if op.deleting > op.inserting {
            MoveSemantics::ForceStay
        } else {
            MoveSemantics::MarkerDefined
        };
        if marker_stays(m, stick_to_previous, common, semantics) {
            return position_in_text(op.start, op.text, m);
        }
    }
    // Either the marker is inside the deleted text, or right at its end:
    // it ends up after the inserted text.
    advance(op.start, op.text)
}

fn adjust_range(
    model: &dyn CursorModel,
    range: Range<Position>,
    stickiness: Stickiness,
    ops: &[AppliedOp],
) -> Range<Position> {
    let mut start = range.start;
    let mut end = range.end;
    // Monaco applies edits bottom to top, so earlier positions are still in
    // original coordinates when each edit is processed.
    for op in ops.iter().rev() {
        start = adjust_marker(model, start, stickiness.start_sticks_to_previous(), op);
        end = adjust_marker(model, end, stickiness.end_sticks_to_previous(), op);
        if start > end {
            end = start;
        }
    }
    start..end
}

/// Runs `commands` (one per entry of `selections_before`, `None` for cursors
/// that do nothing) and assembles the result (Monaco
/// `CommandExecutor.executeCommands` + `_executeEditOperation`).
pub(crate) fn execute_commands(
    model: &dyn CursorModel,
    selections_before: &[Selection],
    mut commands: Vec<Option<Box<dyn Command + '_>>>,
    kind: EditOperationType,
    push_stack_element_before: bool,
    push_stack_element_after: bool,
) -> EditOperation {
    let mut result = EditOperation {
        edits: Vec::new(),
        selections_after: selections_before.to_vec(),
        kind,
        push_stack_element_before,
        push_stack_element_after,
        auto_closed: Vec::new(),
        follow_up: None,
        auto_whitespace: Vec::new(),
    };

    let mut builder = EditBuilder {
        model,
        major: 0,
        minor: 0,
        auto_whitespace: false,
        ops: Vec::new(),
        tracked: Vec::new(),
    };
    for (i, command) in commands.iter_mut().enumerate() {
        if let Some(command) = command {
            builder.major = i;
            builder.minor = 0;
            builder.auto_whitespace = command.inserts_auto_whitespace();
            command.edit_operations(model, &mut builder);
        }
    }
    if builder.ops.is_empty() {
        return result;
    }

    let losers = loser_cursors(&builder.ops);
    if losers.contains(&0) {
        // These commands are very messed up; Monaco ignores them.
        return result;
    }
    let mut ops: Vec<RawOp> = builder
        .ops
        .into_iter()
        .filter(|op| !losers.contains(&op.major))
        .collect();
    // Monaco `_sortOpsAscending`; the sort is stable, so ties keep the order
    // the edits were added in.
    ops.sort_by(|a, b| cmp_using_ends(&a.range, &b.range));

    // Monaco `_getInverseEditRanges`.
    let mut inverse: Vec<Range<Position>> = Vec::with_capacity(ops.len());
    let mut previous: Option<(&RawOp, Position)> = None;
    for op in &ops {
        let start = match previous {
            Some((prev, prev_new_end)) => {
                if prev.range.end.row == op.range.start.row {
                    Position::new(
                        prev_new_end.row,
                        prev_new_end.column + (op.range.start.column - prev.range.end.column),
                    )
                } else {
                    Position::new(
                        prev_new_end.row + (op.range.start.row - prev.range.end.row),
                        op.range.start.column,
                    )
                }
            }
            None => op.range.start,
        };
        let end = advance(start, &op.text);
        inverse.push(start..end);
        previous = Some((op, end));
    }

    let applied: Vec<AppliedOp> = ops
        .iter()
        .map(|op| AppliedOp {
            start: op.range.start,
            end: op.range.end,
            text: &op.text,
            deleting: offset_between(model, op.range.start, op.range.end),
            inserting: op.text.encode_utf16().count() as u64,
            force_move_markers: op.force_move_markers,
        })
        .collect();
    let tracked: Vec<Selection> = builder
        .tracked
        .iter()
        .map(|t| {
            Selection::from_range(
                adjust_range(model, t.range.clone(), t.stickiness, &applied),
                t.reversed,
            )
        })
        .collect();

    let mut selections_after = Vec::with_capacity(selections_before.len());
    let mut auto_closed = Vec::new();
    for (i, before) in selections_before.iter().enumerate() {
        let mut own: Vec<(usize, Range<Position>)> = ops
            .iter()
            .zip(&inverse)
            .filter(|(op, _)| op.major == i)
            .map(|(op, r)| (op.minor, r.clone()))
            .collect();
        let command = commands.get(i).and_then(|c| c.as_ref());
        let selection = match command {
            Some(command) if !own.is_empty() => {
                own.sort_by_key(|(minor, _)| *minor);
                let helper = CursorStateHelper {
                    inverse: own.into_iter().map(|(_, r)| r).collect(),
                    tracked: &tracked,
                };
                if !losers.contains(&i)
                    && let Some(pair) = command.auto_closed(&helper)
                {
                    auto_closed.push(pair);
                }
                command.compute_cursor_state(&helper)
            }
            _ => {
                // Monaco keeps the old selection; we also carry it through the
                // edits so it stays on the same text.
                let stickiness = default_stickiness(model, *before, None);
                Selection::from_range(
                    adjust_range(model, before.range(), stickiness, &applied),
                    before.is_reversed(),
                )
            }
        };
        selections_after.push(selection);
    }
    let mut losers = losers;
    losers.sort_unstable_by(|a, b| b.cmp(a));
    for loser in losers {
        if loser < selections_after.len() {
            selections_after.remove(loser);
        }
    }

    let ops: Vec<RawOp> = ops.into_iter().filter(|op| !op.is_noop).collect();
    result.auto_whitespace = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.auto_whitespace)
        .map(|(i, _)| i)
        .collect();
    result.edits = ops.into_iter().map(|op| (op.range, op.text)).collect();
    result.selections_after = selections_after;
    result.auto_closed = auto_closed;
    result
}

/// Applies `edits` (in pre-edit coordinates) as one batch with undo stops
/// around it (Monaco `editor.executeEdits(source, edits, endCursorState)`).
/// Without `end_state` the selections follow the edits like Monaco's cursor
/// markers do (`readSelectionFromMarkers`): ranges that grow when typing at
/// their edges, with empty selections collapsed to the end.
pub(crate) fn execute_edits(
    model: &dyn CursorModel,
    selections_before: &[Selection],
    edits: Vec<(Range<Position>, String)>,
    end_state: Option<Vec<Selection>>,
) -> EditOperation {
    let edits = edits
        .into_iter()
        .map(|(range, text)| (range, text, false))
        .collect();
    execute_edits_forcing(model, selections_before, edits, end_state)
}

/// [`execute_edits`] where each edit says whether it forces the cursors at
/// its edges to move after its text (Monaco `forceMoveMarkers`).
pub(crate) fn execute_edits_forcing(
    model: &dyn CursorModel,
    selections_before: &[Selection],
    mut edits: Vec<(Range<Position>, String, bool)>,
    end_state: Option<Vec<Selection>>,
) -> EditOperation {
    edits.retain(|(range, text, _)| range.start != range.end || !text.is_empty());
    edits.sort_by(|a, b| cmp_using_ends(&a.0, &b.0));
    let selections_after = match end_state {
        Some(selections) if !selections.is_empty() => selections,
        _ => {
            let applied: Vec<AppliedOp> = edits
                .iter()
                .map(|(range, text, force)| AppliedOp {
                    start: range.start,
                    end: range.end,
                    text,
                    deleting: offset_between(model, range.start, range.end),
                    inserting: text.encode_utf16().count() as u64,
                    force_move_markers: *force,
                })
                .collect();
            selections_before
                .iter()
                .map(|s| {
                    let range = adjust_range(
                        model,
                        s.range(),
                        Stickiness::AlwaysGrowsWhenTypingAtEdges,
                        &applied,
                    );
                    if s.is_empty() {
                        Selection::collapsed(range.end)
                    } else {
                        Selection::from_range(range, s.is_reversed())
                    }
                })
                .collect()
        }
    };
    EditOperation {
        edits: edits
            .into_iter()
            .map(|(range, text, _)| (range, text))
            .collect(),
        selections_after,
        kind: EditOperationType::Other,
        push_stack_element_before: true,
        push_stack_element_after: true,
        auto_closed: Vec::new(),
        follow_up: None,
        auto_whitespace: Vec::new(),
    }
}

/// Carries `ranges` (in pre-edit coordinates) through `edits` (a batch as in
/// [`EditOperation::edits`]) the way Monaco moves decorations with
/// `TrackedRangeStickiness.NeverGrowsWhenTypingAtEdges`.
pub(crate) fn transform_ranges(
    model: &dyn CursorModel,
    ranges: &mut [Range<Position>],
    edits: &[(Range<Position>, String)],
) {
    if edits.is_empty() {
        return;
    }
    let mut sorted: Vec<&(Range<Position>, String)> = edits.iter().collect();
    sorted.sort_by(|a, b| cmp_using_ends(&a.0, &b.0));
    let applied: Vec<AppliedOp> = sorted
        .iter()
        .map(|(range, text)| AppliedOp {
            start: range.start,
            end: range.end,
            text,
            deleting: offset_between(model, range.start, range.end),
            inserting: text.encode_utf16().count() as u64,
            force_move_markers: false,
        })
        .collect();
    for range in ranges {
        *range = adjust_range(
            model,
            range.clone(),
            Stickiness::NeverGrowsWhenTypingAtEdges,
            &applied,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_stack_element_rules() {
        use EditOperationType::*;
        assert!(!should_push_stack_element_between(TypingOther, TypingOther));
        assert!(!should_push_stack_element_between(
            TypingFirstSpace,
            TypingOther
        ));
        assert!(should_push_stack_element_between(
            TypingConsecutiveSpace,
            TypingOther
        ));
        assert!(should_push_stack_element_between(TypingOther, DeletingLeft));
        assert!(!should_push_stack_element_between(
            DeletingLeft,
            DeletingLeft
        ));
        assert_eq!(
            typing_operation(" ", TypingFirstSpace),
            TypingConsecutiveSpace
        );
    }
}
