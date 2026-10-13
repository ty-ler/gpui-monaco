//! Inlay hints: provider text shown inline, such as inferred types and
//! parameter names.
//!
//! Ports Monaco's `src/vs/editor/contrib/inlayHints/browser/`
//! (`inlayHintsController.ts`, `inlayHintsHover.ts`,
//! `inlayHintsLocations.ts`, `inlayHints.ts`) (MIT): label parts with their
//! own hover (with the hover of a part's location), Cmd/Ctrl+click to a
//! part's location or command, a context menu of navigation commands for a
//! part's location, double-click to apply a hint's text edits, the hints'
//! font, spacing and rounded boxes, hint lengths kept on the cursor's line
//! while typing, and Monaco's cache of each model's hints.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::Shared;
use gpui::{App, Bounds, Context, EntityId, Global, Modifiers, Pixels, Point, Task, Window, px};
use text::PointUtf16;
use web_time::Instant;

use crate::api::DecorationsCollection;
use crate::decorations::{
    DecorationId, DecorationOptions, InjectedText, InjectedTextBox, InjectedTextCursorStops,
};
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::options::{InlayHintsMode, LocationKind, MultiCursorModifier};
use crate::providers::{
    Command, Documentation, InlayHint, InlayHintKind, InlayHintsEpoch, InlayHintsProvider,
    Location, MarkdownString, hover_providers, inlay_hints_providers,
};
use crate::theme::TokenStyle;

/// Monaco's adaptive debounce starts here.
const SCROLL_DELAY: Duration = Duration::from_millis(25);
/// After edits.
const EDIT_DELAY: Duration = Duration::from_millis(250);
/// Monaco keeps hint lengths on the cursor's line this long after an edit
/// (`Math.max(scheduler.delay, 800)`), then asks for hints again.
const CURSOR_FREEZE: Duration = Duration::from_millis(800);
/// Monaco `InlayHintsCache`'s `LRUCache` size.
const CACHE_SIZE: usize = 50;

/// Monaco `InlayHintsCache`: the hints last shown for each model and
/// version, shown again at once when an editor switches back to the model
/// or its options change (`_cacheHintsForFastRestore`).
#[derive(Default)]
struct InlayHintsCache(Vec<(EntityId, text::Version, Vec<InlayHintItem>)>);

impl Global for InlayHintsCache {}

impl InlayHintsCache {
    fn get(&mut self, model: EntityId, version: &text::Version) -> Option<Vec<InlayHintItem>> {
        let ix = self
            .0
            .iter()
            .position(|(m, v, _)| *m == model && v == version)?;
        let entry = self.0.remove(ix);
        let items = entry.2.clone();
        self.0.push(entry);
        Some(items)
    }

    fn set(&mut self, model: EntityId, version: text::Version, items: Vec<InlayHintItem>) {
        self.0.retain(|(m, v, _)| !(*m == model && *v == version));
        self.0.push((model, version, items));
        if self.0.len() > CACHE_SIZE {
            self.0.remove(0);
        }
    }
}

/// A hint and the provider that gave it (Monaco `InlayHintItem`).
#[derive(Clone)]
struct InlayHintItem {
    hint: InlayHint,
    provider: Arc<dyn InlayHintsProvider>,
    resolved: bool,
    /// A resolve under way.
    pending: Option<Shared<Task<Option<InlayHint>>>>,
}

/// A rendered label part (Monaco `RenderedInlayHintLabelPart`): the
/// generation of the hints it's from, the hint's slot and the part's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InlayHintPart {
    generation: u64,
    item: usize,
    part: usize,
}

/// A shown hint's slot and label part index, for a decoration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PartRef {
    item: usize,
    part: usize,
}

/// A hint and its decorations (label parts and padding, in the order
/// they were added). A hint keeps its slot while it stays, so hints shown
/// elsewhere are left alone when some rows get new ones.
struct InlayHintSlot {
    item: InlayHintItem,
    decorations: Vec<DecorationId>,
}

/// A label part's hover (Monaco `InlayHintsHover`).
pub(crate) struct InlayHintHover {
    /// The hint's position.
    pub position: PointUtf16,
    /// The tooltips and hints on how to follow the part.
    pub contents: Vec<MarkdownString>,
    /// The part's command, with the keys that run it.
    pub command: Option<(Command, String)>,
}

#[derive(Default)]
pub(crate) struct InlayHintState {
    task: Option<Task<()>>,
    /// The hints, by slot (free slots are `None`).
    slots: Vec<Option<InlayHintSlot>>,
    free: Vec<usize>,
    /// The hints' slots in their order (by position).
    order: Vec<usize>,
    /// Bumped whenever the hints are replaced.
    generation: u64,
    decorations: Option<DecorationsCollection>,
    /// The label part each rendered decoration shows.
    parts: HashMap<DecorationId, PartRef>,
    /// The hint (slot) of each rendered decoration, label parts and the
    /// padding that keeps a hint's length.
    decoration_items: HashMap<DecorationId, usize>,
    /// The text version at which every decoration is at its hint's
    /// position: no edit moved them since, so new hints for some rows
    /// replace just those rows' hints, found by position in `order`.
    anchored: Option<text::Version>,
    /// Tests: always read positions from the decorations and sort.
    full_updates: bool,
    /// Monaco `_cursorInfo`: the cursor at the last edit, and until when
    /// the hints before it on its line keep their lengths.
    cursor: Option<(PointUtf16, Instant)>,
    /// Asks for hints again when that's over.
    cursor_task: Option<Task<()>>,
    /// The part under the mouse that has a location or command, and
    /// whether the link modifier is held (Monaco `_activeInlayHintPart`).
    active: Option<(InlayHintPart, bool)>,
    /// Rows the last request covered.
    requested: Option<Range<u32>>,
    /// Ctrl+Alt is held.
    pressed: bool,
    epoch: u64,
}

/// What's shown for each hint: its label parts (as part index and text),
/// after Monaco's per-line `maximumLength`. Text past it is cut with `…`
/// and later hints on the line are dropped.
fn limit_per_line(hints: &[InlayHint], maximum: usize) -> Vec<(usize, Vec<(usize, String)>)> {
    let mut out = Vec::new();
    let mut row = None;
    let mut total = 0;
    for (ix, hint) in hints.iter().enumerate() {
        if row != Some(hint.position.row) {
            row = Some(hint.position.row);
            total = 0;
        }
        if maximum != 0 && total > maximum {
            continue;
        }
        let mut parts = Vec::new();
        for (part_ix, part) in hint.label.parts().into_iter().enumerate() {
            let mut text = part.label.to_string();
            total += text.chars().count();
            let over = if maximum == 0 {
                0
            } else {
                total.saturating_sub(maximum)
            };
            if over > 0 {
                let keep = text.chars().count().saturating_sub(over);
                text = text.chars().take(keep).collect();
                text.push('…');
                parts.push((part_ix, text));
                break;
            }
            parts.push((part_ix, text));
        }
        out.push((ix, parts));
    }
    out
}

/// Monaco's fixed lengths in `_updateHintsDecorators`: a hint's shown
/// parts cut to `fixed` characters (its last one ending in `…`, which
/// Monaco does even when it's exactly that long), and the characters of
/// padding it needs to reach it.
fn fix_length(parts: Vec<(usize, String)>, fixed: usize) -> (Vec<(usize, String)>, usize) {
    let mut out = Vec::with_capacity(parts.len());
    let mut actual = 0;
    for (part_ix, text) in parts {
        let len = text.chars().count();
        actual += len;
        if actual >= fixed {
            let over = actual - fixed;
            // `textlabel.slice(0, -(1 + over)) + '…'`.
            let mut cut: String = text.chars().take(len.saturating_sub(over + 1)).collect();
            cut.push('…');
            out.push((part_ix, cut));
            return (out, 0);
        }
        out.push((part_ix, text));
    }
    (out, fixed - actual)
}

/// Monaco's matching of the old hints' lengths before the cursor (in
/// order) with the new hints there: each new one takes the next length,
/// or 0 when there are more new hints. Lengths left over are dropped, as
/// Monaco's code computes their sum but never stores it.
fn match_fixed_lengths(new_items: &[usize], lengths: &[usize]) -> HashMap<usize, usize> {
    new_items
        .iter()
        .enumerate()
        .map(|(ix, item)| (*item, lengths.get(ix).copied().unwrap_or(0)))
        .collect()
}

/// Monaco `fixSpace`: spaces show as no-break spaces.
fn fix_space(text: &str) -> String {
    text.replace([' ', '\t'], "\u{a0}")
}

impl CodeEditor {
    fn inlay_hints_active(&self, cx: &App) -> bool {
        self.options.inlay_hints.enabled != InlayHintsMode::Off
            && !self.options.single_line
            && !inlay_hints_providers(cx, self.provider_target(cx)).is_empty()
    }

    /// Requests hints for the viewport after `delay` (Monaco's
    /// `RunOnceScheduler`).
    pub(crate) fn schedule_inlay_hints(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if !self.inlay_hints_active(cx) {
            self.inlay.task = None;
            if !self.inlay.order.is_empty() {
                self.set_inlay_hint_items(Vec::new(), None, false, cx);
            }
            return;
        }
        self.inlay.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok((tasks, rows)) = this.update(cx, |this, cx| {
                let tasks = this.request_inlay_hints(cx);
                (tasks, this.inlay.requested.clone())
            }) else {
                return;
            };
            let mut items = Vec::new();
            for (provider, task) in tasks {
                match task.await {
                    Ok(found) => items.extend(found.into_iter().map(|hint| InlayHintItem {
                        hint,
                        provider: provider.clone(),
                        resolved: false,
                        pending: None,
                    })),
                    Err(err) => log::warn!("inlay hints failed: {err:#}"),
                }
            }
            items.sort_by_key(|item| item.hint.position);
            this.update(cx, |this, cx| {
                this.inlay.task = None;
                // Monaco `_updateHintsDecorators` for the requested rows,
                // then `_cacheHintsForFastRestore`.
                this.set_inlay_hint_items(items, rows, true, cx);
                this.cache_inlay_hints(cx);
            })
            .ok();
        }));
    }

    /// The hint in `slot`, which is in use.
    fn inlay_slot(&self, slot: usize) -> &InlayHintSlot {
        self.inlay.slots[slot]
            .as_ref()
            .expect("inlay hint slot in use")
    }

    /// Puts `item` in a free slot.
    fn alloc_inlay_slot(&mut self, item: InlayHintItem) -> usize {
        let slot = InlayHintSlot {
            item,
            decorations: Vec::new(),
        };
        match self.inlay.free.pop() {
            Some(ix) => {
                self.inlay.slots[ix] = Some(slot);
                ix
            }
            None => {
                self.inlay.slots.push(Some(slot));
                self.inlay.slots.len() - 1
            }
        }
    }

    /// Empties `slots` (not freed yet, so no new hint takes them while
    /// their decorations are around), returning their decorations in order.
    fn take_inlay_slots(&mut self, slots: &[usize]) -> Vec<DecorationId> {
        let mut decorations = Vec::new();
        for slot in slots {
            if let Some(slot) = self.inlay.slots[*slot].take() {
                decorations.extend(slot.decorations);
            }
        }
        decorations
    }

    /// Where `items`, new hints for `rows`, go in `order` when nothing
    /// moved since the hints were shown: the range of the hints in the
    /// rows, which they replace, when they all fit in its place (as sorting
    /// them in would put them).
    fn inlay_splice_range(
        &self,
        rows: &Range<u32>,
        items: &[InlayHintItem],
        version: text::Version,
    ) -> Option<Range<usize>> {
        if self.inlay.full_updates || self.inlay.anchored != Some(version) {
            return None;
        }
        let position = |slot: &usize| self.inlay_slot(*slot).item.hint.position;
        let order = &self.inlay.order;
        let start = order.partition_point(|slot| position(slot).row < rows.start);
        let end = start + order[start..].partition_point(|slot| position(slot).row < rows.end);
        let sorted = items
            .windows(2)
            .all(|pair| pair[0].hint.position <= pair[1].hint.position);
        // Old hints sort before new ones at the same position.
        let after = match (start.checked_sub(1), items.first()) {
            (Some(before), Some(first)) => position(&order[before]) <= first.hint.position,
            _ => true,
        };
        let before = match (order.get(end), items.last()) {
            (Some(next), Some(last)) => last.hint.position < position(next),
            _ => true,
        };
        (sorted && after && before).then_some(start..end)
    }

    /// Shows `items`, the hints for `rows` (all rows when `None`). As
    /// Monaco's `_updateHintsDecorators` only replaces the decorations in
    /// the requested ranges, hints shown elsewhere stay, where their
    /// decorations moved to. With `fixed_lengths`, the hints before the
    /// cursor keep their lengths (see [`Self::cursor_line_lengths`]).
    ///
    /// Hints keep their slots, so the hints elsewhere aren't touched; and
    /// when no edit moved the decorations since they were shown, the hints
    /// in the rows are found by position and replaced in place (the work
    /// is the rows', as Monaco's is), else every hint's position is read
    /// from its decorations and the hints sorted again.
    fn set_inlay_hint_items(
        &mut self,
        mut items: Vec<InlayHintItem>,
        rows: Option<Range<u32>>,
        fixed_lengths: bool,
        cx: &mut Context<Self>,
    ) {
        let rows = rows.filter(|_| self.inlay.decorations.is_some());
        let lengths = if fixed_lengths {
            self.cursor_line_lengths(cx)
        } else {
            None
        };
        let version = self.model().read(cx).buffer().version();
        // The decorations of the hints in the rows, when replaced in place.
        let mut replaced = None;
        // The slots of the hints replaced.
        let dropped: Vec<usize>;
        match &rows {
            None => {
                dropped = std::mem::take(&mut self.inlay.order);
                self.take_inlay_slots(&dropped);
                // Already sorted (from a request or the cache).
                items.sort_by_key(|item| item.hint.position);
                self.inlay.order = items
                    .into_iter()
                    .map(|item| self.alloc_inlay_slot(item))
                    .collect();
            }
            Some(rows) => match self.inlay_splice_range(rows, &items, version) {
                Some(range) => {
                    let new: Vec<usize> = items
                        .into_iter()
                        .map(|item| self.alloc_inlay_slot(item))
                        .collect();
                    dropped = self.inlay.order.splice(range, new).collect();
                    replaced = Some(self.take_inlay_slots(&dropped));
                }
                None => {
                    // Where each old hint shows now.
                    let mut at: HashMap<usize, PointUtf16> = HashMap::new();
                    {
                        let model = self.model().read(cx);
                        for (id, slot) in &self.inlay.decoration_items {
                            if let Some(range) = model.decoration_range(*id) {
                                let position = at.entry(*slot).or_insert(range.start);
                                *position = (*position).min(range.start);
                            }
                        }
                    }
                    let old = std::mem::take(&mut self.inlay.order);
                    let mut order = Vec::with_capacity(old.len() + items.len());
                    let mut gone = Vec::new();
                    for slot in old {
                        let item = &mut self.inlay.slots[slot]
                            .as_mut()
                            .expect("inlay hint slot in use")
                            .item;
                        let position = at.get(&slot).copied().unwrap_or(item.hint.position);
                        if rows.contains(&position.row) {
                            gone.push(slot);
                        } else {
                            item.hint.position = position;
                            order.push(slot);
                        }
                    }
                    self.take_inlay_slots(&gone);
                    dropped = gone;
                    order.extend(items.into_iter().map(|item| self.alloc_inlay_slot(item)));
                    order.sort_by_key(|slot| self.inlay_slot(*slot).item.hint.position);
                    self.inlay.order = order;
                }
            },
        }
        let fixed = match lengths {
            Some((cursor, lengths)) => {
                let before_cursor: Vec<usize> = self
                    .inlay
                    .order
                    .iter()
                    .copied()
                    .filter(|slot| {
                        let position = self.inlay_slot(*slot).item.hint.position;
                        position.row == cursor.row && position.column <= cursor.column
                    })
                    .collect();
                match_fixed_lengths(&before_cursor, &lengths)
            }
            None => HashMap::new(),
        };
        self.inlay.generation += 1;
        self.inlay.active = None;
        self.render_inlay_hints_in(&fixed, rows, replaced, cx);
        self.inlay.free.extend(dropped);
    }

    /// Monaco `_updateHintsDecorators`' fixed lengths: shortly after an
    /// edit, the new hints on the cursor's line before the cursor keep the
    /// lengths the old ones there had, so the cursor doesn't jump while
    /// typing. Returns the cursor and the old hints' lengths there, in
    /// order, when they're kept.
    fn cursor_line_lengths(&mut self, cx: &App) -> Option<(PointUtf16, Vec<usize>)> {
        let (position, until) = self.inlay.cursor?;
        let covered = self
            .inlay
            .requested
            .as_ref()
            .is_some_and(|rows| rows.contains(&position.row));
        if until <= Instant::now() || !covered {
            return None;
        }
        self.inlay.cursor = None;
        // The old hints' lengths there, by slot.
        let mut lengths: HashMap<usize, usize> = HashMap::new();
        if let Some(decorations) = &self.inlay.decorations {
            let model = self.model().read(cx);
            let rope = model.buffer().rope();
            for id in decorations.ids() {
                let Some(item) = self.inlay.decoration_items.get(id) else {
                    continue;
                };
                let Some(decoration) = model.decoration(*id) else {
                    continue;
                };
                let at = rope.offset_to_point_utf16(decoration.range.start);
                if at.row != position.row || at.column > position.column {
                    continue;
                }
                if let Some(before) = &decoration.options.before {
                    *lengths.entry(*item).or_default() += before.content.chars().count();
                }
            }
        }
        // In the hints' order.
        let lengths = self
            .inlay
            .order
            .iter()
            .filter_map(|slot| lengths.get(slot).copied())
            .collect();
        Some((position, lengths))
    }

    /// The hints shown, at where their decorations are now (Monaco
    /// `_copyInlayHintsWithCurrentAnchor`).
    fn inlay_hints_with_current_positions(&self, cx: &App) -> Vec<InlayHintItem> {
        let Some(decorations) = &self.inlay.decorations else {
            return Vec::new();
        };
        let model = self.model().read(cx);
        let rope = model.buffer().rope();
        let mut positions: HashMap<usize, PointUtf16> = HashMap::new();
        for id in decorations.ids() {
            if let Some(slot) = self.inlay.decoration_items.get(id)
                && !positions.contains_key(slot)
                && let Some(decoration) = model.decoration(*id)
            {
                positions.insert(*slot, rope.offset_to_point_utf16(decoration.range.start));
            }
        }
        let mut items: Vec<InlayHintItem> = self
            .inlay
            .order
            .iter()
            .filter_map(|slot| {
                let position = positions.get(slot)?;
                let mut item = self.inlay_slot(*slot).item.clone();
                item.hint.position = *position;
                Some(item)
            })
            .collect();
        items.sort_by_key(|item| item.hint.position);
        items
    }

    /// Monaco `_cacheHintsForFastRestore`.
    fn cache_inlay_hints(&self, cx: &mut Context<Self>) {
        let items = self.inlay_hints_with_current_positions(cx);
        if items.is_empty() {
            return;
        }
        let model = self.model().read(cx);
        let (id, version) = (self.model().entity_id(), model.buffer().version());
        cx.default_global::<InlayHintsCache>()
            .set(id, version, items);
    }

    /// Monaco `_update`'s "quickly update from cache": shows the hints
    /// last shown for this model at this version, if any.
    pub(crate) fn restore_cached_inlay_hints(&mut self, cx: &mut Context<Self>) {
        if !self.inlay_hints_active(cx) {
            return;
        }
        let version = self.model().read(cx).buffer().version();
        let id = self.model().entity_id();
        let cached = cx
            .try_global::<InlayHintsCache>()
            .is_some()
            .then(|| cx.global_mut::<InlayHintsCache>().get(id, &version))
            .flatten();
        if let Some(items) = cached {
            self.set_inlay_hint_items(items, None, false, cx);
        }
    }

    /// Monaco `getVisibleRangesPlusViewportAboveBelow`.
    #[allow(clippy::type_complexity)]
    fn request_inlay_hints(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<(
        Arc<dyn InlayHintsProvider>,
        Task<anyhow::Result<Vec<InlayHint>>>,
    )> {
        let last_row = self.snapshot.max_point().row;
        let visible = self
            .layout
            .as_ref()
            .and_then(|layout| Some(layout.lines.first()?.row..layout.lines.last()?.row + 1))
            .unwrap_or(0..100.min(last_row + 1));
        let height = visible.len() as u32;
        let rows = visible.start.saturating_sub(height)..(visible.end + height).min(last_row + 1);
        self.inlay.requested = Some(rows.clone());
        let range = PointUtf16::new(rows.start, 0)
            ..PointUtf16::new(
                rows.end.saturating_sub(1),
                self.snapshot
                    .rope()
                    .line_len_utf16(rows.end.saturating_sub(1)),
            );
        let model = self.model().read(cx);
        let context = model.provider_context();
        // Monaco `InlayHintsFragments.create` asks `ordered(model).reverse()`.
        inlay_hints_providers(cx, self.provider_target(cx))
            .into_iter()
            .rev()
            .map(|provider| {
                let task = provider.provide_inlay_hints(&context, range.clone(), cx);
                (provider, task)
            })
            .collect()
    }

    /// The viewport shows `rows`: request hints if they weren't covered
    /// (Monaco requests on scroll).
    pub(crate) fn inlay_hints_viewport(&mut self, rows: Range<u32>, cx: &mut Context<Self>) {
        let epoch = cx.try_global::<InlayHintsEpoch>().map_or(0, |e| e.0);
        let covered = self
            .inlay
            .requested
            .as_ref()
            .is_some_and(|r| r.start <= rows.start && rows.end <= r.end);
        if epoch != self.inlay.epoch {
            self.inlay.epoch = epoch;
            self.schedule_inlay_hints(Duration::ZERO, cx);
        } else if !covered && self.inlay.task.is_none() && self.inlay_hints_active(cx) {
            self.schedule_inlay_hints(SCROLL_DELAY, cx);
        }
    }

    /// The text changed: hints move with it until the new ones arrive.
    pub(crate) fn inlay_hints_after_edit(&mut self, cx: &mut Context<Self>) {
        if self.inlay_hints_active(cx) {
            // Monaco marks the cursor, and when the hints before it may
            // change length again, asking for them then.
            let delay = EDIT_DELAY.max(CURSOR_FREEZE);
            self.inlay.cursor = Some((self.cursors.primary().position, Instant::now() + delay));
            self.inlay.cursor_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                this.update(cx, |this, cx| this.schedule_inlay_hints(Duration::ZERO, cx))
                    .ok();
            }));
            self.schedule_inlay_hints(EDIT_DELAY, cx);
        }
    }

    /// The model, its language or the options changed (Monaco `_update`):
    /// the model's hints are cached, then shown again from the cache, if
    /// it has this model's, until new ones arrive.
    pub(crate) fn reset_inlay_hints(&mut self, cx: &mut Context<Self>) {
        self.cache_inlay_hints(cx);
        if let Some(mut decorations) = self.inlay.decorations.take() {
            decorations.clear(cx);
        }
        self.inlay.parts.clear();
        self.inlay.decoration_items.clear();
        self.inlay.slots.clear();
        self.inlay.free.clear();
        self.inlay.order.clear();
        self.inlay.anchored = None;
        self.inlay.generation += 1;
        self.inlay.active = None;
        self.inlay.requested = None;
        self.inlay.cursor = None;
        self.inlay.cursor_task = None;
        self.restore_cached_inlay_hints(cx);
        self.schedule_inlay_hints(Duration::ZERO, cx);
    }

    /// Ctrl+Alt toggles hints in the "unless pressed" modes, and the link
    /// modifier turns a hovered part into a link.
    pub(crate) fn inlay_hints_modifiers(&mut self, modifiers: &Modifiers, cx: &mut Context<Self>) {
        if let Some((part, _)) = self.inlay.active {
            let link = self.is_definition_modifier(modifiers);
            self.set_active_inlay_part(Some((part, link)), cx);
        }
        let pressed = modifiers.control && modifiers.alt && !modifiers.shift && !modifiers.platform;
        if pressed != self.inlay.pressed {
            self.inlay.pressed = pressed;
            if matches!(
                self.options.inlay_hints.enabled,
                InlayHintsMode::OnUnlessPressed | InlayHintsMode::OffUnlessPressed
            ) {
                self.render_inlay_hints(cx);
            }
        }
    }

    fn inlay_hints_visible(&self) -> bool {
        match self.options.inlay_hints.enabled {
            InlayHintsMode::On => true,
            InlayHintsMode::OnUnlessPressed => !self.inlay.pressed,
            InlayHintsMode::OffUnlessPressed => self.inlay.pressed,
            InlayHintsMode::Off => false,
        }
    }

    /// Monaco `_getLayoutInfo`: the hints' font size, unless it's the
    /// editor's, and font family.
    fn inlay_hints_font(&self) -> (Option<Pixels>, Option<gpui::SharedString>) {
        let options = &self.options.inlay_hints;
        let editor_size = f32::from(self.options.font_size);
        let size = options.font_size;
        let size = (size >= 5. && size < editor_size).then_some(px(size));
        let family = options
            .font_family
            .clone()
            .filter(|family| !family.is_empty() && *family != self.options.font_family);
        (size, family)
    }

    /// Monaco `_fillInColors`: (foreground, background) for a hint kind.
    fn inlay_hint_colors(&self, kind: Option<InlayHintKind>) -> (gpui::Hsla, gpui::Hsla) {
        let colors = &self.theme.colors;
        match kind {
            Some(InlayHintKind::Type) => (
                colors.inlay_hint_type_foreground,
                colors.inlay_hint_type_background,
            ),
            Some(InlayHintKind::Parameter) => (
                colors.inlay_hint_parameter_foreground,
                colors.inlay_hint_parameter_background,
            ),
            None => (colors.inlay_hint_foreground, colors.inlay_hint_background),
        }
    }

    /// A label part's text style: Monaco underlines the part under the
    /// mouse when it has a location or command, in the link color while the
    /// link modifier is held.
    fn inlay_part_style(&self, part: InlayHintPart) -> TokenStyle {
        let kind = self.inlay_hint_item(part).and_then(|item| item.hint.kind);
        let mut style = TokenStyle {
            foreground: Some(self.inlay_hint_colors(kind).0),
            ..Default::default()
        };
        if let Some((active, modifier)) = self.inlay.active
            && active == part
        {
            style.underline = true;
            if modifier {
                style.foreground = Some(self.theme.colors.link_active_foreground);
            }
        }
        style
    }

    /// Makes `active` the active part, restyling the old and new one in
    /// place (keeping their decorations, which the mouse hit-tests).
    fn set_active_inlay_part(
        &mut self,
        active: Option<(InlayHintPart, bool)>,
        cx: &mut Context<Self>,
    ) {
        let previous = std::mem::replace(&mut self.inlay.active, active);
        if previous == active {
            return;
        }
        let touched = |part: &InlayHintPart| {
            [previous, active]
                .iter()
                .any(|a| a.is_some_and(|(a, _)| a == *part))
        };
        let restyle: Vec<(DecorationId, TokenStyle)> = self
            .inlay
            .parts
            .iter()
            .map(|(id, part)| (*id, self.inlay_part(*part)))
            .filter(|(_, part)| touched(part))
            .map(|(id, part)| (id, self.inlay_part_style(part)))
            .collect();
        self.model().update(cx, |model, cx| {
            for (id, style) in restyle {
                let Some(mut options) = model.decoration(id).map(|d| (*d.options).clone()) else {
                    continue;
                };
                if let Some(before) = &mut options.before {
                    before.style = style;
                }
                model.change_decoration_options(id, options, cx);
            }
        });
        cx.notify();
    }

    /// Shows the hints as injected text the editor owns.
    fn render_inlay_hints(&mut self, cx: &mut Context<Self>) {
        self.render_inlay_hints_in(&HashMap::new(), None, None, cx);
    }

    /// Forgets decoration `id`, which is removed or shown again.
    fn forget_inlay_decoration(&mut self, id: DecorationId) {
        self.inlay.parts.remove(&id);
        if let Some(slot) = self.inlay.decoration_items.remove(&id)
            && let Some(Some(slot)) = self.inlay.slots.get_mut(slot)
            && let Some(ix) = slot.decorations.iter().position(|d| *d == id)
        {
            slot.decorations.remove(ix);
        }
    }

    /// Monaco `_updateHintsDecorators`: one decoration per label part,
    /// boxed as Monaco's CSS boxes them, and the padding of hints that keep
    /// a `fixed` length (by slot), for the hints in `rows` (all when
    /// `None`). Decorations elsewhere stay. `replaced` are the decorations
    /// in the rows, when known (the hints there replaced in place);
    /// otherwise every decoration is looked at.
    ///
    /// Monaco replaces every decoration in the rows. Here a position whose
    /// decorations would come out the same keeps them: the same text shows,
    /// and the view lines don't gather its injected text again (a port-side
    /// optimization, e.g. for the two thirds of the hints a Page Down asks
    /// for again).
    fn render_inlay_hints_in(
        &mut self,
        fixed: &HashMap<usize, usize>,
        rows: Option<Range<u32>>,
        replaced: Option<Vec<DecorationId>>,
        cx: &mut Context<Self>,
    ) {
        if !self.inlay_hints_visible() || self.inlay.order.is_empty() {
            self.inlay.parts.clear();
            self.inlay.decoration_items.clear();
            for slot in self.inlay.slots.iter_mut().flatten() {
                slot.decorations.clear();
            }
            self.inlay.anchored = None;
            if let Some(mut decorations) = self.inlay.decorations.take() {
                decorations.clear(cx);
            }
            cx.notify();
            return;
        }
        let padding = self.options.inlay_hints.padding;
        let (font_size, font_family) = self.inlay_hints_font();
        // Monaco `_getLayoutInfo().fontSize`, whole pixels as in its CSS.
        let size = f32::from(font_size.unwrap_or(self.options.font_size));
        // `addInjectedWhitespace`: an inline-block `(fontSize / 3) | 0` wide.
        let spacer = px((size / 3.).trunc());
        // The `padding` option: `padding: 1px max(1, fontSize / 4)px` and
        // `border-radius: (fontSize / 4)px`, split over the first and last
        // parts.
        let (padding_x, padding_y, radius) = if padding {
            (
                px((size / 4.).max(1.).trunc()),
                px(1.),
                px((size / 4.).trunc()),
            )
        } else {
            (px(0.), px(0.), px(0.))
        };
        // The hints to show (those in the rows: the hints are in order), by
        // slot.
        let shown = match &rows {
            None => 0..self.inlay.order.len(),
            Some(rows) => {
                let position = |slot: &usize| self.inlay_slot(*slot).item.hint.position;
                let order = &self.inlay.order;
                let start = order.partition_point(|slot| position(slot).row < rows.start);
                start..start + order[start..].partition_point(|slot| position(slot).row < rows.end)
            }
        };
        let (indices, hints): (Vec<usize>, Vec<InlayHint>) = self.inlay.order[shown]
            .iter()
            .map(|slot| (*slot, self.inlay_slot(*slot).item.hint.clone()))
            .unzip();
        let mut decorations = Vec::new();
        // The hint (slot), and the part, each decoration shows, by index
        // into `decorations`.
        let mut items = Vec::new();
        let mut parts = Vec::new();
        for (shown_ix, shown) in limit_per_line(&hints, self.options.inlay_hints.maximum_length) {
            let hint = &hints[shown_ix];
            let item = indices[shown_ix];
            let (shown, pad) = match fixed.get(&item) {
                Some(fixed) => fix_length(shown, *fixed),
                None => (shown, 0),
            };
            let (_, background) = self.inlay_hint_colors(hint.kind);
            let position = hint.position..hint.position;
            let margin_left = if hint.padding_left { spacer } else { px(0.) };
            let margin_right = if hint.padding_right { spacer } else { px(0.) };
            let count = shown.len();
            for (ix, (part_ix, text)) in shown.into_iter().enumerate() {
                let part = InlayHintPart {
                    generation: self.inlay.generation,
                    item,
                    part: part_ix,
                };
                let (first, last) = (ix == 0, ix + 1 == count);
                let edge = |at_edge: bool, value: Pixels| if at_edge { value } else { px(0.) };
                items.push(item);
                parts.push(Some(PartRef {
                    item,
                    part: part_ix,
                }));
                decorations.push((
                    position.clone(),
                    DecorationOptions {
                        before: Some(InjectedText {
                            content: fix_space(&text).into(),
                            style: self.inlay_part_style(part),
                            background: Some(background),
                            swatch: None,
                            font_family: font_family.clone(),
                            font_size,
                            text_box: Some(InjectedTextBox {
                                margin: (
                                    edge(first, margin_left),
                                    edge(last && pad == 0, margin_right),
                                ),
                                padding: (edge(first, padding_x), edge(last, padding_x)),
                                padding_y,
                                corner_radius: (edge(first, radius), edge(last, radius)),
                            }),
                            // Monaco: only the end of the hint (its last
                            // part, or the padding after it) is a stop.
                            cursor_stops: if last && pad == 0 {
                                InjectedTextCursorStops::Right
                            } else {
                                InjectedTextCursorStops::None
                            },
                            ..Default::default()
                        }),
                        // Monaco `showIfCollapsed: range.isEmpty()`.
                        show_if_collapsed: true,
                        ..Default::default()
                    },
                ));
            }
            if pad > 0 {
                // Monaco pads a hint shorter than its fixed length with hair
                // spaces, without a background.
                items.push(item);
                parts.push(None);
                decorations.push((
                    position.clone(),
                    DecorationOptions {
                        before: Some(InjectedText {
                            content: "\u{200a}".repeat(pad).into(),
                            font_family: font_family.clone(),
                            font_size,
                            text_box: Some(InjectedTextBox {
                                margin: (
                                    if count == 0 { margin_left } else { px(0.) },
                                    margin_right,
                                ),
                                ..Default::default()
                            }),
                            // Monaco's padding is `None`; the `paddingRight`
                            // spacer after it is `Right`.
                            cursor_stops: if hint.padding_right {
                                InjectedTextCursorStops::Right
                            } else {
                                InjectedTextCursorStops::None
                            },
                            ..Default::default()
                        }),
                        show_if_collapsed: true,
                        ..Default::default()
                    },
                ));
            }
        }
        let model = self.model().clone();
        let mut collection = self
            .inlay
            .decorations
            .take()
            .unwrap_or_else(|| DecorationsCollection::for_owner(model.clone(), cx.entity_id()));
        // Whether every decoration ends up at its hint's position.
        let mut anchored = true;
        // The current decorations at each position in the rows, in order,
        // and those elsewhere, which stay.
        let mut current: BTreeMap<PointUtf16, Vec<(DecorationId, Arc<DecorationOptions>)>> =
            BTreeMap::new();
        let mut remove = Vec::new();
        {
            let model = model.read(cx);
            match replaced {
                Some(replaced) => {
                    for id in replaced {
                        self.forget_inlay_decoration(id);
                        match model.decoration_range(id).zip(model.decoration_options(id)) {
                            Some((range, options)) => {
                                current.entry(range.start).or_default().push((id, options));
                            }
                            None => remove.push(id),
                        }
                    }
                }
                None => {
                    if rows.is_none() {
                        for slot in self.inlay.slots.iter_mut().flatten() {
                            slot.decorations.clear();
                        }
                    }
                    for id in collection.ids() {
                        let found = model
                            .decoration_range(*id)
                            .zip(model.decoration_options(*id));
                        let Some((range, options)) = found else {
                            self.forget_inlay_decoration(*id);
                            remove.push(*id);
                            continue;
                        };
                        let inside = rows
                            .as_ref()
                            .is_none_or(|rows| rows.contains(&range.start.row));
                        if inside {
                            self.forget_inlay_decoration(*id);
                            current.entry(range.start).or_default().push((*id, options));
                            continue;
                        }
                        // A hint elsewhere stays, if it's still there.
                        let kept = self
                            .inlay
                            .decoration_items
                            .get(id)
                            .and_then(|slot| self.inlay.slots.get(*slot)?.as_ref());
                        match kept {
                            Some(slot) => anchored &= slot.item.hint.position == range.start,
                            None => {
                                self.forget_inlay_decoration(*id);
                                remove.push(*id);
                            }
                        }
                    }
                }
            }
        }
        // Each position's new decorations, in order; a position whose
        // current ones are the same keeps them.
        let mut add = Vec::new();
        let mut added = Vec::new();
        let mut ix = 0;
        while ix < decorations.len() {
            let position = decorations[ix].0.start;
            let end = ix + decorations[ix..].partition_point(|(range, _)| range.start == position);
            let same = current.remove(&position).and_then(|old| {
                let same = old.len() == end - ix
                    && old
                        .iter()
                        .zip(&decorations[ix..end])
                        .all(|((_, old), (_, new))| **old == *new);
                if same {
                    Some(old)
                } else {
                    remove.extend(old.iter().map(|(id, _)| *id));
                    None
                }
            });
            for (offset, decoration) in decorations[ix..end].iter().enumerate() {
                let new_ix = ix + offset;
                match &same {
                    Some(old) => {
                        let id = old[offset].0;
                        self.note_inlay_decoration(id, items[new_ix], parts[new_ix]);
                    }
                    None => {
                        add.push(decoration.clone());
                        added.push(new_ix);
                    }
                }
            }
            ix = end;
        }
        remove.extend(current.into_values().flatten().map(|(id, _)| id));
        let ids = collection.replace(&remove, add, cx);
        {
            let model = model.read(cx);
            for (id, new_ix) in ids.iter().zip(&added) {
                anchored &= model.decoration_range(*id).map(|range| range.start)
                    == Some(decorations[*new_ix].0.start);
            }
        }
        for (id, new_ix) in ids.into_iter().zip(added) {
            self.note_inlay_decoration(id, items[new_ix], parts[new_ix]);
        }
        self.inlay.anchored = anchored.then(|| model.read(cx).buffer().version());
        self.inlay.decorations = Some(collection);
        cx.notify();
    }

    /// Decoration `id` shows hint `slot` (and label part `part`).
    fn note_inlay_decoration(&mut self, id: DecorationId, slot: usize, part: Option<PartRef>) {
        self.inlay.decoration_items.insert(id, slot);
        if let Some(part) = part {
            self.inlay.parts.insert(id, part);
        }
        if let Some(Some(slot)) = self.inlay.slots.get_mut(slot) {
            slot.decorations.push(id);
        }
    }

    /// The label part at `point` (window coordinates), if the mouse is over
    /// one (Monaco `_getInlayHintLabelPart`).
    pub(crate) fn inlay_hint_part_at(&self, point: Point<Pixels>) -> Option<InlayHintPart> {
        if self.inlay.parts.is_empty() {
            return None;
        }
        let layout = self.layout.as_ref()?;
        if !layout.text_bounds.contains(&point) {
            return None;
        }
        let line = layout.lines.iter().find(|line| {
            let top = layout.line_top(line.view_line);
            top <= point.y && point.y < top + layout.line_height_of(line.view_line)
        })?;
        let x = point.x - layout.text_origin_x() - line.indent_x;
        // The spacers beside a hint aren't part of it (Monaco's
        // `_whitespaceData`).
        line.injected
            .iter()
            .find(|injected| injected.background_range(&line.shaped).contains(&x))
            .and_then(|injected| self.inlay.parts.get(&injected.decoration?))
            .map(|part| self.inlay_part(*part))
    }

    /// Where a shown label part is drawn, in window coordinates: its box
    /// (Monaco's part `span`).
    fn inlay_part_bounds(&self, part: InlayHintPart) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        for line in &layout.lines {
            for injected in &line.injected {
                let Some(id) = injected.decoration else {
                    continue;
                };
                if self.inlay.parts.get(&id).map(|p| self.inlay_part(*p)) != Some(part) {
                    continue;
                }
                let x = injected.background_range(&line.shaped);
                let height = line.shaped.text_height(injected.range.start)
                    + injected.text_box.map_or(px(0.), |b| b.padding_y * 2.);
                let top =
                    layout.text_top(line.view_line) + (layout.metrics.line_height - height) / 2.;
                let left = layout.text_origin_x() + line.indent_x + x.start;
                return Some(Bounds::new(
                    gpui::point(left, top),
                    gpui::size(x.end - x.start, height),
                ));
            }
        }
        None
    }

    fn inlay_hint_item(&self, part: InlayHintPart) -> Option<&InlayHintItem> {
        (part.generation == self.inlay.generation)
            .then(|| Some(&self.inlay.slots.get(part.item)?.as_ref()?.item))
            .flatten()
    }

    fn inlay_hint_item_mut(&mut self, part: InlayHintPart) -> Option<&mut InlayHintItem> {
        (part.generation == self.inlay.generation)
            .then(|| Some(&mut self.inlay.slots.get_mut(part.item)?.as_mut()?.item))
            .flatten()
    }

    /// A rendered decoration's label part, of the hints shown now.
    fn inlay_part(&self, part: PartRef) -> InlayHintPart {
        InlayHintPart {
            generation: self.inlay.generation,
            item: part.item,
            part: part.part,
        }
    }

    /// Resolves `part`'s hint once (Monaco `InlayHintItem.resolve`), and
    /// returns it.
    fn resolve_inlay_hint(
        &mut self,
        part: InlayHintPart,
        cx: &mut Context<Self>,
    ) -> Task<Option<InlayHint>> {
        let Some(item) = self.inlay_hint_item(part).cloned() else {
            return Task::ready(None);
        };
        if item.resolved {
            return Task::ready(Some(item.hint));
        }
        // Monaco waits for a resolve that's under way.
        if let Some(pending) = item.pending {
            return cx.spawn(async move |_, _| pending.await);
        }
        let Some(task) = item.provider.resolve_inlay_hint(&item.hint, cx) else {
            return Task::ready(Some(item.hint));
        };
        let pending = cx
            .spawn(async move |this, cx| {
                let result = task.await;
                let resolved = result.is_ok();
                let mut hint = item.hint.clone();
                match result {
                    Ok(resolved) => {
                        hint.label = resolved.label;
                        if resolved.tooltip.is_some() {
                            hint.tooltip = resolved.tooltip;
                        }
                        if !resolved.text_edits.is_empty() {
                            hint.text_edits = resolved.text_edits;
                        }
                    }
                    Err(err) => log::warn!("resolving an inlay hint failed: {err:#}"),
                }
                this.update(cx, |this, cx| {
                    if let Some(item) = this.inlay_hint_item_mut(part) {
                        let changed = item.hint.label != hint.label;
                        item.hint = hint.clone();
                        item.resolved = resolved;
                        item.pending = None;
                        if changed {
                            this.render_inlay_hints(cx);
                        }
                    }
                })
                .ok();
                Some(hint)
            })
            .shared();
        if let Some(item) = self.inlay_hint_item_mut(part) {
            item.pending = Some(pending.clone());
        }
        cx.spawn(async move |_, _| pending.await)
    }

    /// The mouse moved over `part` (or off every part), with or without
    /// the link modifier: underlines a part that has a location or command
    /// (Monaco's link gesture `onMouseMoveOrRelevantKeyDown`).
    pub(crate) fn inlay_hints_mouse_move(
        &mut self,
        part: Option<InlayHintPart>,
        modifier: bool,
        cx: &mut Context<Self>,
    ) {
        let active = part
            .filter(|part| {
                self.inlay_hint_item(*part)
                    .and_then(|item| item.hint.label.parts().get(part.part).cloned())
                    .is_some_and(|p| p.location.is_some() || p.command.is_some())
            })
            .map(|part| (part, modifier));
        self.set_active_inlay_part(active, cx);
        if let Some(part) = part {
            // Monaco resolves the item as soon as the mouse is over it.
            self.resolve_inlay_hint(part, cx).detach();
        }
    }

    /// Whether Cmd/Ctrl is held over a clickable part (for the pointer
    /// cursor).
    pub(crate) fn has_active_inlay_hint_link(&self) -> bool {
        self.inlay.active.is_some_and(|(_, modifier)| modifier)
    }

    /// Monaco's link gesture `onCancel`: the hovered label part's styling
    /// goes.
    pub(crate) fn cancel_inlay_hint_link(&mut self, cx: &mut Context<Self>) {
        if self.inlay.active.is_some() {
            self.set_active_inlay_part(None, cx);
        }
    }

    /// Cmd/Ctrl+click on `part` (Monaco's link gesture `onExecute`): opens
    /// its location like go to definition, else runs its command. Returns
    /// whether it had either.
    pub(crate) fn inlay_hint_link_click(
        &mut self,
        part: InlayHintPart,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(label_part) = self
            .inlay_hint_item(part)
            .and_then(|item| item.hint.label.parts().get(part.part).cloned())
        else {
            return false;
        };
        if let Some(location) = label_part.location {
            self.set_active_inlay_part(None, cx);
            self.open_location(&location, true, cx);
            true
        } else if let Some(command) = label_part.command {
            self.run_inlay_hint_command(&command, cx);
            true
        } else {
            false
        }
    }

    /// Runs a label part's command: through the command registry, or
    /// [`CodeEditorEvent::ExecuteCommand`] for the host.
    pub(crate) fn run_inlay_hint_command(&mut self, command: &Command, cx: &mut Context<Self>) {
        if crate::commands::execute_command(cx, command).is_err() {
            cx.emit(CodeEditorEvent::ExecuteCommand(command.clone()));
        }
    }

    /// A double-click on `part` (Monaco `_installDblClickGesture`):
    /// resolves its hint and applies the hint's text edits, then asks for
    /// hints again.
    pub(crate) fn inlay_hint_double_click(&mut self, part: InlayHintPart, cx: &mut Context<Self>) {
        let resolve = self.resolve_inlay_hint(part, cx);
        cx.spawn(async move |this, cx| {
            let Some(hint) = resolve.await else {
                return;
            };
            if hint.text_edits.is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                this.execute_edits_from("inlayHint.default", hint.text_edits, None, cx);
                this.schedule_inlay_hints(Duration::ZERO, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Monaco `InlayHintsHover.computeAsync`: resolves `part`'s hint, then
    /// returns where it is, its hover contents and the part's command.
    pub(crate) fn inlay_hint_hover_contents(
        &mut self,
        part: InlayHintPart,
        cx: &mut Context<Self>,
    ) -> Task<Option<InlayHintHover>> {
        let resolve = self.resolve_inlay_hint(part, cx);
        let keys = match (
            self.options.multi_cursor_modifier,
            cfg!(target_os = "macos"),
        ) {
            (MultiCursorModifier::Alt, true) => "cmd + click",
            (MultiCursorModifier::Alt, false) => "ctrl + click",
            (MultiCursorModifier::CtrlCmd, true) => "option + click",
            (MultiCursorModifier::CtrlCmd, false) => "alt + click",
        };
        cx.spawn(async move |this, cx| {
            let hint = resolve.await?;
            let label_part = hint.label.parts().get(part.part).cloned()?;
            let mut contents = Vec::new();
            // The hint's tooltip, then the part's (Monaco's ordinals 0, 1).
            // String tooltips are plain text (Monaco `appendText`).
            contents.extend(hint.tooltip.as_ref().map(Documentation::to_markdown));
            contents.extend(label_part.tooltip.as_ref().map(Documentation::to_markdown));
            // Then the hovers at the part's location (ordinals 2 and up:
            // `_resolveInlayHintLabelPartHover`).
            if let Some(location) = &label_part.location {
                let tasks = this
                    .update(cx, |this, cx| this.inlay_hint_location_hovers(location, cx))
                    .unwrap_or_default();
                for task in tasks {
                    match task.await {
                        Ok(Some(hover)) => contents.extend(
                            hover
                                .contents
                                .into_iter()
                                .filter(|content| !content.value.trim().is_empty()),
                        ),
                        Ok(None) => {}
                        Err(err) => log::warn!("hover failed: {err:#}"),
                    }
                }
            }
            let mut command = None;
            match (&label_part.location, label_part.command) {
                (Some(_), Some(_)) => {
                    contents.push(format!("Go to Definition ({keys}), right click for more").into())
                }
                (Some(_), None) => contents.push(format!("Go to Definition ({keys})").into()),
                (None, Some(part_command)) => {
                    command = Some((part_command, format!(" ({keys})")));
                }
                (None, None) => {}
            }
            if !hint.text_edits.is_empty() {
                contents.push("Double-click to insert".into());
            }
            Some(InlayHintHover {
                position: hint.position,
                contents,
                command,
            })
        })
    }

    /// Asks the hover providers of `location`'s model about its start
    /// (Monaco `getHoverProviderResultsAsAsyncIterable` on the model
    /// `ITextModelService` resolves). Nothing when no open model has it.
    #[allow(clippy::type_complexity)]
    fn inlay_hint_location_hovers(
        &mut self,
        location: &Location,
        cx: &mut Context<Self>,
    ) -> Vec<Task<anyhow::Result<Option<crate::providers::Hover>>>> {
        let Some(model) = self.model_for_location(location, cx) else {
            return Vec::new();
        };
        let model = model.read(cx);
        let context = model.provider_context();
        let target: crate::providers::ProviderTarget = model.into();
        hover_providers(cx, target)
            .into_iter()
            .map(|provider| provider.provide_hover(&context, location.range.start, cx))
            .collect()
    }

    /// A right-click on an inlay hint label part (Monaco
    /// `_installContextMenu`): once its hint is resolved, a part with a
    /// location shows Monaco's `showGoToContextMenu` below it, in place of
    /// the editor's context menu.
    pub(crate) fn inlay_hint_context_menu(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(part) = self.inlay_hint_part_at(point) else {
            return;
        };
        // Monaco anchors it 8px below the part.
        let anchor = self.inlay_part_bounds(part).map_or(point, |bounds| {
            bounds.bottom_left() + gpui::point(px(0.), px(8.))
        });
        let resolve = self.resolve_inlay_hint(part, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(hint) = resolve.await else {
                return;
            };
            let Some(label_part) = hint.label.parts().get(part.part).cloned() else {
                return;
            };
            let Some(location) = label_part.location else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.show_inlay_hint_location_menu(location, label_part.command, anchor, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Runs the navigation command `kind` from `location` (an item of the
    /// inlay hint location menu: Monaco runs the `SymbolNavigationAction`
    /// with a `SymbolNavigationAnchor` at the location's start).
    pub(crate) fn run_inlay_hint_location(
        &mut self,
        kind: LocationKind,
        location: &Location,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.model_for_location(location, cx) else {
            log::warn!("no model for {:?}", location.uri);
            return;
        };
        let foreign = model != *self.model();
        self.navigate_from(
            kind,
            &model,
            location.range.start,
            crate::goto::Navigation {
                // The no-result message would be about another model's
                // word.
                mute_message: foreign,
                allow_alternative: false,
                ..Default::default()
            },
            cx,
        );
    }

    /// The hints, in order, as (position, label), and whether new hints
    /// for some rows would replace just those rows' hints in place, for
    /// tests.
    #[doc(hidden)]
    pub fn inlay_hint_items_for_tests(&self) -> (Vec<(PointUtf16, String)>, bool) {
        let items = self
            .inlay
            .order
            .iter()
            .map(|slot| {
                let hint = &self.inlay_slot(*slot).item.hint;
                let label = hint
                    .label
                    .parts()
                    .iter()
                    .map(|p| p.label.to_string())
                    .collect();
                (hint.position, label)
            })
            .collect();
        let anchored = self.inlay.anchored.is_some() && !self.inlay.full_updates;
        (items, anchored)
    }

    /// Tests: whether new hints for some rows always find the hints they
    /// replace by reading every decoration (as when an edit moved them).
    #[doc(hidden)]
    pub fn set_inlay_hints_full_updates_for_tests(&mut self, full: bool) {
        self.inlay.full_updates = full;
    }

    /// The inlay hints shown, as (position, text).
    pub fn visible_inlay_hints(&self, cx: &App) -> Vec<(PointUtf16, String)> {
        let Some(decorations) = &self.inlay.decorations else {
            return Vec::new();
        };
        let model = self.model().read(cx);
        let mut hints: Vec<(PointUtf16, String)> = Vec::new();
        let mut last_item = None;
        for id in decorations.ids() {
            let Some(&item) = self.inlay.decoration_items.get(id) else {
                continue;
            };
            let Some(decoration) = model.decoration(*id) else {
                continue;
            };
            let Some(before) = decoration.options.before.as_ref() else {
                continue;
            };
            let text = before.content.replace('\u{a0}', " ");
            match hints.last_mut() {
                // The parts of one hint show as one.
                Some((_, label)) if last_item == Some(item) => label.push_str(&text),
                _ => {
                    let position = model
                        .buffer()
                        .rope()
                        .offset_to_point_utf16(decoration.range.start);
                    hints.push((position, text));
                }
            }
            last_item = Some(item);
        }
        hints
    }

    /// The window point over the middle of the shown label part `text`, for
    /// tests.
    #[doc(hidden)]
    pub fn inlay_hint_part_point_for_tests(&self, text: &str, cx: &App) -> Option<Point<Pixels>> {
        let layout = self.layout.as_ref()?;
        let model = self.model().read(cx);
        let decorations = self.inlay.decorations.as_ref()?;
        let id = decorations.ids().iter().copied().find(|id| {
            self.inlay.parts.contains_key(id)
                && model
                    .decoration(*id)
                    .and_then(|d| d.options.before.clone())
                    .is_some_and(|before| before.content.replace('\u{a0}', " ") == text)
        })?;
        let bounds = self.inlay_part_bounds(self.inlay_part(*self.inlay.parts.get(&id)?))?;
        let _ = layout;
        Some(bounds.center())
    }

    /// The window bounds of the shown label part `text`'s box, for tests.
    #[doc(hidden)]
    pub fn inlay_hint_part_bounds_for_tests(&self, text: &str, cx: &App) -> Option<Bounds<Pixels>> {
        let center = self.inlay_hint_part_point_for_tests(text, cx)?;
        self.inlay_part_bounds(self.inlay_hint_part_at(center)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::InlayHintLabelPart;

    fn hint(row: u32, column: u32, label: &str) -> InlayHint {
        InlayHint {
            position: PointUtf16::new(row, column),
            label: label.into(),
            ..Default::default()
        }
    }

    fn texts(hints: &[InlayHint], maximum: usize) -> Vec<Vec<String>> {
        limit_per_line(hints, maximum)
            .into_iter()
            .map(|(_, parts)| parts.into_iter().map(|(_, text)| text).collect())
            .collect()
    }

    #[test]
    fn long_lines_are_cut() {
        let hints = [
            hint(0, 1, "abcdef"),
            hint(0, 5, "ghijkl"),
            hint(0, 9, "x"),
            hint(1, 0, "abcdefgh"),
        ];
        assert_eq!(
            texts(&hints, 8),
            [vec!["abcdef"], vec!["gh…"], vec!["abcdefgh"]]
        );
        // No limit.
        assert_eq!(texts(&hints, 0).len(), 4);
    }

    fn parts(texts: &[&str]) -> Vec<(usize, String)> {
        texts
            .iter()
            .enumerate()
            .map(|(ix, text)| (ix, text.to_string()))
            .collect()
    }

    /// Monaco's fixed lengths: longer labels are cut to the length (the
    /// part reaching it ends in `…`, even at exactly the length), shorter
    /// ones padded.
    #[test]
    fn fixed_lengths_cut_or_pad() {
        let texts = |(parts, pad): (Vec<(usize, String)>, usize)| {
            (
                parts.into_iter().map(|(_, text)| text).collect::<Vec<_>>(),
                pad,
            )
        };
        assert_eq!(
            texts(fix_length(parts(&[": ", "Foo"]), 4)),
            (vec![": ".to_string(), "F…".to_string()], 0)
        );
        assert_eq!(
            texts(fix_length(parts(&[": ", "Foo"]), 5)),
            (vec![": ".to_string(), "Fo…".to_string()], 0)
        );
        assert_eq!(
            texts(fix_length(parts(&[": ", "Foo"]), 8)),
            (vec![": ".to_string(), "Foo".to_string()], 3)
        );
        // Cut within the first part: the rest is dropped.
        assert_eq!(
            texts(fix_length(parts(&["abcd", "ef"]), 2)),
            (vec!["a…".to_string()], 0)
        );
        // A new hint with no old length shows as `…`.
        assert_eq!(
            texts(fix_length(parts(&["ab"]), 0)),
            (vec!["…".to_string()], 0)
        );
    }

    #[test]
    fn fixed_lengths_match_in_order() {
        let fixed = match_fixed_lengths(&[3, 5], &[4]);
        assert_eq!(fixed.get(&3), Some(&4));
        assert_eq!(fixed.get(&5), Some(&0));
        // Lengths left over are dropped.
        assert_eq!(match_fixed_lengths(&[1], &[2, 3]).len(), 1);
    }

    #[test]
    fn long_label_parts_are_cut() {
        // Monaco cuts the part that crosses the limit and drops the rest.
        let hints = [InlayHint {
            label: vec![
                InlayHintLabelPart {
                    label: "abc".into(),
                    ..Default::default()
                },
                InlayHintLabelPart {
                    label: "defg".into(),
                    ..Default::default()
                },
                InlayHintLabelPart {
                    label: "h".into(),
                    ..Default::default()
                },
            ]
            .into(),
            ..Default::default()
        }];
        assert_eq!(texts(&hints, 5), [vec!["abc", "de…"]]);
        assert_eq!(texts(&hints, 8), [vec!["abc", "defg", "h"]]);
    }
}
