//! The overview ruler: decoration and cursor marks under the vertical
//! scrollbar.
//!
//! Ports Monaco's `DecorationsOverviewRuler`
//! (`src/vs/editor/browser/viewParts/overviewRuler/decorationsOverviewRuler.ts`),
//! the grouping of `OverviewRulerDecorations` (`viewModel.ts`), and
//! `OverviewZoneManager` and `OverviewRuler._renderOneLane`
//! (`src/vs/editor/common/viewModel/overviewZoneManager.ts`,
//! `viewParts/overviewRuler/overviewRuler.ts`) (MIT).
//!
//! As on Monaco's canvases, marks are computed in device pixels: a content
//! offset `y` maps to `y * canvas_height / scroll_height`, so the marks line
//! up with the slider, view zones, padding and scrolling beyond the last
//! line included.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Bounds, Hsla, Pixels, Window, fill, point, px, size};

use crate::decorations::OverviewRulerLane;

/// Monaco `MIN_DECORATION_HEIGHT`: a decoration mark is at least this tall
/// (in CSS pixels).
const MIN_DECORATION_HEIGHT: f32 = 6.;

/// Monaco `OverviewZoneManager`'s `MINIMUM_HEIGHT`.
const MINIMUM_ZONE_HEIGHT: f32 = 4.;

/// Monaco's `OverviewRulerLane` bit flags, indexing [`RulerGeometry`]'s
/// lanes.
pub(crate) fn lane_bits(lane: OverviewRulerLane) -> u8 {
    match lane {
        OverviewRulerLane::Left => 1,
        OverviewRulerLane::Center => 2,
        OverviewRulerLane::Right => 4,
        OverviewRulerLane::Full => 7,
    }
}

/// The x offset and width of each lane combination (Monaco
/// `Settings._initLanes`): lanes start `canvas_left_offset` in; with three,
/// the side lanes are floored thirds and the center takes the rest; with
/// two, the center shares the left half.
pub(crate) fn init_lanes(
    canvas_left_offset: i32,
    canvas_width: i32,
    lane_count: u32,
) -> ([i32; 8], [i32; 8]) {
    let remaining = canvas_width - canvas_left_offset;
    let o = canvas_left_offset;
    if lane_count >= 3 {
        let left = remaining / 3;
        let right = remaining / 3;
        let center = remaining - left - right;
        let center_offset = o + left;
        let right_offset = o + left + center;
        (
            [0, o, center_offset, o, right_offset, o, center_offset, o],
            [
                0,
                left,
                center,
                left + center,
                right,
                left + center + right,
                center + right,
                left + center + right,
            ],
        )
    } else if lane_count == 2 {
        let left = remaining / 2;
        let right = remaining - left;
        let right_offset = o + left;
        (
            [0, o, o, o, right_offset, o, o, o],
            [
                0,
                left,
                left,
                left,
                right,
                left + right,
                left + right,
                left + right,
            ],
        )
    } else {
        (
            [0, o, o, o, o, o, o, o],
            [
                0, remaining, remaining, remaining, remaining, remaining, remaining, remaining,
            ],
        )
    }
}

/// The decorations of one color (Monaco `OverviewRulerDecorationsGroup`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecorationGroup {
    pub color: Hsla,
    pub z_index: i32,
    /// (lane bits, first view line, last view line), in decoration order.
    pub data: Vec<(u8, u32, u32)>,
}

/// Groups overview ruler decorations by color (Monaco
/// `OverviewRulerDecorations`): a decoration in the same lane as the
/// previous one of its color, and starting at most a line after it,
/// extends it.
#[derive(Default)]
pub(crate) struct DecorationGroups {
    groups: Vec<DecorationGroup>,
    by_color: HashMap<u32, usize>,
}

impl DecorationGroups {
    /// Adds a decoration covering view lines `start..=end`. Decorations
    /// come sorted by start.
    pub fn accept(&mut self, color: Hsla, z_index: i32, start: u32, end: u32, lane: u8) {
        let key = color_key(color);
        if let Some(&ix) = self.by_color.get(&key) {
            let data = &mut self.groups[ix].data;
            if let Some(last) = data.last_mut()
                && last.0 == lane
                && last.2 + 1 >= start
            {
                last.2 = last.2.max(end);
                return;
            }
            data.push((lane, start, end));
        } else {
            self.by_color.insert(key, self.groups.len());
            self.groups.push(DecorationGroup {
                color,
                z_index,
                data: vec![(lane, start, end)],
            });
        }
    }

    /// The groups in painting order (Monaco
    /// `OverviewRulerDecorationsGroup.compareByRenderingProps`: by z-index,
    /// then color).
    pub fn into_sorted(mut self) -> Vec<DecorationGroup> {
        self.groups
            .sort_by_key(|group| (group.z_index, color_key(group.color)));
        self.groups
    }
}

/// A color as packed RGBA, to group and order by.
fn color_key(color: Hsla) -> u32 {
    let rgba = gpui::Rgba::from(color);
    let byte = |c: f32| (c.clamp(0., 1.) * 255.).round() as u32;
    (byte(rgba.r) << 24) | (byte(rgba.g) << 16) | (byte(rgba.b) << 8) | byte(rgba.a)
}

/// A rectangle on the ruler, in device pixels from its top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CanvasRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The ruler's canvas (Monaco `DecorationsOverviewRuler`'s `Settings`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RulerGeometry {
    pub canvas_width: i32,
    pub canvas_height: i32,
    pub pixel_ratio: f32,
    /// Monaco `getScrollHeight`: the content height, scrolling beyond the
    /// last line included, and at least the viewport.
    pub scroll_height: f32,
    pub line_height: f32,
    /// Each lane combination's x and width ([`init_lanes`], 1px in).
    pub x: [i32; 8],
    pub w: [i32; 8],
}

impl RulerGeometry {
    /// A ruler `width` × `height` CSS pixels with `lanes` lanes.
    pub fn new(
        width: f32,
        height: f32,
        pixel_ratio: f32,
        scroll_height: f32,
        line_height: f32,
        lanes: u32,
    ) -> Self {
        // `(domWidth * pixelRatio) | 0`.
        let canvas_width = (width * pixel_ratio) as i32;
        let canvas_height = (height * pixel_ratio) as i32;
        let (x, w) = init_lanes(1, canvas_width, lanes);
        Self {
            canvas_width,
            canvas_height,
            pixel_ratio,
            scroll_height: scroll_height.max(1.),
            line_height,
            x,
            w,
        }
    }

    fn height_ratio(&self) -> f32 {
        self.canvas_height as f32 / self.scroll_height
    }
}

/// The decoration marks (Monaco `DecorationsOverviewRuler._render`): each
/// spans its lines, at least [`MIN_DECORATION_HEIGHT`] tall and centered on
/// them, and marks of a group in the same lane within a pixel are merged.
/// `offset_of` is a view line's content top (Monaco
/// `getVerticalOffsetForLineNumber`).
pub(crate) fn render_decorations(
    groups: &[DecorationGroup],
    geometry: &RulerGeometry,
    offset_of: impl Fn(u32) -> f32,
) -> Vec<(CanvasRect, Hsla)> {
    let ratio = geometry.height_ratio();
    let canvas_height = geometry.canvas_height;
    let min_height = (MIN_DECORATION_HEIGHT * geometry.pixel_ratio) as i32;
    let half_min_height = min_height / 2;
    let rect = |lane: u8, y1: i32, y2: i32| CanvasRect {
        x: geometry.x[lane as usize],
        y: y1,
        width: geometry.w[lane as usize],
        height: y2 - y1,
    };
    let mut marks = Vec::new();
    for group in groups {
        let mut prev: Option<(u8, i32, i32)> = None;
        for &(lane, start, end) in &group.data {
            let mut y1 = (offset_of(start) * ratio) as i32;
            let mut y2 = ((offset_of(end) + geometry.line_height) * ratio) as i32;
            if y2 - y1 < min_height {
                let mut center = (y1 + y2) / 2;
                if center < half_min_height {
                    center = half_min_height;
                } else if center + half_min_height > canvas_height {
                    center = canvas_height - half_min_height;
                }
                y1 = center - half_min_height;
                y2 = center + half_min_height;
            }
            match &mut prev {
                Some((prev_lane, _, prev_y2)) if *prev_lane == lane && y1 <= *prev_y2 + 1 => {
                    *prev_y2 = (*prev_y2).max(y2);
                }
                _ => {
                    if let Some((lane, y1, y2)) = prev {
                        marks.push((rect(lane, y1, y2), group.color));
                    }
                    prev = Some((lane, y1, y2));
                }
            }
        }
        if let Some((lane, y1, y2)) = prev {
            marks.push((rect(lane, y1, y2), group.color));
        }
    }
    marks
}

/// The cursor marks (Monaco `DecorationsOverviewRuler._render`): 2px tall
/// across every lane, centered on the top of the cursor's line and kept
/// on the canvas, merged with the previous one of the same color within a
/// pixel. `cursors` are (view line, color), sorted by position.
pub(crate) fn render_cursors(
    cursors: &[(u32, Hsla)],
    geometry: &RulerGeometry,
    offset_of: impl Fn(u32) -> f32,
) -> Vec<(CanvasRect, Hsla)> {
    let ratio = geometry.height_ratio();
    let canvas_height = geometry.canvas_height;
    let cursor_height = (2. * geometry.pixel_ratio) as i32;
    let half_cursor_height = cursor_height / 2;
    let rect = |y1: i32, y2: i32| CanvasRect {
        x: geometry.x[7],
        y: y1,
        width: geometry.w[7],
        height: y2 - y1,
    };
    let mut marks = Vec::new();
    let mut prev: Option<(Hsla, i32, i32)> = None;
    for &(line, color) in cursors {
        let mut center = (offset_of(line) * ratio) as i32;
        if center < half_cursor_height {
            center = half_cursor_height;
        } else if center + half_cursor_height > canvas_height {
            center = canvas_height - half_cursor_height;
        }
        let y1 = center - half_cursor_height;
        let y2 = y1 + cursor_height;
        match &mut prev {
            Some((prev_color, _, prev_y2)) if *prev_color == color && y1 <= *prev_y2 + 1 => {
                *prev_y2 = (*prev_y2).max(y2);
            }
            _ => {
                if let Some((color, y1, y2)) = prev {
                    marks.push((rect(y1, y2), color));
                }
                prev = Some((color, y1, y2));
            }
        }
    }
    if let Some((color, y1, y2)) = prev {
        marks.push((rect(y1, y2), color));
    }
    marks
}

/// The overview ruler as painted (Monaco `DecorationsOverviewRuler`).
#[derive(Clone, Debug)]
pub(crate) struct RulerPaint {
    pub bounds: Bounds<Pixels>,
    /// Device pixels per pixel: the marks are in device pixels.
    pub pixel_ratio: f32,
    /// `editorOverviewRuler.background`, else the editor's with the
    /// minimap on the right.
    pub background: Option<Hsla>,
    /// The canvas's size in device pixels.
    pub canvas_size: (i32, i32),
    pub marks: Arc<Vec<(CanvasRect, Hsla)>>,
    pub cursors: Vec<(CanvasRect, Hsla)>,
    /// `editorOverviewRuler.border`, with `overviewRulerBorder`.
    pub border: Option<Hsla>,
}

impl RulerPaint {
    /// Where a canvas rectangle goes on screen.
    pub fn bounds_of(&self, rect: CanvasRect) -> Bounds<Pixels> {
        let device = |value: i32| px(value as f32 / self.pixel_ratio);
        Bounds::new(
            point(
                self.bounds.left() + device(rect.x),
                self.bounds.top() + device(rect.y),
            ),
            size(device(rect.width), device(rect.height)),
        )
    }

    /// The left and top border: Monaco strokes a 1px line along the
    /// canvas's edges, on pixel boundaries, so half of it shows.
    pub fn border_rects(&self) -> Option<([Bounds<Pixels>; 2], Hsla)> {
        let border = self.border?;
        let (width, height) = self.canvas_size;
        let left = CanvasRect {
            x: 0,
            y: 0,
            width: 1,
            height,
        };
        let top = CanvasRect {
            x: 1,
            y: 0,
            width: width - 1,
            height: 1,
        };
        Some((
            [self.bounds_of(left), self.bounds_of(top)],
            border.opacity(0.5),
        ))
    }

    /// Monaco `DecorationsOverviewRuler._render`: the background, the
    /// decorations, the cursors, then the border.
    pub fn paint(&self, window: &mut Window) {
        if let Some(background) = self.background {
            window.paint_quad(fill(self.bounds, background));
        }
        for (rect, color) in self.marks.iter().chain(&self.cursors) {
            window.paint_quad(fill(self.bounds_of(*rect), *color));
        }
        if let Some((rects, color)) = self.border_rects() {
            for rect in rects {
                window.paint_quad(fill(rect, color));
            }
        }
    }
}

/// A resolved zone on a ruler, in device pixels (Monaco `ColorZone`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColorZone {
    pub from: i32,
    pub to: i32,
    pub color_id: u32,
}

/// Monaco `OverviewZoneManager.resolveColorZones`: zones given as (content
/// top, content bottom, color id) on a ruler `dom_height` CSS pixels tall
/// for content `outer_height` tall, each at least [`MINIMUM_ZONE_HEIGHT`]
/// and centered on its range, sorted by color, then position.
pub(crate) fn resolve_color_zones(
    zones: impl IntoIterator<Item = (f32, f32, u32)>,
    dom_height: f32,
    outer_height: f32,
    pixel_ratio: f32,
) -> Vec<ColorZone> {
    let total_height = (dom_height * pixel_ratio).floor() as i32;
    let outer_height = outer_height.floor().max(1.);
    let ratio = total_height as f32 / outer_height;
    let half_minimum_height = (MINIMUM_ZONE_HEIGHT * pixel_ratio / 2.).floor() as i32;
    let mut result: Vec<ColorZone> = zones
        .into_iter()
        .map(|(offset1, offset2, color_id)| {
            let y1 = (ratio * offset1).floor() as i32;
            let y2 = (ratio * offset2).floor() as i32;
            let mut center = (y1 + y2).div_euclid(2);
            let half_height = (y2 - center).max(half_minimum_height);
            if center - half_height < 0 {
                center = half_height;
            }
            if center + half_height > total_height {
                center = total_height - half_height;
            }
            ColorZone {
                from: center - half_height,
                to: center + half_height,
                color_id,
            }
        })
        .collect();
    result.sort_by_key(|zone| (zone.color_id, zone.from, zone.to));
    result
}

/// Monaco `OverviewRuler._renderOneLane`: overlapping zones of a color
/// become one (color id, from, to) run.
pub(crate) fn merge_color_zones(zones: &[ColorZone]) -> Vec<(u32, i32, i32)> {
    let mut runs: Vec<(u32, i32, i32)> = Vec::new();
    for zone in zones {
        match runs.last_mut() {
            Some((color, _, to)) if *color == zone.color_id && *to >= zone.from => {
                *to = (*to).max(zone.to);
            }
            _ => runs.push((zone.color_id, zone.from, zone.to)),
        }
    }
    runs
}

/// An `Arc` compared by identity. Holding it keeps the address from being
/// reused while it's a cache key.
#[derive(Clone, Debug)]
pub(crate) struct ByPtr<T>(pub Arc<T>);

impl<T> PartialEq for ByPtr<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What the decoration groups depend on: (model, decorations version,
/// view lines generation, the decorations shown, the marker colors, the
/// theme for theme color ids).
pub(crate) type GroupsKey = (
    gpui::EntityId,
    u64,
    u64,
    crate::api::DecorationFilter,
    [Hsla; 3],
    usize,
);

/// The decoration marks and what they were computed from.
#[derive(Clone)]
struct MarksEntry {
    groups: ByPtr<Vec<DecorationGroup>>,
    zones: crate::element::ZoneLayout,
    geometry: RulerGeometry,
    marks: Arc<Vec<(CanvasRect, Hsla)>>,
}

/// The editor's overview ruler marks, recomputed only when the
/// decorations or the vertical layout change, not every frame.
#[derive(Clone, Default)]
pub(crate) struct OverviewRulerCache {
    groups: Option<(GroupsKey, Arc<Vec<DecorationGroup>>)>,
    marks: Option<MarksEntry>,
    /// The ruler last laid out, for tests.
    pub painted: Option<RulerPaint>,
}

impl OverviewRulerCache {
    /// The decoration groups for `key`, computed if it changed.
    pub fn groups(
        &mut self,
        key: GroupsKey,
        compute: impl FnOnce() -> Vec<DecorationGroup>,
    ) -> Arc<Vec<DecorationGroup>> {
        match &self.groups {
            Some((k, groups)) if *k == key => groups.clone(),
            _ => {
                let groups = Arc::new(compute());
                self.groups = Some((key, groups.clone()));
                groups
            }
        }
    }

    /// The marks of `groups` with the view zones and line heights in
    /// `zones` on a ruler of `geometry`, computed if any of them changed.
    pub fn marks(
        &mut self,
        groups: &Arc<Vec<DecorationGroup>>,
        zones: &crate::element::ZoneLayout,
        geometry: &RulerGeometry,
        offset_of: impl Fn(u32) -> f32,
    ) -> Arc<Vec<(CanvasRect, Hsla)>> {
        if let Some(entry) = &self.marks
            && Arc::ptr_eq(&entry.groups.0, groups)
            && entry.geometry == *geometry
            && entry.zones == *zones
        {
            return entry.marks.clone();
        }
        let marks = Arc::new(render_decorations(groups, geometry, offset_of));
        self.marks = Some(MarksEntry {
            groups: ByPtr(groups.clone()),
            zones: zones.clone(),
            geometry: geometry.clone(),
            marks: marks.clone(),
        });
        marks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgb;

    /// Monaco's `OverviewRulerZone(start, end, 0, color)` with the tests'
    /// `getVerticalOffsetForLine` (`LINE_HEIGHT * lineNumber`).
    fn zone(start: u32, end: u32, color_id: u32) -> (f32, f32, u32) {
        const LINE_HEIGHT: f32 = 20.;
        (
            LINE_HEIGHT * start as f32,
            LINE_HEIGHT * end as f32 + LINE_HEIGHT,
            color_id,
        )
    }

    fn zones() -> Vec<(f32, f32, u32)> {
        vec![
            zone(1, 1, 1),
            zone(10, 10, 2),
            zone(30, 31, 3),
            zone(50, 50, 4),
        ]
    }

    fn color_zone(from: i32, to: i32, color_id: u32) -> ColorZone {
        ColorZone { from, to, color_id }
    }

    // `overviewZoneManager.test.ts`.
    #[test]
    fn pixel_ratio_1_dom_height_600() {
        // One line is 12px, more than the 4px minimum.
        assert_eq!(
            resolve_color_zones(zones(), 600., 50. * 20., 1.),
            vec![
                color_zone(12, 24, 1),
                color_zone(120, 132, 2),
                color_zone(360, 384, 3),
                color_zone(588, 600, 4),
            ]
        );
    }

    #[test]
    fn pixel_ratio_1_dom_height_300() {
        assert_eq!(
            resolve_color_zones(zones(), 300., 50. * 20., 1.),
            vec![
                color_zone(6, 12, 1),
                color_zone(60, 66, 2),
                color_zone(180, 192, 3),
                color_zone(294, 300, 4),
            ]
        );
    }

    #[test]
    fn pixel_ratio_2_dom_height_300() {
        assert_eq!(
            resolve_color_zones(zones(), 300., 50. * 20., 2.),
            vec![
                color_zone(12, 24, 1),
                color_zone(120, 132, 2),
                color_zone(360, 384, 3),
                color_zone(588, 600, 4),
            ]
        );
    }

    #[test]
    fn short_zones_get_the_minimum_height_and_stay_on_the_ruler() {
        // A 1px range near the bottom: 4px tall, ending at the bottom.
        let resolved = resolve_color_zones([(999., 1000., 1)], 100., 1000., 1.);
        assert_eq!(resolved, vec![color_zone(96, 100, 1)]);
    }

    #[test]
    fn render_one_lane_merges_overlapping_zones_of_a_color() {
        let zones = [
            color_zone(0, 4, 1),
            color_zone(3, 8, 1),
            color_zone(10, 12, 1),
            color_zone(11, 20, 2),
        ];
        assert_eq!(
            merge_color_zones(&zones),
            vec![(1, 0, 8), (1, 10, 12), (2, 11, 20)]
        );
    }

    #[test]
    fn lanes_start_one_pixel_in_and_the_center_takes_the_rest() {
        // Monaco `_initLanes(1, 28, 3)`: 27px left, as 9 + 9 + 9.
        let (x, w) = init_lanes(1, 28, 3);
        assert_eq!((x[1], w[1]), (1, 9));
        assert_eq!((x[2], w[2]), (10, 9));
        assert_eq!((x[4], w[4]), (19, 9));
        assert_eq!((x[7], w[7]), (1, 27));
        // 13px left: the sides floor to 4, the center takes 5.
        let (x, w) = init_lanes(1, 14, 3);
        assert_eq!((x[1], w[1], x[2], w[2], x[4], w[4]), (1, 4, 5, 5, 10, 4));
        // Two lanes: the center shares the left half.
        let (x, w) = init_lanes(1, 14, 2);
        assert_eq!((x[2], w[2], x[4], w[4]), (1, 6, 7, 7));
        let (x, w) = init_lanes(1, 14, 1);
        assert_eq!((x[4], w[4]), (1, 13));
    }

    #[test]
    fn groups_merge_adjacent_lines_in_a_lane_and_sort_by_z_index() {
        let red: Hsla = rgb(0xff0000).into();
        let blue: Hsla = rgb(0x0000ff).into();
        let mut groups = DecorationGroups::default();
        groups.accept(red, 10, 0, 0, 4);
        // The next line extends it; a gap starts another.
        groups.accept(red, 10, 1, 2, 4);
        groups.accept(red, 10, 4, 4, 4);
        // Another lane doesn't merge.
        groups.accept(red, 10, 5, 5, 2);
        groups.accept(blue, 5, 0, 0, 7);
        let sorted = groups.into_sorted();
        assert_eq!(sorted[0].color, blue);
        assert_eq!(sorted[1].data, vec![(4, 0, 2), (4, 4, 4), (2, 5, 5)]);
    }

    fn geometry(height: f32, scroll_height: f32, pixel_ratio: f32) -> RulerGeometry {
        RulerGeometry::new(14., height, pixel_ratio, scroll_height, 20., 3)
    }

    #[test]
    fn decoration_marks_span_their_lines_with_a_minimum_height() {
        let red: Hsla = rgb(0xff0000).into();
        let groups = vec![DecorationGroup {
            color: red,
            z_index: 0,
            // Line 0, lines 10–29, and line 49 (the last).
            data: vec![(4, 0, 0), (4, 10, 29), (4, 49, 49)],
        }];
        // 50 lines of 20px on a 500px ruler: 10px per line.
        let g = geometry(500., 1000., 1.);
        let marks = render_decorations(&groups, &g, |line| line as f32 * 20.);
        let spans: Vec<(i32, i32)> = marks.iter().map(|(r, _)| (r.y, r.height)).collect();
        // One line is 10px, above the 6px minimum.
        assert_eq!(spans, vec![(0, 10), (100, 200), (490, 10)]);
        assert!(marks.iter().all(|(r, _)| (r.x, r.width) == (10, 4)));

        // On a 100px ruler a line is 2px: 6px marks, centered and kept on
        // the canvas.
        let g = geometry(100., 1000., 1.);
        let marks = render_decorations(&groups, &g, |line| line as f32 * 20.);
        let spans: Vec<(i32, i32)> = marks.iter().map(|(r, _)| (r.y, r.height)).collect();
        assert_eq!(spans, vec![(0, 6), (20, 40), (94, 6)]);

        // Twice the pixel ratio: twice the minimum, in device pixels.
        let g = geometry(100., 1000., 2.);
        let marks = render_decorations(&groups, &g, |line| line as f32 * 20.);
        assert_eq!(marks[0].0.height, 12);
    }

    #[test]
    fn decoration_marks_within_a_pixel_merge() {
        let red: Hsla = rgb(0xff0000).into();
        // On a 100px ruler of 50 lines, lines 0 and 2 are 6px marks at 0..6
        // and 2..8: one mark.
        let groups = vec![DecorationGroup {
            color: red,
            z_index: 0,
            data: vec![(4, 0, 0), (4, 2, 2), (4, 40, 40)],
        }];
        let g = geometry(100., 1000., 1.);
        let marks = render_decorations(&groups, &g, |line| line as f32 * 20.);
        let spans: Vec<(i32, i32)> = marks.iter().map(|(r, _)| (r.y, r.height)).collect();
        assert_eq!(spans, vec![(0, 8), (78, 6)]);
    }

    #[test]
    fn cursor_marks_are_two_pixels_at_the_line_top() {
        let a: Hsla = rgb(0xff0000).into();
        let b: Hsla = rgb(0x00ff00).into();
        let g = geometry(500., 1000., 1.);
        let marks = render_cursors(&[(0, a), (10, a), (10, b), (49, a)], &g, |line| {
            line as f32 * 20.
        });
        let spans: Vec<(i32, i32, Hsla)> = marks.iter().map(|(r, c)| (r.y, r.height, *c)).collect();
        // Clamped to the top, at line 10's top (100px), and a different
        // color doesn't merge.
        assert_eq!(spans, vec![(0, 2, a), (99, 2, a), (99, 2, b), (489, 2, a)]);
        assert!(marks.iter().all(|(r, _)| (r.x, r.width) == (1, 13)));
    }
}
