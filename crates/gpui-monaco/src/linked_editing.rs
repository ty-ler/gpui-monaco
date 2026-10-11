//! Linked editing (Monaco's `contrib/linkedEditing`, formerly
//! `renameOnType`): ranges that change together, such as an HTML
//! element's start and end tag names. While the cursor is in one, typing
//! there is mirrored to the others in the same undo step.

use std::ops::Range;
use std::time::Duration;

use web_time::Instant;

use gpui::{App, Context, Task};
use text::PointUtf16;

use crate::cursor::EditOperationType;
use crate::decorations::{DecorationId, DecorationOptions, TrackedRangeStickiness};
use crate::editor::CodeEditor;
use crate::events::ContentChange;
use crate::providers::{LinkedEditingRanges, linked_editing_range_providers};

/// Monaco's debounce bounds for linked editing (`{ max: 200 }`, its minimum
/// defaulting to 50).
const DELAY: (Duration, Duration) = (Duration::from_millis(50), Duration::from_millis(200));

#[derive(Default)]
pub(crate) struct LinkedEditingState {
    /// The ranges' decorations; the first is the reference range.
    decorations: Vec<DecorationId>,
    /// What the reference range may contain.
    word_pattern: Option<syntax::Pattern>,
    /// The position and version the last request was for.
    requested: Option<(PointUtf16, text::Version)>,
    request: Option<Task<()>>,
    scheduled: Option<Task<()>>,
    /// Set while mirroring, so those edits don't trigger anything.
    syncing: bool,
    /// The average request time (Monaco's adaptive debounce).
    samples: Vec<Duration>,
}

impl CodeEditor {
    fn linked_editing_enabled(&self, cx: &App) -> bool {
        if !self.options.linked_editing {
            return false;
        }
        let model = self.model().read(cx);
        !linked_editing_range_providers(cx, self.provider_target(cx)).is_empty()
            || model.syntax_snapshot().is_some()
    }

    /// Whether linked ranges are shown (Monaco `LinkedEditingInputVisible`).
    pub fn linked_editing_active(&self) -> bool {
        !self.linked.decorations.is_empty()
    }

    /// The linked ranges, the reference first (for tests).
    #[doc(hidden)]
    pub fn linked_editing_ranges_for_tests(&self, cx: &App) -> Vec<Range<PointUtf16>> {
        self.linked
            .decorations
            .iter()
            .filter_map(|id| self.decoration_point_range(*id, cx))
            .collect()
    }

    pub(crate) fn decoration_point_range(
        &self,
        id: DecorationId,
        cx: &App,
    ) -> Option<Range<PointUtf16>> {
        let model = self.model().read(cx);
        let range = model.decoration(id)?.range.clone();
        let rope = model.buffer().rope();
        Some(rope.offset_to_point_utf16(range.start)..rope.offset_to_point_utf16(range.end))
    }

    /// Ends linked editing (Monaco `cancelLinkedEditingInput`, Escape).
    pub fn clear_linked_editing(&mut self, cx: &mut Context<Self>) {
        self.linked.request = None;
        self.linked.requested = None;
        let old = std::mem::take(&mut self.linked.decorations);
        if !old.is_empty() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &old, Vec::new(), cx)
            });
        }
    }

    fn linked_delay(&self) -> Duration {
        if self.linked.samples.is_empty() {
            return DELAY.0.mul_f32(1.5).min(DELAY.1);
        }
        let average =
            self.linked.samples.iter().sum::<Duration>() / self.linked.samples.len() as u32;
        average.clamp(DELAY.0, DELAY.1)
    }

    /// Asks again after a pause (Monaco's `rangeUpdateScheduler`).
    fn schedule_linked_editing(&mut self, cx: &mut Context<Self>) {
        let delay = self.linked_delay();
        self.linked.scheduled = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                this.linked.scheduled = None;
                this.update_linked_editing(false, cx);
            })
            .ok();
        }));
    }

    /// The cursor moved.
    pub(crate) fn linked_editing_cursor_changed(&mut self, cx: &mut Context<Self>) {
        if !self.linked.syncing && self.linked_editing_enabled(cx) {
            self.schedule_linked_editing(cx);
        }
    }

    /// The text changed: edits inside the reference range are mirrored,
    /// others ask for the ranges again.
    pub(crate) fn linked_editing_after_edit(
        &mut self,
        changes: &[ContentChange],
        own: bool,
        cx: &mut Context<Self>,
    ) {
        if self.linked.syncing {
            return;
        }
        if !self.linked_editing_enabled(cx) {
            if self.linked_editing_active() {
                self.clear_linked_editing(cx);
            }
            return;
        }
        if own
            && let Some(reference) = self
                .linked
                .decorations
                .first()
                .and_then(|id| self.decoration_point_range(*id, cx))
        {
            // Changes are in pre-edit coordinates; the reference range grew
            // to cover typing at its edges, so compare against its start.
            let inside = changes.iter().all(|change| {
                change.range.start >= PointUtf16::new(reference.start.row, 0)
                    && change.range.start.row == reference.start.row
                    && change.range.end.row == reference.start.row
                    && change.range.start <= reference.end
            });
            if inside {
                // Monaco mirrors on the next tick (a 0ms delayer).
                let this = cx.entity().downgrade();
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| this.sync_linked_ranges(cx)).ok();
                });
                return;
            }
        }
        self.schedule_linked_editing(cx);
    }

    /// Monaco `_syncRanges`: copies the reference range's text to the
    /// others, editing only what differs.
    fn sync_linked_ranges(&mut self, cx: &mut Context<Self>) {
        let ranges: Vec<Range<PointUtf16>> = self
            .linked
            .decorations
            .iter()
            .filter_map(|id| self.decoration_point_range(*id, cx))
            .collect();
        let Some(reference) = ranges.first() else {
            return;
        };
        if reference.start.row != reference.end.row {
            self.clear_linked_editing(cx);
            return;
        }
        let rope = self.model().read(cx).buffer().rope().clone();
        let text_of = |range: &Range<PointUtf16>| {
            rope.text_for_range(
                rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end),
            )
        };
        let value = text_of(reference);
        // Text the word pattern doesn't cover (e.g. a space) ends it.
        if let Some(regex) = self
            .linked
            .word_pattern
            .as_ref()
            .and_then(syntax::Pattern::regex)
        {
            let length = regex
                .find(&value)
                .ok()
                .flatten()
                .filter(|m| m.start() == 0)
                .map_or(0, |m| m.end());
            if length != value.len() {
                self.clear_linked_editing(cx);
                return;
            }
        }
        let mut edits = Vec::new();
        for mirror in &ranges[1..] {
            if mirror.start.row != mirror.end.row {
                edits.push((mirror.clone(), value.clone()));
                continue;
            }
            let old: Vec<u16> = text_of(mirror).encode_utf16().collect();
            let new: Vec<u16> = value.encode_utf16().collect();
            let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(new[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let start = mirror.start.column + prefix as u32;
            let end = mirror.end.column - suffix as u32;
            let text = String::from_utf16_lossy(&new[prefix..new.len() - suffix]);
            if start != end || !text.is_empty() {
                edits.push((
                    PointUtf16::new(mirror.start.row, start)..PointUtf16::new(mirror.end.row, end),
                    text,
                ));
            }
        }
        if edits.is_empty() {
            return;
        }
        // Part of the same undo step as the typing (Monaco pops the undo
        // stop and keeps the previous edit type).
        self.linked.syncing = true;
        let kind = self.prev_edit_kind.unwrap_or(EditOperationType::Other);
        edits.sort_by_key(|(range, _)| range.start);
        self.apply_edits_moving_cursors(&edits, kind, cx);
        self.prev_edit_kind = Some(kind);
        self.linked.syncing = false;
    }

    /// Asks for the ranges at the cursor (Monaco `updateRanges`); `force`
    /// works even with the option off (`editor.action.linkedEditing`).
    pub fn update_linked_editing(&mut self, force: bool, cx: &mut Context<Self>) {
        if (!self.linked_editing_enabled(cx) && !force) || self.cursors.selections().len() > 1 {
            self.clear_linked_editing(cx);
            return;
        }
        let position = self.cursors.primary().position;
        let model = self.model().read(cx);
        let version = model.buffer().version();
        if let Some((requested, requested_version)) = &self.linked.requested
            && *requested_version == version
        {
            if *requested == position {
                return;
            }
            // Moving within the reference range keeps it.
            if self
                .linked
                .decorations
                .first()
                .and_then(|id| self.decoration_point_range(*id, cx))
                .is_some_and(|r| r.start <= position && position <= r.end)
            {
                return;
            }
        }
        let inside_current = self
            .linked
            .decorations
            .first()
            .and_then(|id| self.decoration_point_range(*id, cx))
            .is_some_and(|r| r.start <= position && position <= r.end);
        if !inside_current {
            self.clear_linked_editing(cx);
        }
        let model = self.model().read(cx);
        self.linked.requested = Some((position, version));
        let language_pattern = model
            .language()
            .and_then(|language| language.config.word_pattern.clone());
        let context = model.provider_context();
        let providers = linked_editing_range_providers(cx, self.provider_target(cx));
        // Without providers: HTML tag names from the syntax tree.
        let built_in = providers.is_empty().then(|| {
            let offset = model.buffer().point_utf16_to_offset(position);
            let rope = model.buffer().rope().clone();
            model.syntax_snapshot().and_then(|syntax| {
                (syntax.text.version() == version)
                    .then(|| syntax.highlighter.linked_tag_ranges(&syntax.state, offset))
                    .flatten()
                    .map(|ranges| LinkedEditingRanges {
                        ranges: ranges
                            .into_iter()
                            .map(|r| {
                                rope.offset_to_point_utf16(r.start)
                                    ..rope.offset_to_point_utf16(r.end)
                            })
                            .collect(),
                        word_pattern: None,
                    })
            })
        });
        let requests: Vec<_> = providers
            .iter()
            .map(|provider| provider.provide_linked_editing_ranges(&context, position, cx))
            .collect();
        let started = Instant::now();
        self.linked.request = Some(cx.spawn(async move |this, cx| {
            // Monaco: the first provider (by score) with ranges.
            let mut response = built_in.flatten();
            for request in requests {
                match request.await {
                    Ok(Some(ranges)) if !ranges.ranges.is_empty() => {
                        response = Some(ranges);
                        break;
                    }
                    Ok(_) => {}
                    Err(err) => log::warn!("linked editing failed: {err:#}"),
                }
            }
            this.update(cx, |this, cx| {
                this.linked.request = None;
                if this.linked.samples.len() == 6 {
                    this.linked.samples.remove(0);
                }
                this.linked.samples.push(started.elapsed());
                if this.model().read(cx).buffer().version() != version {
                    return;
                }
                this.set_linked_ranges(response, language_pattern, position, cx);
            })
            .ok();
        }));
    }

    fn set_linked_ranges(
        &mut self,
        response: Option<LinkedEditingRanges>,
        language_pattern: Option<syntax::Pattern>,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        let Some(response) = response else {
            self.clear_linked_editing(cx);
            return;
        };
        let mut ranges = response.ranges;
        // The range with the cursor is the reference.
        let Some(reference) = ranges
            .iter()
            .position(|r| r.start <= position && position <= r.end)
        else {
            self.clear_linked_editing(cx);
            return;
        };
        let reference = ranges.remove(reference);
        ranges.insert(0, reference);
        self.linked.word_pattern = response.word_pattern.or(language_pattern);
        let rope = self.model().read(cx).buffer().rope().clone();
        let options = DecorationOptions {
            background: Some(self.theme.colors.linked_editing_background),
            stickiness: TrackedRangeStickiness::AlwaysGrowsWhenTypingAtEdges,
            ..Default::default()
        };
        let decorations = ranges
            .iter()
            .map(|r| {
                (
                    rope.point_utf16_to_offset(r.start)..rope.point_utf16_to_offset(r.end),
                    options.clone(),
                )
            })
            .collect();
        let old = std::mem::take(&mut self.linked.decorations);
        let owner = Some(cx.entity_id());
        self.linked.decorations = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, decorations, cx)
        });
        cx.notify();
    }
}
