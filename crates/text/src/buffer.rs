//! [`Buffer`]: a versioned rope with batched edits, anchors and
//! operation-based undo/redo.

use std::any::Any;
use std::ops::{Deref, Range};
use std::sync::Arc;

use sum_tree::Bias;

use crate::line_ending::{LineEnding, normalize_line_endings};
use crate::patch::{Edit, Patch};
use crate::rope::Rope;

/// Monotonic version number: the count of applied patches. An edit bumps it
/// by one; an undo or redo bumps it once per edit it reverts or reapplies.
pub type Version = usize;

/// Opaque state saved with an undo transaction, typically the editor's cursor
/// state before and after the change (Monaco's `beforeCursorState`).
pub type TransactionMetadata = Arc<dyn Any + Send + Sync>;

/// Identifies one undo step. Ids increase in creation order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransactionId(u64);

/// Why [`Buffer::edit`] rejected a batch of edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// Two edits in a batch overlap. Monaco rejects these too.
    OverlappingRanges {
        /// The earlier range, after clipping and merging touching ranges.
        first: Range<usize>,
        /// The range that overlaps it.
        second: Range<usize>,
    },
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::OverlappingRanges { first, second } => {
                write!(f, "overlapping edit ranges {first:?} and {second:?}")
            }
        }
    }
}

impl std::error::Error for EditError {}

/// A position that follows the text as it is edited.
///
/// `bias` decides what happens when text is inserted exactly at the anchor:
/// [`Bias::Left`] stays before the inserted text, [`Bias::Right`] moves after
/// it. An anchor inside deleted text collapses to the deletion point.
///
/// Anchors are plain values: they record the version they were created at and
/// are transformed through every later edit when resolved, so resolution costs
/// `O(edits since creation)`. Long-lived holders should [`Buffer::refresh`]
/// their anchors after edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Anchor {
    offset: usize,
    bias: Bias,
    version: Version,
}

impl Anchor {
    /// The start of the text, whatever is inserted there.
    pub const MIN: Anchor = Anchor {
        offset: 0,
        bias: Bias::Left,
        version: 0,
    };

    /// The end of the text, whatever is appended.
    pub const MAX: Anchor = Anchor {
        offset: usize::MAX,
        bias: Bias::Right,
        version: 0,
    };

    /// Which side of an insertion at this position the anchor stays on.
    pub fn bias(&self) -> Bias {
        self.bias
    }
}

/// How a tracked range behaves when text is typed at its edges. Mirrors
/// Monaco's `TrackedRangeStickiness`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Stickiness {
    /// Text typed at either edge is included in the range.
    #[default]
    AlwaysGrowsWhenTypingAtEdges,
    /// Text typed at either edge is excluded from the range.
    NeverGrowsWhenTypingAtEdges,
    /// Text typed at the start is included; text typed at the end is not.
    GrowsOnlyWhenTypingBefore,
    /// Text typed at the end is included; text typed at the start is not.
    GrowsOnlyWhenTypingAfter,
}

impl Stickiness {
    /// Biases for the start and end anchors of a range.
    pub fn biases(self) -> (Bias, Bias) {
        match self {
            Stickiness::AlwaysGrowsWhenTypingAtEdges => (Bias::Left, Bias::Right),
            Stickiness::NeverGrowsWhenTypingAtEdges => (Bias::Right, Bias::Left),
            Stickiness::GrowsOnlyWhenTypingBefore => (Bias::Left, Bias::Left),
            Stickiness::GrowsOnlyWhenTypingAfter => (Bias::Right, Bias::Right),
        }
    }
}

/// An immutable, cheaply clonable view of a buffer at one version.
#[derive(Clone, Debug)]
pub struct BufferSnapshot {
    rope: Rope,
    version: Version,
    line_ending: LineEnding,
}

impl BufferSnapshot {
    /// The buffer version this snapshot was taken at.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The text, with `\n` line breaks.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// The buffer's line ending at the time of the snapshot.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }
}

impl Deref for BufferSnapshot {
    type Target = Rope;

    fn deref(&self) -> &Rope {
        &self.rope
    }
}

/// Result of [`Buffer::undo`] or [`Buffer::redo`].
#[derive(Clone)]
pub struct UndoOutcome {
    /// The undone or redone transaction.
    pub transaction_id: TransactionId,
    /// How the text changed, from the version before the call to the current one.
    pub patch: Patch,
    /// The cursor state to restore: the transaction's `before` metadata for
    /// undo, `after` for redo.
    pub metadata: Option<TransactionMetadata>,
}

#[derive(Clone)]
struct RecordedEdit {
    old_range: Range<usize>,
    new_range: Range<usize>,
    old_text: String,
    new_text: String,
}

#[derive(Clone)]
struct Transaction {
    id: TransactionId,
    /// Each batch was applied atomically; batches were applied in order.
    batches: Vec<Vec<RecordedEdit>>,
    group: Option<u64>,
    sealed: bool,
    /// Reopened by [`Buffer::pop_stack_element`]: any edit joins it until
    /// it's sealed again.
    reopened: bool,
    before: Option<TransactionMetadata>,
    after: Option<TransactionMetadata>,
    /// The alternative version before and after it (Monaco
    /// `beforeVersionId`/`afterVersionId`).
    before_version: Version,
    after_version: Version,
}

#[derive(Default, Clone)]
struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Explicit transaction being built by `start_transaction`.
    open: Option<Transaction>,
    open_depth: usize,
    next_id: u64,
}

impl History {
    fn new_transaction(
        &mut self,
        group: Option<u64>,
        before: Option<TransactionMetadata>,
        before_version: Version,
    ) -> Transaction {
        self.next_id += 1;
        Transaction {
            id: TransactionId(self.next_id),
            batches: Vec::new(),
            group,
            sealed: false,
            reopened: false,
            before,
            after: None,
            before_version,
            after_version: before_version,
        }
    }
}

/// A versioned text buffer.
///
/// Text is stored with `\n` line breaks; [`Buffer::line_ending`] is applied by
/// [`Buffer::text`] and normalized away from inserted text, like a Monaco
/// `TextModel` with a single EOL. Offsets are byte offsets into that
/// normalized text.
#[derive(Clone)]
pub struct Buffer {
    rope: Rope,
    /// `patches[v]` transforms version `v` into version `v + 1`.
    patches: Vec<Patch>,
    history: History,
    line_ending: LineEnding,
    has_bom: bool,
    /// Monaco's alternative version id: the version, except that undo and
    /// redo go back to the one the text had then.
    alternative_version: Version,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new("")
    }
}

impl Buffer {
    /// Creates a buffer from file contents: detects the line ending (LF when
    /// there are no line breaks), normalizes to `\n` and strips a BOM.
    pub fn new(text: &str) -> Self {
        Self::with_default_line_ending(text, LineEnding::Lf)
    }

    /// Like [`Buffer::new`], but uses `default` as the line ending when `text`
    /// has no line breaks.
    pub fn with_default_line_ending(text: &str, default: LineEnding) -> Self {
        let (text, has_bom) = match text.strip_prefix('\u{feff}') {
            Some(rest) => (rest, true),
            None => (text, false),
        };
        Self {
            line_ending: LineEnding::detect(text, default),
            rope: Rope::from(&*normalize_line_endings(text)),
            patches: Vec::new(),
            history: History::default(),
            has_bom,
            alternative_version: 0,
        }
    }

    /// The current version. Each applied edit batch bumps it by one; undo and
    /// redo bump it once per batch in the transaction.
    pub fn version(&self) -> Version {
        self.patches.len()
    }

    /// The version, except that undoing or redoing returns to the version
    /// the text had then (Monaco `getAlternativeVersionId`): equal values
    /// mean equal text, e.g. to tell whether undo got back to the saved
    /// state.
    pub fn alternative_version(&self) -> Version {
        self.alternative_version
    }

    /// The text, with `\n` line breaks.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// A cheap immutable copy of the current text and version.
    pub fn snapshot(&self) -> BufferSnapshot {
        BufferSnapshot {
            rope: self.rope.clone(),
            version: self.version(),
            line_ending: self.line_ending,
        }
    }

    /// The line ending used by [`Buffer::text`].
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Changes the line ending used by [`Buffer::text`]. The stored text and
    /// offsets are unaffected, so this is not an undoable edit.
    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        self.line_ending = line_ending;
    }

    /// Whether the original text started with a UTF-8 BOM.
    pub fn has_bom(&self) -> bool {
        self.has_bom
    }

    /// The text with the buffer's line ending and without a BOM (Monaco's
    /// `getValue()`).
    pub fn text(&self) -> String {
        match self.line_ending {
            LineEnding::Lf => self.rope.to_string(),
            LineEnding::Crlf => {
                let mut out =
                    String::with_capacity(self.rope.len() + self.rope.max_point().row as usize);
                for chunk in self.rope.chunks() {
                    let mut parts = chunk.split('\n');
                    if let Some(first) = parts.next() {
                        out.push_str(first);
                    }
                    for part in parts {
                        out.push_str("\r\n");
                        out.push_str(part);
                    }
                }
                out
            }
        }
    }

    /// [`Buffer::text`] with the BOM restored, for saving.
    pub fn file_text(&self) -> String {
        let text = self.text();
        if self.has_bom {
            format!("\u{feff}{text}")
        } else {
            text
        }
    }

    /// Applies `edits` atomically as its own undo transaction (or as part of
    /// the open transaction, see [`Buffer::start_transaction`]).
    ///
    /// Ranges are byte offsets in the current text, clamped and clipped to
    /// char boundaries. They may be given in any order but must not overlap;
    /// touching ranges are merged. Inserted text has its line breaks
    /// normalized.
    pub fn edit<I, S>(&mut self, edits: I) -> Result<Patch, EditError>
    where
        I: IntoIterator<Item = (Range<usize>, S)>,
        S: AsRef<str>,
    {
        self.edit_internal(edits, None)
    }

    /// Like [`Buffer::edit`], but merged into the previous undo transaction if
    /// that transaction used the same `group` and no
    /// [`Buffer::push_stack_element`] happened since. This is how consecutive
    /// typing becomes a single undo step.
    pub fn edit_grouped<I, S>(&mut self, edits: I, group: u64) -> Result<Patch, EditError>
    where
        I: IntoIterator<Item = (Range<usize>, S)>,
        S: AsRef<str>,
    {
        self.edit_internal(edits, Some(group))
    }

    fn edit_internal<I, S>(&mut self, edits: I, group: Option<u64>) -> Result<Patch, EditError>
    where
        I: IntoIterator<Item = (Range<usize>, S)>,
        S: AsRef<str>,
    {
        let edits = self.normalize_edits(edits)?;
        if edits.is_empty() {
            return Ok(Patch::default());
        }
        let before_version = self.alternative_version;
        let (patch, recorded) = self.apply(edits);
        self.alternative_version = self.version();
        self.record(recorded, group, before_version);
        Ok(patch)
    }

    fn normalize_edits<I, S>(&self, edits: I) -> Result<Vec<(Range<usize>, String)>, EditError>
    where
        I: IntoIterator<Item = (Range<usize>, S)>,
        S: AsRef<str>,
    {
        let mut edits: Vec<(Range<usize>, String)> = edits
            .into_iter()
            .map(|(range, text)| {
                let start = self
                    .rope
                    .clip_offset(range.start.min(range.end), Bias::Left);
                let end = self
                    .rope
                    .clip_offset(range.end.max(range.start), Bias::Right);
                (
                    start..end,
                    normalize_line_endings(text.as_ref()).into_owned(),
                )
            })
            .collect();
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        let mut merged: Vec<(Range<usize>, String)> = Vec::with_capacity(edits.len());
        for (range, text) in edits {
            if let Some((last_range, last_text)) = merged.last_mut() {
                if last_range.end > range.start {
                    return Err(EditError::OverlappingRanges {
                        first: last_range.clone(),
                        second: range,
                    });
                }
                if last_range.end == range.start {
                    last_range.end = range.end;
                    last_text.push_str(&text);
                    continue;
                }
            }
            merged.push((range, text));
        }
        merged.retain(|(range, text)| !(range.is_empty() && text.is_empty()));
        Ok(merged)
    }

    /// Applies normalized edits without touching undo history.
    fn apply(&mut self, edits: Vec<(Range<usize>, String)>) -> (Patch, Vec<RecordedEdit>) {
        let old = std::mem::take(&mut self.rope);
        let mut new = Rope::new();
        let mut patch_edits = Vec::with_capacity(edits.len());
        let mut recorded = Vec::with_capacity(edits.len());
        let mut prev_end = 0;
        for (range, text) in edits {
            new.append(old.slice(prev_end..range.start));
            let new_start = new.len();
            new.push(&text);
            let new_range = new_start..new.len();
            recorded.push(RecordedEdit {
                old_text: old.text_for_range(range.clone()),
                old_range: range.clone(),
                new_range: new_range.clone(),
                new_text: text,
            });
            patch_edits.push(Edit {
                old: range.clone(),
                new: new_range,
            });
            prev_end = range.end;
        }
        new.append(old.slice(prev_end..old.len()));
        self.rope = new;
        let patch = Patch::new(patch_edits);
        self.patches.push(patch.clone());
        (patch, recorded)
    }

    fn record(&mut self, batch: Vec<RecordedEdit>, group: Option<u64>, before_version: Version) {
        let after_version = self.version();
        let history = &mut self.history;
        history.redo.clear();
        if let Some(open) = history.open.as_mut() {
            open.batches.push(batch);
            open.after_version = after_version;
            return;
        }
        if let Some(last) = history.undo.last_mut()
            && !last.sealed
            && (last.reopened || group.is_some_and(|group| last.group == Some(group)))
        {
            last.batches.push(batch);
            last.after_version = after_version;
            return;
        }
        if let Some(last) = history.undo.last_mut() {
            last.sealed = true;
        }
        let mut transaction = history.new_transaction(group, None, before_version);
        transaction.batches.push(batch);
        transaction.after_version = after_version;
        transaction.sealed = group.is_none();
        history.undo.push(transaction);
    }

    /// Starts an explicit transaction: every edit until the matching
    /// [`Buffer::end_transaction`] becomes one undo step. Nests.
    pub fn start_transaction(&mut self, before: Option<TransactionMetadata>) {
        let history = &mut self.history;
        if history.open_depth == 0 {
            let transaction = history.new_transaction(None, before, self.alternative_version);
            history.open = Some(transaction);
        }
        history.open_depth += 1;
    }

    /// Ends the transaction started by [`Buffer::start_transaction`]. Returns
    /// its id when the outermost transaction closes and it contains edits.
    pub fn end_transaction(&mut self, after: Option<TransactionMetadata>) -> Option<TransactionId> {
        let history = &mut self.history;
        debug_assert!(history.open_depth > 0, "end_transaction without start");
        history.open_depth = history.open_depth.saturating_sub(1);
        if history.open_depth > 0 {
            return None;
        }
        let mut transaction = history.open.take()?;
        if transaction.batches.is_empty() {
            return None;
        }
        transaction.after = after;
        // A reopened step takes it in (Monaco appends to the open element).
        if let Some(last) = history.undo.last_mut()
            && last.reopened
            && !last.sealed
        {
            last.batches.append(&mut transaction.batches);
            last.after_version = transaction.after_version;
            if transaction.after.is_some() {
                last.after = transaction.after;
            }
            return Some(last.id);
        }
        transaction.sealed = true;
        if let Some(last) = history.undo.last_mut() {
            last.sealed = true;
        }
        let id = transaction.id;
        history.undo.push(transaction);
        Some(id)
    }

    /// Prevents the next grouped edit from merging into the current undo
    /// step (Monaco's `pushStackElement`).
    pub fn push_stack_element(&mut self) {
        if let Some(last) = self.history.undo.last_mut() {
            last.sealed = true;
            last.reopened = false;
        }
    }

    /// Reopens the most recent undo step so the next edits join it, until
    /// [`Buffer::push_stack_element`] (Monaco's `popStackElement`).
    pub fn pop_stack_element(&mut self) {
        if let Some(last) = self.history.undo.last_mut() {
            last.sealed = false;
            last.reopened = true;
        }
    }

    /// Sets the cursor state saved with the most recent undo step. `before` is
    /// only recorded if the step has none yet, so grouped typing keeps the
    /// state from before its first edit; `after` always replaces.
    pub fn set_last_transaction_metadata(
        &mut self,
        before: Option<TransactionMetadata>,
        after: Option<TransactionMetadata>,
    ) {
        // An open transaction is the step being recorded.
        let history = &mut self.history;
        let last = match history.open.as_mut() {
            Some(open) => Some(open),
            None => history.undo.last_mut(),
        };
        if let Some(last) = last {
            if last.before.is_none() {
                last.before = before;
            }
            if after.is_some() {
                last.after = after;
            }
        }
    }

    /// Drops undo and redo history (Monaco `setValue` does this). Versions
    /// and patches are kept so anchors and consumers stay valid.
    /// An open transaction keeps nesting, but forgets the edits it recorded
    /// so far, so closing it adds no undo step for them.
    pub fn clear_history(&mut self) {
        let history = &mut self.history;
        history.undo.clear();
        history.redo.clear();
        if let Some(open) = history.open.as_mut() {
            open.batches.clear();
            open.before = None;
        }
    }

    /// The id of the most recent undo step, if any.
    pub fn last_transaction_id(&self) -> Option<TransactionId> {
        self.history.undo.last().map(|t| t.id)
    }

    /// The id of the transaction being recorded, if one is open, else of the
    /// most recent undo step.
    pub fn current_transaction_id(&self) -> Option<TransactionId> {
        match &self.history.open {
            Some(open) => Some(open.id),
            None => self.last_transaction_id(),
        }
    }

    /// Whether there is an undo step to revert.
    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    /// Whether there is an undone step to re-apply.
    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    /// Reverts the most recent undo step.
    pub fn undo(&mut self) -> Option<UndoOutcome> {
        debug_assert!(self.history.open.is_none(), "undo inside a transaction");
        let mut transaction = self.history.undo.pop()?;
        let mut patch = Patch::default();
        for batch in transaction.batches.iter_mut().rev() {
            let inverse = batch
                .iter()
                .map(|edit| (edit.new_range.clone(), edit.old_text.clone()))
                .collect();
            let (step, _) = self.apply(inverse);
            patch = patch.compose(&step);
        }
        transaction.sealed = true;
        transaction.reopened = false;
        self.alternative_version = transaction.before_version;
        let outcome = UndoOutcome {
            transaction_id: transaction.id,
            patch,
            metadata: transaction.before.clone(),
        };
        self.history.redo.push(transaction);
        Some(outcome)
    }

    /// Re-applies the most recently undone step.
    pub fn redo(&mut self) -> Option<UndoOutcome> {
        let transaction = self.history.redo.pop()?;
        let mut patch = Patch::default();
        for batch in &transaction.batches {
            let forward = batch
                .iter()
                .map(|edit| (edit.old_range.clone(), edit.new_text.clone()))
                .collect();
            let (step, _) = self.apply(forward);
            patch = patch.compose(&step);
        }
        self.alternative_version = transaction.after_version;
        let outcome = UndoOutcome {
            transaction_id: transaction.id,
            patch,
            metadata: transaction.after.clone(),
        };
        self.history.undo.push(transaction);
        Some(outcome)
    }

    /// All changes since `version`, as one patch.
    pub fn edits_since(&self, version: Version) -> Patch {
        self.patches[version.min(self.patches.len())..]
            .iter()
            .fold(Patch::default(), |acc, patch| acc.compose(patch))
    }

    /// An anchor at `offset` (clipped to a char boundary) in the current text.
    pub fn anchor_at(&self, offset: usize, bias: Bias) -> Anchor {
        Anchor {
            offset: self.rope.clip_offset(offset, bias),
            bias,
            version: self.version(),
        }
    }

    /// An anchor at `offset` with [`Bias::Left`]: it stays before text inserted there.
    pub fn anchor_before(&self, offset: usize) -> Anchor {
        self.anchor_at(offset, Bias::Left)
    }

    /// An anchor at `offset` with [`Bias::Right`]: it moves after text inserted there.
    pub fn anchor_after(&self, offset: usize) -> Anchor {
        self.anchor_at(offset, Bias::Right)
    }

    /// Anchors for a range with Monaco-style `stickiness`.
    pub fn anchor_range(&self, range: Range<usize>, stickiness: Stickiness) -> Range<Anchor> {
        let (start_bias, end_bias) = stickiness.biases();
        let start = self.anchor_at(range.start, start_bias);
        let end = self.anchor_at(range.end, end_bias);
        start..end
    }

    /// The current offset of `anchor`.
    pub fn resolve(&self, anchor: &Anchor) -> usize {
        if anchor.offset == usize::MAX {
            return self.rope.len();
        }
        let mut offset = anchor.offset;
        for patch in &self.patches[anchor.version.min(self.patches.len())..] {
            offset = patch.transform_offset(offset, anchor.bias);
        }
        offset.min(self.rope.len())
    }

    /// The current range of an anchor range. Collapses to the start if the
    /// ends crossed.
    pub fn resolve_range(&self, range: &Range<Anchor>) -> Range<usize> {
        let start = self.resolve(&range.start);
        let end = self.resolve(&range.end).max(start);
        start..end
    }

    /// The same position as `anchor`, re-based on the current version so later
    /// resolution is cheap.
    pub fn refresh(&self, anchor: &Anchor) -> Anchor {
        if anchor.offset == usize::MAX {
            return *anchor;
        }
        Anchor {
            offset: self.resolve(anchor),
            bias: anchor.bias,
            version: self.version(),
        }
    }
}

impl Deref for Buffer {
    type Target = Rope;

    fn deref(&self) -> &Rope {
        &self.rope
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn crlf_round_trip() {
        let text = "\u{feff}one\r\ntwo\r\nthree";
        let buffer = Buffer::new(text);
        assert_eq!(buffer.line_ending(), LineEnding::Crlf);
        assert!(buffer.has_bom());
        assert_eq!(buffer.rope(), "one\ntwo\nthree");
        assert_eq!(buffer.text(), "one\r\ntwo\r\nthree");
        assert_eq!(buffer.file_text(), text);
    }

    #[test]
    fn edits_normalize_and_merge() {
        let mut buffer = Buffer::new("hello world");
        let patch = buffer
            .edit([(6..11, "there\r\nfriend"), (0..5, "HELLO"), (5..6, "_")])
            .unwrap();
        assert_eq!(buffer.rope(), "HELLO_there\nfriend");
        assert_eq!(
            patch.edits(),
            &[Edit {
                old: 0..11,
                new: 0..18
            }]
        );
        assert!(matches!(
            buffer.edit([(0..3, "a"), (2..4, "b")]),
            Err(EditError::OverlappingRanges { .. })
        ));
    }

    #[test]
    fn anchors_follow_edits() {
        let mut buffer = Buffer::new("abcdef");
        let left = buffer.anchor_before(3);
        let right = buffer.anchor_after(3);
        let inside = buffer.anchor_before(4);
        buffer.edit([(3..3, "XY")]).unwrap();
        assert_eq!(buffer.resolve(&left), 3);
        assert_eq!(buffer.resolve(&right), 5);
        buffer.edit([(5..7, "")]).unwrap(); // deletes "de"
        assert_eq!(buffer.resolve(&inside), 5);
        buffer.edit([(0..0, ">>")]).unwrap();
        assert_eq!(buffer.resolve(&left), 5);
        assert_eq!(buffer.resolve(&Anchor::MAX), buffer.len());
        assert_eq!(buffer.resolve(&Anchor::MIN), 0);
        let refreshed = buffer.refresh(&right);
        assert_eq!(buffer.resolve(&refreshed), buffer.resolve(&right));
    }

    #[test]
    fn stickiness() {
        let mut buffer = Buffer::new("[abc]");
        let grows = buffer.anchor_range(1..4, Stickiness::AlwaysGrowsWhenTypingAtEdges);
        let never = buffer.anchor_range(1..4, Stickiness::NeverGrowsWhenTypingAtEdges);
        buffer.edit([(1..1, "<"), (4..4, ">")]).unwrap();
        assert_eq!(buffer.resolve_range(&grows), 1..6);
        assert_eq!(buffer.resolve_range(&never), 2..5);
    }

    #[test]
    fn grouped_typing_undoes_together() {
        let mut buffer = Buffer::new("");
        for (i, ch) in ["a", "b", "c"].into_iter().enumerate() {
            buffer.edit_grouped([(i..i, ch)], 1).unwrap();
        }
        buffer.push_stack_element();
        buffer.edit_grouped([(3..3, "d")], 1).unwrap();
        buffer.edit([(0..0, ">")]).unwrap();
        assert_eq!(buffer.rope(), ">abcd");
        buffer.undo().unwrap();
        assert_eq!(buffer.rope(), "abcd");
        buffer.undo().unwrap();
        assert_eq!(buffer.rope(), "abc");
        let outcome = buffer.undo().unwrap();
        assert_eq!(buffer.rope(), "");
        assert_eq!(
            outcome.patch.edits(),
            &[Edit {
                old: 0..3,
                new: 0..0
            }]
        );
        assert!(!buffer.can_undo());
        buffer.redo().unwrap();
        assert_eq!(buffer.rope(), "abc");
        buffer.redo().unwrap();
        buffer.redo().unwrap();
        assert_eq!(buffer.rope(), ">abcd");
        assert!(!buffer.can_redo());
    }

    #[test]
    fn transactions_and_metadata() {
        let mut buffer = Buffer::new("x");
        buffer.start_transaction(Some(Arc::new("before")));
        buffer.edit([(0..0, "1")]).unwrap();
        buffer.start_transaction(None);
        buffer.edit([(2..2, "2")]).unwrap();
        assert_eq!(buffer.end_transaction(None), None);
        let id = buffer.end_transaction(Some(Arc::new("after")));
        assert!(id.is_some());
        assert_eq!(buffer.rope(), "1x2");
        let undo = buffer.undo().unwrap();
        assert_eq!(buffer.rope(), "x");
        assert_eq!(
            undo.metadata.unwrap().downcast_ref::<&str>(),
            Some(&"before")
        );
        let redo = buffer.redo().unwrap();
        assert_eq!(
            redo.metadata.unwrap().downcast_ref::<&str>(),
            Some(&"after")
        );
        // A new edit clears the redo stack.
        buffer.undo();
        buffer.edit([(0..0, "z")]).unwrap();
        assert!(!buffer.can_redo());
    }

    #[test]
    fn clear_history_inside_a_transaction() {
        let mut buffer = Buffer::new("x");
        buffer.start_transaction(None);
        buffer.edit([(0..0, "1")]).unwrap();
        buffer.clear_history();
        buffer.edit([(2..2, "2")]).unwrap();
        assert!(buffer.end_transaction(None).is_some());
        // Only the edit after clearing is undoable.
        buffer.undo().unwrap();
        assert_eq!(buffer.rope(), "1x");
        assert!(!buffer.can_undo());

        // Clearing with nothing after it leaves no undo step.
        buffer.start_transaction(None);
        buffer.edit([(0..0, "3")]).unwrap();
        buffer.clear_history();
        assert_eq!(buffer.end_transaction(None), None);
        assert!(!buffer.can_undo());
    }

    proptest! {
        #[test]
        fn undo_redo_and_edits_since(
            text in "[a-c\n]{0,20}",
            ops in proptest::collection::vec((any::<usize>(), any::<usize>(), "[x-z\n]{0,3}", any::<bool>()), 1..12),
        ) {
            let mut buffer = Buffer::new(&text);
            for (a, b, insert, grouped) in ops {
                let len = buffer.len();
                let (start, end) = (a % (len + 1), b % (len + 1));
                let range = start.min(end)..start.max(end);
                let before = buffer.snapshot();
                let patch = if grouped {
                    buffer.edit_grouped([(range, insert)], 7).unwrap()
                } else {
                    buffer.edit([(range, insert)]).unwrap()
                };
                // The patch describes exactly the change.
                let mut rebuilt = String::new();
                let mut prev = 0;
                for edit in patch.edits() {
                    rebuilt.push_str(&before.text_for_range(prev..edit.old.start));
                    rebuilt.push_str(&buffer.text_for_range(edit.new.clone()));
                    prev = edit.old.end;
                }
                rebuilt.push_str(&before.text_for_range(prev..before.len()));
                prop_assert_eq!(&rebuilt, &buffer.rope().to_string());
            }
            // edits_since(0) turns the original text into the current text.
            let since = buffer.edits_since(0);
            let mut rebuilt = String::new();
            let mut prev = 0;
            for edit in since.edits() {
                rebuilt.push_str(&text[prev..edit.old.start]);
                rebuilt.push_str(&buffer.text_for_range(edit.new.clone()));
                prev = edit.old.end;
            }
            rebuilt.push_str(&text[prev..]);
            prop_assert_eq!(&rebuilt, &buffer.rope().to_string());

            let final_text = buffer.rope().to_string();
            while buffer.undo().is_some() {}
            prop_assert_eq!(buffer.rope().to_string(), text.clone());
            while buffer.redo().is_some() {}
            prop_assert_eq!(buffer.rope().to_string(), final_text);
        }
    }

    /// Monaco `cursor.test.ts` "issue #90973: Undo brings back model
    /// alternative version", plus redo.
    #[test]
    fn undo_brings_back_the_alternative_version() {
        let mut buffer = Buffer::new("");
        let (before, before_alt) = (buffer.version(), buffer.alternative_version());
        for (i, ch) in "Hello".chars().enumerate() {
            buffer.edit_grouped([(i..i, ch.to_string())], 1).unwrap();
        }
        let typed_alt = buffer.alternative_version();
        assert_eq!(typed_alt, buffer.version());
        buffer.undo().unwrap();
        assert_ne!(buffer.version(), before);
        assert_eq!(buffer.alternative_version(), before_alt);
        buffer.redo().unwrap();
        assert_eq!(buffer.alternative_version(), typed_alt);
    }

    /// Monaco `popStackElement`: the next edits join the last undo step.
    #[test]
    fn pop_stack_element_reopens_the_last_step() {
        let mut buffer = Buffer::new("");
        buffer.edit([(0..0, "a")]).unwrap();
        buffer.edit([(1..1, "b")]).unwrap();
        buffer.pop_stack_element();
        buffer.edit([(2..2, "c")]).unwrap();
        buffer.start_transaction(None);
        buffer.edit([(3..3, "d")]).unwrap();
        buffer.end_transaction(None);
        buffer.push_stack_element();
        buffer.edit([(4..4, "e")]).unwrap();
        buffer.undo().unwrap();
        assert_eq!(buffer.text(), "abcd");
        // "b", "c" and "d" were one step.
        buffer.undo().unwrap();
        assert_eq!(buffer.text(), "a");
    }
}
