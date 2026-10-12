//! Cursor state and the multi-cursor collection.
//!
//! Ports `SingleCursorState` from Monaco's
//! `src/vs/editor/common/cursorCommon.ts`, `Cursor` state validation from
//! `src/vs/editor/common/cursor/oneCursor.ts` and `CursorCollection` from
//! `src/vs/editor/common/cursor/cursorCollection.ts` (MIT).

use std::ops::Range;

use super::auto_closed::{AutoClosedPair, AutoClosedRanges};
use super::column::ColumnSelectData;
use super::composition::CompositionState;
use super::{CursorModel, Position, Selection, validate_position, validate_range};

/// How the selection was started, which decides how extending it behaves
/// (Monaco `SelectionStartKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionStartKind {
    /// A plain position or selection; extending moves only the head.
    Simple,
    /// Started by selecting a word; extending keeps that word selected and
    /// snaps the head to word boundaries.
    Word,
    /// Started by selecting a line; extending keeps that line selected and
    /// snaps the head to whole lines.
    Line,
}

/// The state of one cursor (Monaco `SingleCursorState`).
///
/// `selection_start` is a range rather than a position so word and line
/// selections can keep the initially selected word or line selected while
/// being extended in either direction. The leftover visible columns remember
/// how far right of the actual column the cursor "wants" to be after moving
/// vertically through shorter lines; they can be negative when the cursor
/// snapped to the right of its desired visible column (e.g. past a tab).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SingleCursorState {
    /// The range the selection was started from; empty for a
    /// [`SelectionStartKind::Simple`] start.
    pub selection_start: Range<Position>,
    /// How the selection was started.
    pub selection_start_kind: SelectionStartKind,
    /// Leftover visible columns of the selection start, used when it moves
    /// vertically (e.g. [`super::add_cursor_above`]).
    pub selection_start_leftover_visible_columns: i32,
    /// The cursor position (the selection's head).
    pub position: Position,
    /// Leftover visible columns of `position`.
    pub leftover_visible_columns: i32,
}

impl SingleCursorState {
    /// A collapsed cursor at `p` with no leftover columns.
    pub fn from_position(p: Position) -> Self {
        Self {
            selection_start: p..p,
            selection_start_kind: SelectionStartKind::Simple,
            selection_start_leftover_visible_columns: 0,
            position: p,
            leftover_visible_columns: 0,
        }
    }

    /// Monaco `CursorState.fromModelSelection`.
    pub fn from_selection(s: Selection) -> Self {
        Self {
            selection_start: s.anchor..s.anchor,
            selection_start_kind: SelectionStartKind::Simple,
            selection_start_leftover_visible_columns: 0,
            position: s.head,
            leftover_visible_columns: 0,
        }
    }

    /// Monaco `SingleCursorState._computeSelection`.
    pub fn selection(&self) -> Selection {
        let start = &self.selection_start;
        if start.start == start.end || self.position > start.start {
            Selection::new(start.start, self.position)
        } else {
            Selection::new(start.end, self.position)
        }
    }

    /// True if the selection or the selection start is non-empty (Monaco
    /// `SingleCursorState.hasSelection`).
    pub fn has_selection(&self) -> bool {
        !self.selection().is_empty() || self.selection_start.start != self.selection_start.end
    }

    /// Monaco `SingleCursorState.move`: moves just the position when
    /// extending the selection, or everything otherwise.
    pub fn move_to(
        &self,
        in_selection_mode: bool,
        p: Position,
        leftover_visible_columns: i32,
    ) -> Self {
        if in_selection_mode {
            Self {
                selection_start: self.selection_start.clone(),
                selection_start_kind: self.selection_start_kind,
                selection_start_leftover_visible_columns: self
                    .selection_start_leftover_visible_columns,
                position: p,
                leftover_visible_columns,
            }
        } else {
            Self {
                selection_start: p..p,
                selection_start_kind: SelectionStartKind::Simple,
                selection_start_leftover_visible_columns: leftover_visible_columns,
                position: p,
                leftover_visible_columns,
            }
        }
    }

    /// Monaco `Cursor._setState` model-state validation.
    pub(crate) fn validated(&self, model: &dyn CursorModel) -> Self {
        let selection_start = validate_range(model, self.selection_start.clone());
        let selection_start_leftover_visible_columns = if selection_start == self.selection_start {
            self.selection_start_leftover_visible_columns
        } else {
            0
        };
        let position = validate_position(model, self.position);
        let leftover_visible_columns = if position == self.position {
            self.leftover_visible_columns
        } else {
            0
        };
        Self {
            selection_start,
            selection_start_kind: self.selection_start_kind,
            selection_start_leftover_visible_columns,
            position,
            leftover_visible_columns,
        }
    }
}

/// All cursors of an editor (Monaco `CursorCollection`). The first state is
/// the primary cursor; there is always at least one.
#[derive(Clone, Debug)]
pub struct CursorCollection {
    states: Vec<SingleCursorState>,
    /// Index into `states` of the most recently added cursor (0 if none).
    last_added_cursor_index: usize,
    /// The rectangle of an ongoing column selection; cleared whenever the
    /// cursors are set by anything else.
    column_select_data: Option<ColumnSelectData>,
    /// Closing characters the editor auto-inserted.
    auto_closed: AutoClosedRanges,
    /// The ongoing IME composition, if any.
    composition: Option<CompositionState>,
    /// Whether overlapping selections merge (Monaco
    /// `multiCursorMergeOverlapping`, default `true`).
    merge_overlapping: bool,
}

impl Default for CursorCollection {
    fn default() -> Self {
        Self::new(SingleCursorState::from_position(Position::zero()))
    }
}

impl CursorCollection {
    /// A collection holding just `state` as the primary cursor.
    pub fn new(state: SingleCursorState) -> Self {
        Self {
            states: vec![state],
            last_added_cursor_index: 0,
            column_select_data: None,
            auto_closed: AutoClosedRanges::default(),
            composition: None,
            merge_overlapping: true,
        }
    }

    /// The primary cursor.
    pub fn primary(&self) -> &SingleCursorState {
        &self.states[0]
    }

    /// All cursors, primary first.
    pub fn all(&self) -> &[SingleCursorState] {
        &self.states
    }

    /// The selection of every cursor, primary first.
    pub fn selections(&self) -> Vec<Selection> {
        self.states.iter().map(|s| s.selection()).collect()
    }

    /// Index of the most recently added secondary cursor, or 0 (Monaco
    /// `getLastAddedCursorIndex`).
    pub fn last_added_cursor_index(&self) -> usize {
        if self.states.len() == 1 {
            0
        } else {
            self.last_added_cursor_index
        }
    }

    /// Replaces all cursor states (the first becomes primary), then merges
    /// overlapping cursors. Empty `states` are ignored.
    pub fn set_states(&mut self, states: Vec<SingleCursorState>) {
        if states.is_empty() {
            return;
        }
        self.column_select_data = None;
        let old_len = self.states.len();
        let new_len = states.len();
        if new_len > old_len {
            // Monaco `_addSecondaryCursor` marks each added cursor as last added.
            self.last_added_cursor_index = new_len - 1;
        } else {
            for _ in new_len..old_len {
                let remove_index = self.states.len() - 2;
                self.remove_secondary_cursor_bookkeeping(remove_index);
                self.states.pop();
            }
        }
        self.states = states;
        self.normalize();
        self.validate_auto_closed();
    }

    /// Like [`CursorCollection::set_states`] with plain selections (Monaco
    /// `CursorState.fromModelSelections`).
    pub fn set_selections(&mut self, selections: &[Selection]) {
        self.set_states(
            selections
                .iter()
                .map(|s| SingleCursorState::from_selection(*s))
                .collect(),
        );
    }

    /// The ongoing column selection, if the cursors were last set by
    /// [`super::column_select`] or [`super::column_select_to`].
    pub fn column_select_data(&self) -> Option<ColumnSelectData> {
        self.column_select_data
    }

    pub(crate) fn set_column_select_data(&mut self, data: Option<ColumnSelectData>) {
        self.column_select_data = data;
    }

    /// Removes all secondary cursors.
    pub fn kill_secondary_cursors(&mut self) {
        self.states.truncate(1);
        self.last_added_cursor_index = 0;
        self.validate_auto_closed();
    }

    /// The closing characters the editor auto-inserted (see
    /// [`AutoClosedRanges`]). Validated against the cursors whenever they
    /// change.
    pub fn auto_closed_ranges(&self) -> &AutoClosedRanges {
        &self.auto_closed
    }

    /// Mutable access to [`Self::auto_closed_ranges`].
    pub fn auto_closed_ranges_mut(&mut self) -> &mut AutoClosedRanges {
        &mut self.auto_closed
    }

    /// Keeps [`Self::auto_closed_ranges`] in sync with an edit operation:
    /// carries the existing ranges through `edits` and records the newly
    /// `auto_closed` pairs. Call it with the text *before* the edits are
    /// applied (`edits` and `auto_closed` are an [`super::EditOperation`]'s
    /// fields), then apply the edits and set the operation's
    /// `selections_after`, which drops pairs the cursors have left.
    pub fn track_auto_closed(
        &mut self,
        model_before: &impl CursorModel,
        edits: &[(Range<Position>, String)],
        auto_closed: &[AutoClosedPair],
    ) {
        self.auto_closed.transform(model_before, edits);
        self.auto_closed.push(auto_closed);
    }

    /// Starts an IME composition (Monaco `Cursor.startComposition`):
    /// records the cursors' lines to tell what it typed when it ends. Until
    /// [`Self::end_composition`], typing skips most language interceptors
    /// (see [`super::type_with_language`]).
    pub fn start_composition(&mut self, model: &impl CursorModel) {
        self.composition = Some(CompositionState::new(model, &self.selections()));
    }

    /// Whether an IME composition is in progress (Monaco's
    /// `isDoingComposition`).
    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    /// Ends the IME composition, returning what was recorded when it
    /// started, for [`super::composition_end_with_language`].
    pub fn end_composition(&mut self) -> Option<CompositionState> {
        self.composition.take()
    }

    fn validate_auto_closed(&mut self) {
        if !self.auto_closed.is_empty() {
            let selections = self.selections();
            self.auto_closed.validate(&selections);
        }
    }

    /// Clamps every cursor into `model` (e.g. after an external edit) and
    /// merges cursors that now overlap.
    pub fn validate(&mut self, model: &impl CursorModel) {
        let model: &dyn CursorModel = model;
        for state in &mut self.states {
            *state = state.validated(model);
        }
        self.normalize();
        self.validate_auto_closed();
    }

    pub(crate) fn validated_states(
        &mut self,
        model: &dyn CursorModel,
        states: Vec<SingleCursorState>,
    ) {
        self.set_states(states.iter().map(|s| s.validated(model)).collect());
    }

    fn remove_secondary_cursor_bookkeeping(&mut self, remove_index: usize) {
        if self.last_added_cursor_index > remove_index {
            self.last_added_cursor_index -= 1;
        }
    }

    /// Monaco `CursorCollection.normalize`: merges overlapping cursors (and
    /// touching ones if one of them is collapsed). The cursor added earlier
    /// wins, so the primary cursor survives.
    /// Sets whether overlapping selections merge into one (Monaco
    /// `multiCursorMergeOverlapping`).
    pub fn set_merge_overlapping(&mut self, merge: bool) {
        self.merge_overlapping = merge;
        if merge {
            self.normalize();
        }
    }

    fn normalize(&mut self) {
        if self.states.len() == 1 || !self.merge_overlapping {
            return;
        }
        struct Sorted {
            index: usize,
            selection: Selection,
        }
        let mut sorted: Vec<Sorted> = self
            .states
            .iter()
            .enumerate()
            .map(|(index, s)| Sorted {
                index,
                selection: s.selection(),
            })
            .collect();
        sorted.sort_by(|a, b| {
            (a.selection.start(), a.selection.end()).cmp(&(b.selection.start(), b.selection.end()))
        });

        let mut i = 0;
        while i + 1 < sorted.len() {
            let current = sorted[i].selection;
            let next = sorted[i + 1].selection;
            let should_merge = if next.is_empty() || current.is_empty() {
                next.start() <= current.end()
            } else {
                next.start() < current.end()
            };
            if !should_merge {
                i += 1;
                continue;
            }
            let (winner_sorted, loser_sorted) = if sorted[i].index < sorted[i + 1].index {
                (i, i + 1)
            } else {
                (i + 1, i)
            };
            let loser_index = sorted[loser_sorted].index;
            let winner_index = sorted[winner_sorted].index;
            let loser_selection = sorted[loser_sorted].selection;
            let winner_selection = sorted[winner_sorted].selection;
            if loser_selection != winner_selection {
                let start = loser_selection.start().min(winner_selection.start());
                let end = loser_selection.end().max(winner_selection.end());
                let loser_ltr = loser_selection.anchor == loser_selection.start();
                let winner_ltr = winner_selection.anchor == winner_selection.start();
                let ltr = if loser_index == self.last_added_cursor_index {
                    self.last_added_cursor_index = winner_index;
                    loser_ltr
                } else {
                    winner_ltr
                };
                let resulting = if ltr {
                    Selection::new(start, end)
                } else {
                    Selection::new(end, start)
                };
                sorted[winner_sorted].selection = resulting;
                self.states[winner_index] = SingleCursorState::from_selection(resulting);
            }
            for s in &mut sorted {
                if s.index > loser_index {
                    s.index -= 1;
                }
            }
            sorted.remove(loser_sorted);
            self.remove_secondary_cursor_bookkeeping(loser_index - 1);
            self.states.remove(loser_index);
            // Stay at `i`: re-examine the merged cursor against its new
            // neighbour.
        }
    }
}
