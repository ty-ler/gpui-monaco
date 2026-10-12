//! Remembering which closing characters the editor auto-inserted.
//!
//! Ports `AutoClosedAction` and the auto-closed bookkeeping of `Cursor`
//! (`_pushAutoClosedAction`, `_validateAutoClosedActions`,
//! `getAutoClosedCharacters`) from Monaco's
//! `src/vs/editor/common/cursor/cursor.ts` (MIT). Monaco keeps these ranges
//! as model decorations; here they are plain ranges that the caller carries
//! through edits.

use std::ops::Range;

use super::edit::transform_ranges;
use super::{CursorModel, Position, Selection};

/// A closing character the editor inserted while auto-closing a pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoClosedPair {
    /// The auto-inserted closing text.
    pub closing: Range<Position>,
    /// From the typed opening character through the closing text. Tracking
    /// stops once a cursor leaves it.
    pub enclosing: Range<Position>,
}

/// One typing operation's auto-closed pairs (one per cursor).
#[derive(Clone, Debug, PartialEq, Eq)]
struct AutoClosedAction {
    closing: Vec<Range<Position>>,
    enclosing: Vec<Range<Position>>,
}

impl AutoClosedAction {
    /// Monaco `AutoClosedAction.isValid`: every cursor must be strictly inside
    /// its enclosing range, and the ranges must stay on one line.
    fn is_valid(&self, selections: &[Selection]) -> bool {
        let mut enclosing = self.enclosing.clone();
        if enclosing.iter().any(|r| r.start.row != r.end.row) {
            return false;
        }
        enclosing.sort_by_key(|r| (r.start, r.end));
        let mut selections: Vec<Range<Position>> = selections.iter().map(|s| s.range()).collect();
        selections.sort_by_key(|r| (r.start, r.end));
        for (i, selection) in selections.iter().enumerate() {
            let Some(range) = enclosing.get(i) else {
                return false;
            };
            // Monaco `Range.strictContainsRange`.
            if !(range.start < selection.start && selection.end < range.end) {
                return false;
            }
        }
        true
    }
}

/// The closing characters the editor auto-inserted and has not given up on
/// yet. Typing such a character again types over it, and backspace between
/// it and its opening character deletes both (with
/// [`super::AutoClosingEditStrategy::Auto`], the default).
///
/// Every [`super::CursorCollection`] owns one (see
/// [`super::CursorCollection::auto_closed_ranges`]); the cursor collection
/// validates it whenever its cursors change. Whoever applies edits keeps it
/// in sync with the text:
///
/// 1. For an [`super::EditOperation`] from the cursor core, call
///    [`super::CursorCollection::track_auto_closed`] with the text *before*
///    the edits, the operation's `edits` and its `auto_closed`, apply the
///    edits, then [`super::CursorCollection::set_selections`] with
///    `selections_after`. Then do the same for its `follow_up`, if any.
/// 2. For any other edit (undo, redo, paste from elsewhere, collaborators),
///    [`AutoClosedRanges::transform`] or [`AutoClosedRanges::map`] the
///    ranges through it, as Monaco's decorations follow every edit, then
///    set the cursors. [`AutoClosedRanges::clear`] forgets them instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoClosedRanges {
    actions: Vec<AutoClosedAction>,
}

impl AutoClosedRanges {
    /// The auto-inserted closing characters (Monaco
    /// `getAutoClosedCharacters`).
    pub fn closing_ranges(&self) -> Vec<Range<Position>> {
        self.actions
            .iter()
            .flat_map(|a| a.closing.iter().cloned())
            .collect()
    }

    /// True if no auto-closed characters are tracked.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Forgets all auto-closed characters.
    pub fn clear(&mut self) {
        self.actions.clear();
    }

    /// Carries the ranges through `edits` (non-overlapping, in coordinates of
    /// `model_before`, the text before they are applied).
    pub fn transform(
        &mut self,
        model_before: &impl CursorModel,
        edits: &[(Range<Position>, String)],
    ) {
        if edits.is_empty() {
            return;
        }
        let model: &dyn CursorModel = model_before;
        for action in &mut self.actions {
            transform_ranges(model, &mut action.closing, edits);
            transform_ranges(model, &mut action.enclosing, edits);
        }
    }

    /// Moves every range with `f` (from coordinates before an edit to after
    /// it), for edits known only by how they move positions. Map starts
    /// after text inserted at them and ends before it, as
    /// [`Self::transform`] does (Monaco's `NeverGrowsWhenTypingAtEdges`).
    pub fn map(&mut self, mut f: impl FnMut(Range<Position>) -> Range<Position>) {
        for action in &mut self.actions {
            for range in action.closing.iter_mut().chain(&mut action.enclosing) {
                *range = f(range.clone());
            }
        }
    }

    /// Records the pairs one operation auto-closed (Monaco
    /// `_pushAutoClosedAction`), in coordinates after its edits.
    pub fn push(&mut self, pairs: &[AutoClosedPair]) {
        if pairs.is_empty() {
            return;
        }
        self.actions.push(AutoClosedAction {
            closing: pairs.iter().map(|p| p.closing.clone()).collect(),
            enclosing: pairs.iter().map(|p| p.enclosing.clone()).collect(),
        });
    }

    /// Drops the pairs whose cursors left them (Monaco
    /// `_validateAutoClosedActions`).
    pub fn validate(&mut self, selections: &[Selection]) {
        self.actions.retain(|a| a.is_valid(selections));
    }
}
