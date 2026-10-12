//! The multi-diff list's anchored geometry edits (Monaco
//! `compressedVirtualizedScrollLayout.ts` and the anchoring part of
//! `CompressedVirtualizedScrollView._update`, MIT).
//!
//! When files change height (measured, collapsed, expanded), Monaco keeps
//! the point at the viewport's top (its bottom while scrolling up) where it
//! is on screen: it maps that point through the size edits and scrolls to
//! match. When that would scroll above the start or past the end, it adds
//! scroll slack there instead (`leadingScrollSlack`, `trailingScrollSlack`),
//! which scrolling towards it uses up. Monaco's item gap
//! (`_spaceBetweenPx`) is always 0, so it's left out.

use std::ops::Range;
use std::time::Duration;

use gpui::{Pixels, px};
use web_time::Instant;

/// Monaco's `scrollDirectionRetentionDurationMs`: how long the last scroll
/// direction picks the anchor.
const SCROLL_DIRECTION_RETENTION: Duration = Duration::from_millis(100);

/// One item's old and new range (Monaco `ISizeEdit`).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SizeEdit {
    pub old: Range<Pixels>,
    pub new: Range<Pixels>,
}

/// Monaco `computeItemRanges`.
pub(super) fn item_ranges(heights: &[Pixels]) -> Vec<Range<Pixels>> {
    let mut top = px(0.);
    heights
        .iter()
        .map(|&height| {
            let range = top..top + height;
            top += height;
            range
        })
        .collect()
}

/// Monaco `computeCompressedVirtualizedScrollHeight`.
pub(super) fn scroll_height(heights: &[Pixels]) -> Pixels {
    heights.iter().fold(px(0.), |a, &b| a + b)
}

/// The edits of `createAnchoredSizeEditBatch`: each item whose range
/// changed. `old` and `new` have the same items.
pub(super) fn size_edits(old: &[Pixels], new: &[Pixels]) -> Vec<SizeEdit> {
    item_ranges(old)
        .into_iter()
        .zip(item_ranges(new))
        .filter(|(old, new)| old != new)
        .map(|(old, new)| SizeEdit { old, new })
        .collect()
}

/// Monaco `mapLogicalPosition`: `offset` after `edits`. A point inside an
/// edited item keeps its offset in the item, or goes to its start when the
/// item got shorter than that.
pub(super) fn map_offset(offset: Pixels, edits: &[SizeEdit]) -> Pixels {
    let mut mapped = offset;
    for edit in edits {
        if offset < edit.old.start {
            break;
        }
        if edit.old.contains(&offset) {
            let relative = offset - edit.old.start;
            return if relative < edit.new.end - edit.new.start {
                edit.new.start + relative
            } else {
                edit.new.start
            };
        }
        mapped = offset + edit.new.end - edit.old.end;
    }
    mapped
}

/// The list's scroll anchoring state (Monaco
/// `CompressedVirtualizedScrollView`'s `_previousItemHeights`, scroll
/// slack and scroll direction).
#[derive(Debug, Default)]
pub(super) struct ScrollAnchoring {
    /// The item heights last laid out, `None` before the first layout of
    /// the current items.
    previous_heights: Option<Vec<Pixels>>,
    /// Room above the first item (Monaco `leadingScrollSlack`).
    pub leading_slack: Pixels,
    /// Room below the last item (Monaco `trailingScrollSlack`).
    pub trailing_slack: Pixels,
    /// The scroll position last laid out (Monaco `_lastScrollTop`).
    pub last_scroll_top: Pixels,
    /// The last scroll direction (up?) and when (Monaco
    /// `_lastScrollDirection`, cleared after 100ms).
    last_direction: Option<(bool, Instant)>,
}

impl ScrollAnchoring {
    /// Forgets the items: new ones start without slack (Monaco's
    /// `itemsChanged`).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The whole scroll height: the items and the slack.
    pub fn scroll_height(&self, heights: &[Pixels]) -> Pixels {
        self.leading_slack + scroll_height(heights) + self.trailing_slack
    }

    /// Monaco `_update`: the scroll position to lay `heights` out at for
    /// the `requested` one, anchoring when the heights changed and using up
    /// slack when scrolling towards it. Returns it, and whether the
    /// geometry changed (Monaco then scrolls at once, ending a smooth
    /// scroll).
    pub fn update(
        &mut self,
        heights: &[Pixels],
        viewport: Pixels,
        requested: Pixels,
        now: Instant,
    ) -> (Pixels, bool) {
        let mut target = requested;
        let delta = requested - self.last_scroll_top;
        if delta != px(0.) {
            self.last_direction = Some((delta < px(0.), now));
        }
        let scrolling_up = self
            .last_direction
            .is_some_and(|(up, at)| up && now.duration_since(at) < SCROLL_DIRECTION_RETENTION);
        let previous = self.previous_heights.replace(heights.to_vec());
        let changed = match previous {
            Some(previous) if previous.len() == heights.len() && previous != heights => {
                let old_height = scroll_height(&previous);
                let old_top = requested - self.leading_slack;
                let anchor = (old_top + if scrolling_up { viewport } else { px(0.) })
                    .clamp(px(0.), old_height.max(px(0.)));
                let anchor_viewport_offset = anchor - old_top;
                let mapped = map_offset(anchor, &size_edits(&previous, heights));
                let desired = mapped - anchor_viewport_offset;
                let natural_max = (scroll_height(heights) - viewport).max(px(0.));
                self.leading_slack = (-desired).max(px(0.));
                self.trailing_slack = (desired - natural_max).max(px(0.));
                target = self.leading_slack + desired;
                true
            }
            Some(previous) if previous.len() != heights.len() => {
                self.leading_slack = px(0.);
                self.trailing_slack = px(0.);
                false
            }
            _ => {
                if delta > px(0.) && self.leading_slack > px(0.) {
                    let consumed = delta.min(self.leading_slack);
                    self.leading_slack -= consumed;
                    target -= consumed;
                } else if delta < px(0.) && self.trailing_slack > px(0.) {
                    let natural_max =
                        self.leading_slack + (scroll_height(heights) - viewport).max(px(0.));
                    self.trailing_slack = (target - natural_max).max(px(0.));
                }
                false
            }
        };
        (target, changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pxs(values: &[f32]) -> Vec<Pixels> {
        values.iter().map(|&v| px(v)).collect()
    }

    #[test]
    fn map_offset_through_edits() {
        // Monaco `compressedVirtualizedScrollLayout.test.ts`-style cases:
        // an item above shrinks, the point moves up by as much.
        let edits = size_edits(&pxs(&[100., 100., 100.]), &pxs(&[50., 100., 100.]));
        assert_eq!(map_offset(px(150.), &edits), px(100.));
        // Inside the edited item: same offset in it, or its start when it
        // got shorter than that.
        assert_eq!(map_offset(px(20.), &edits), px(20.));
        assert_eq!(map_offset(px(80.), &edits), px(0.));
        // Before every edit: unchanged.
        let edits = size_edits(&pxs(&[100., 100.]), &pxs(&[100., 300.]));
        assert_eq!(map_offset(px(50.), &edits), px(50.));
    }

    #[test]
    fn anchoring_keeps_the_viewport_top() {
        let now = Instant::now();
        let mut anchoring = ScrollAnchoring::default();
        let viewport = px(100.);
        let heights = pxs(&[500., 500., 500.]);
        assert_eq!(
            anchoring.update(&heights, viewport, px(0.), now),
            (px(0.), false)
        );
        anchoring.last_scroll_top = px(0.);
        // Scrolled into the second item, the first one shrinks: the second
        // item's row at the top stays there.
        let (top, _) = anchoring.update(&heights, viewport, px(600.), now);
        anchoring.last_scroll_top = top;
        let later = now + Duration::from_millis(200);
        let (top, changed) = anchoring.update(&pxs(&[100., 500., 500.]), viewport, top, later);
        assert!(changed);
        assert_eq!(top, px(200.));
        assert_eq!(anchoring.leading_slack, px(0.));
    }

    #[test]
    fn slack_past_the_end_is_used_up_scrolling_back() {
        let now = Instant::now();
        let mut anchoring = ScrollAnchoring::default();
        let viewport = px(100.);
        anchoring.update(&pxs(&[500., 500.]), viewport, px(0.), now);
        // At the end, the last item shrinks: the top stays, so there's
        // room past the end.
        anchoring.last_scroll_top = px(900.);
        let later = now + Duration::from_millis(200);
        let (top, _) = anchoring.update(&pxs(&[500., 450.]), viewport, px(900.), later);
        assert_eq!(top, px(900.));
        assert_eq!(anchoring.trailing_slack, px(50.));
        assert_eq!(anchoring.scroll_height(&pxs(&[500., 450.])), px(1000.));
        anchoring.last_scroll_top = top;
        // Scrolling up uses it up.
        let (top, _) = anchoring.update(&pxs(&[500., 450.]), viewport, px(870.), later);
        assert_eq!(top, px(870.));
        assert_eq!(anchoring.trailing_slack, px(20.));
    }

    #[test]
    fn slack_above_the_start_is_used_up_scrolling_down() {
        let now = Instant::now();
        let mut anchoring = ScrollAnchoring::default();
        let viewport = px(100.);
        anchoring.update(&pxs(&[100., 500.]), viewport, px(0.), now);
        anchoring.last_scroll_top = px(50.);
        // Scrolling up past the half of the first item, which then grows
        // above the anchor at the viewport's bottom.
        let (top, _) = anchoring.update(&pxs(&[100., 500.]), viewport, px(40.), now);
        anchoring.last_scroll_top = top;
        let (top, changed) = anchoring.update(&pxs(&[100., 500.]), viewport, top, now);
        assert!(!changed);
        assert_eq!(top, px(40.));
        // Items before the anchor shrink while scrolling up: the bottom
        // (140, now 60) stays 100px down, which needs 40px above the start.
        let (top, _) = anchoring.update(&pxs(&[20., 500.]), viewport, px(40.), now);
        assert_eq!(anchoring.leading_slack, px(40.));
        assert_eq!(top, px(0.));
        anchoring.last_scroll_top = top;
        // Scrolling down uses the slack up while the content scrolls.
        let (top, _) = anchoring.update(&pxs(&[20., 500.]), viewport, px(30.), now);
        assert_eq!(anchoring.leading_slack, px(10.));
        assert_eq!(top, px(0.));
    }
}
