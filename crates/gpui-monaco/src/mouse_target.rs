//! Mouse hit testing (Monaco's `IMouseTarget` and `MouseTargetFactory`)
//! and the mouse events hosts subscribe to (`onMouseDown`, `onMouseUp`,
//! `onMouseMove`, `onMouseLeave`, `onContextMenu`), e.g. to toggle
//! breakpoints in the glyph margin.

use std::ops::Range;

use gpui::{App, EntityId, Modifiers, MouseButton, Pixels, Point, Window, px};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::widgets::WidgetId;

/// What an embedded view's bounds belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedKind {
    /// A view zone between lines.
    ViewZone,
    /// A view zone's margin part, in the gutter (Monaco `marginDomNode`).
    ViewZoneMargin,
    /// A content widget.
    ContentWidget,
    /// An overlay widget.
    OverlayWidget,
    /// A glyph margin widget.
    GlyphMarginWidget,
}

/// The part of the editor under the mouse (Monaco `MouseTargetType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseTargetKind {
    /// The glyph margin, where breakpoint-style glyphs go, glyph margin
    /// widgets included.
    GutterGlyphMargin,
    /// The line numbers.
    GutterLineNumbers,
    /// The line decorations and folding area between the numbers and the
    /// text.
    GutterLineDecorations,
    /// A view zone in the gutter.
    GutterViewZone,
    /// Text.
    ContentText,
    /// Past the end of a line, or below the last line (`after_lines`).
    ContentEmpty {
        /// Below the last line.
        after_lines: bool,
    },
    /// A view zone in the text area.
    ContentViewZone,
    /// A content widget.
    ContentWidget,
    /// An overlay widget.
    OverlayWidget,
    /// The vertical scrollbar and its overview ruler.
    OverviewRuler,
    /// The horizontal scrollbar.
    Scrollbar,
    /// The minimap.
    Minimap,
    /// A sticky scroll header.
    StickyScroll,
    /// Outside the editor's text and gutter.
    Unknown,
}

/// More about what the mouse is over (Monaco's `IMouseTarget` variants'
/// `detail`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MouseTargetDetail {
    /// Nothing more.
    #[default]
    None,
    /// The gutter (Monaco `IMouseTargetMarginData`).
    Margin {
        /// Below the last line (Monaco `isAfterLines`).
        is_after_lines: bool,
        /// Where the glyph margin starts, from the editor's left (Monaco
        /// `glyphMarginLeft`).
        glyph_margin_left: Pixels,
        /// The glyph margin's width (Monaco `glyphMarginWidth`).
        glyph_margin_width: Pixels,
        /// Over the glyph margin, its lane among those in use on the line,
        /// as a [`GlyphMarginLane`](crate::GlyphMarginLane) number: 1 left,
        /// 2 center, 3 right (Monaco `glyphMarginLane`).
        glyph_margin_lane: Option<u32>,
        /// The line numbers' width (Monaco `lineNumbersWidth`).
        line_numbers_width: Pixels,
        /// The mouse from the editor's left (Monaco `offsetX`).
        offset_x: Pixels,
        /// The glyph margin widget under the mouse, if any.
        widget: Option<WidgetId>,
    },
    /// A view zone (Monaco `IMouseTargetViewZoneData`).
    ViewZone {
        /// The zone, when it's known (Monaco `viewZoneId`); the editor's own
        /// zones (peek views, code lenses) have ids too.
        id: Option<WidgetId>,
        /// The line the zone is after, 1-based, 0 above the first line
        /// (Monaco `afterLineNumber`, as [`ViewZone::after_line`](crate::ViewZone)).
        after_line: Option<u32>,
        /// The end of the line above the zone, if any (Monaco
        /// `positionBefore`).
        position_before: Option<PointUtf16>,
        /// The start of the line below the zone, if any (Monaco
        /// `positionAfter`).
        position_after: Option<PointUtf16>,
    },
    /// A content widget (Monaco `IMouseTargetContentWidget.detail`).
    ContentWidget(WidgetId),
    /// An overlay widget (Monaco `IMouseTargetOverlayWidget.detail`).
    OverlayWidget(WidgetId),
    /// Empty content (Monaco `IMouseTargetContentEmptyData`).
    ContentEmpty {
        /// Below the last line (Monaco `isAfterLines`).
        is_after_lines: bool,
        /// How far right of the line's end the mouse is (Monaco
        /// `horizontalDistanceToText`).
        horizontal_distance_to_text: Option<Pixels>,
    },
}

/// What the mouse is over (Monaco `IMouseTarget`).
#[derive(Clone, Debug, PartialEq)]
pub struct MouseTarget {
    /// The kind of target (Monaco `type`).
    pub kind: MouseTargetKind,
    /// The text position: the nearest one in text, the line start in the
    /// gutter, the header's row in sticky scroll; `None` elsewhere (Monaco
    /// `position`).
    pub position: Option<PointUtf16>,
    /// The mouse position in window coordinates.
    pub point: Point<Pixels>,
    /// The text range the target covers (Monaco `range`): the position in
    /// the text, the whole line in the gutter.
    pub range: Option<Range<PointUtf16>>,
    /// The column under the mouse counted in the font's typical character
    /// width, even past a line's end (Monaco `mouseColumn`, 0-based here);
    /// 0 left of the text.
    pub mouse_column: u32,
    /// More about the target (Monaco `detail`).
    pub detail: MouseTargetDetail,
    /// Over text, the injected text under the mouse, if any (Monaco
    /// `IMouseTargetContentText.detail.injectedText`).
    pub injected_text: Option<InjectedTextTarget>,
}

/// Injected text under the mouse (Monaco's mouse target `injectedText`).
#[derive(Clone, Debug, PartialEq)]
pub struct InjectedTextTarget {
    /// The decoration whose `before` or `after` text it is.
    pub decoration: Option<crate::DecorationId>,
    /// Its [`InjectedText::attached_data`](crate::InjectedText::attached_data)
    /// (Monaco `options.attachedData`).
    pub attached_data: Option<crate::AttachedData>,
}

/// A mouse event over the editor (Monaco `IEditorMouseEvent`).
#[derive(Clone, Debug, PartialEq)]
pub struct EditorMouseEvent {
    /// What the mouse is over.
    pub target: MouseTarget,
    /// The button, for down and up events.
    pub button: Option<MouseButton>,
    /// Held modifiers.
    pub modifiers: Modifiers,
    /// Click count (2 for a double click), for down events.
    pub click_count: usize,
}

impl CodeEditor {
    /// The widget whose view is `view`, of the kind `kind` hit.
    fn widget_for_view(&self, kind: EmbeddedKind, view: EntityId) -> Option<WidgetId> {
        let widgets = &self.widgets;
        match kind {
            EmbeddedKind::ViewZone => widgets
                .zones
                .iter()
                .find(|w| w.view.entity_id() == view)
                .map(|w| w.id),
            EmbeddedKind::ViewZoneMargin => widgets
                .zones
                .iter()
                .find(|w| w.margin_view.as_ref().map(|v| v.entity_id()) == Some(view))
                .map(|w| w.id),
            EmbeddedKind::ContentWidget => widgets
                .content
                .iter()
                .find(|w| w.view.entity_id() == view)
                .map(|w| w.id),
            EmbeddedKind::OverlayWidget => widgets
                .overlays
                .iter()
                .find(|w| w.view.entity_id() == view)
                .map(|w| w.id),
            EmbeddedKind::GlyphMarginWidget => widgets
                .glyphs
                .iter()
                .find(|w| w.view.entity_id() == view)
                .map(|w| w.id),
        }
    }

    /// A view zone's target (Monaco `HitTestContext.getZoneAtCoord`): the
    /// position before the zone in its upper half, after it in the lower
    /// half (whichever exists), with `zone` the part of it hit.
    fn zone_target(
        &self,
        id: Option<WidgetId>,
        zone: Option<gpui::Bounds<Pixels>>,
        point: Point<Pixels>,
        cx: &App,
    ) -> (Option<PointUtf16>, MouseTargetDetail) {
        let after_line = id.and_then(|id| self.zone_after_line(id, cx));
        let model = self.model().read(cx);
        let line_count = model.buffer().max_point().row + 1;
        let (position_before, position_after) = match after_line {
            Some(after) => (
                (after > 0).then(|| {
                    let row = (after - 1).min(line_count - 1);
                    PointUtf16::new(row, model.line_len_utf16(row))
                }),
                (after < line_count).then(|| PointUtf16::new(after, 0)),
            ),
            None => (None, None),
        };
        let upper_half = zone.is_none_or(|zone| point.y < zone.top() + zone.size.height / 2.);
        let position = match (position_before, position_after) {
            (before, None) => before,
            (None, after) => after,
            (Some(before), Some(after)) => Some(if upper_half { before } else { after }),
        };
        (
            position,
            MouseTargetDetail::ViewZone {
                id,
                after_line,
                position_before,
                position_after,
            },
        )
    }

    /// The line view zone `id` is after (Monaco `afterLineNumber`).
    fn zone_after_line(&self, id: WidgetId, cx: &App) -> Option<u32> {
        let zone = self.widgets.zones.iter().find(|z| z.id == id)?;
        let buffer = self.model().read(cx).buffer();
        Some(match &zone.anchor {
            Some(anchor) => buffer.offset_to_point(buffer.resolve(anchor)).row + 1,
            None => 0,
        })
    }

    /// What's under `point` (window coordinates) as of the last frame
    /// (Monaco's mouse target).
    pub fn mouse_target(&self, point: Point<Pixels>, window: &mut Window, cx: &App) -> MouseTarget {
        let unknown = MouseTarget {
            kind: MouseTargetKind::Unknown,
            position: None,
            point,
            range: None,
            mouse_column: 0,
            detail: MouseTargetDetail::None,
            injected_text: None,
        };
        let Some(layout) = self.layout.as_ref() else {
            return unknown;
        };
        if !layout.bounds.contains(&point) {
            return unknown;
        }
        // Monaco `_getMouseColumn`, from the typical character width.
        let relative_x = point.x - layout.text_origin_x();
        let mouse_column = if relative_x < px(0.) || layout.metrics.em_width <= px(0.) {
            0
        } else {
            (relative_x / layout.metrics.em_width).round() as u32
        };
        let target = |kind, position: Option<PointUtf16>, detail| MouseTarget {
            kind,
            position,
            point,
            range: position.map(|p| p..p),
            mouse_column,
            detail,
            injected_text: None,
        };
        // Monaco `_hitTestMargin`: over the glyph margin, the lane among
        // those in use on `row`'s line.
        let margin = |is_after_lines, widget, row: Option<u32>| MouseTargetDetail::Margin {
            is_after_lines,
            glyph_margin_left: px(0.),
            glyph_margin_width: layout.glyph_margin_width,
            glyph_margin_lane: row
                .and_then(|row| layout.glyph_margin_lane_at(point.x, row))
                .map(|lane| lane as u32),
            line_numbers_width: layout.line_numbers_width,
            offset_x: point.x - layout.bounds.left(),
            widget,
        };
        // Widgets first, top-most (last painted) first.
        let hits = layout
            .widget_bounds
            .iter()
            .zip(&layout.widget_kinds)
            .zip(&layout.widget_views)
            .rev();
        for ((bounds, kind), view) in hits.clone() {
            if !bounds.contains(&point) {
                continue;
            }
            let id = self.widget_for_view(*kind, *view);
            let (kind, detail) = match kind {
                EmbeddedKind::ContentWidget => (
                    MouseTargetKind::ContentWidget,
                    id.map_or(MouseTargetDetail::None, MouseTargetDetail::ContentWidget),
                ),
                EmbeddedKind::OverlayWidget => (
                    MouseTargetKind::OverlayWidget,
                    id.map_or(MouseTargetDetail::None, MouseTargetDetail::OverlayWidget),
                ),
                EmbeddedKind::ViewZone | EmbeddedKind::ViewZoneMargin => {
                    let kind = if *kind == EmbeddedKind::ViewZone {
                        MouseTargetKind::ContentViewZone
                    } else {
                        MouseTargetKind::GutterViewZone
                    };
                    let (position, detail) = self.zone_target(id, Some(*bounds), point, cx);
                    return target(kind, position, detail);
                }
                EmbeddedKind::GlyphMarginWidget => {
                    let row = self.position_for_point(point, window, cx).row;
                    (
                        MouseTargetKind::GutterGlyphMargin,
                        margin(false, id, Some(row)),
                    )
                }
            };
            return target(kind, None, detail);
        }
        if layout
            .minimap
            .is_some_and(|m| m.interactive && m.bounds.contains(&point))
        {
            return target(MouseTargetKind::Minimap, None, MouseTargetDetail::None);
        }
        if layout
            .vertical_scrollbar
            .is_some_and(|s| s.bar.contains(&point))
        {
            return target(
                MouseTargetKind::OverviewRuler,
                None,
                MouseTargetDetail::None,
            );
        }
        if layout
            .horizontal_scrollbar
            .is_some_and(|s| s.bar.contains(&point))
        {
            return target(MouseTargetKind::Scrollbar, None, MouseTargetDetail::None);
        }
        if let Some(line) = layout
            .sticky_lines
            .iter()
            .find(|l| l.bounds.contains(&point))
        {
            return target(
                MouseTargetKind::StickyScroll,
                Some(PointUtf16::new(line.row, 0)),
                MouseTargetDetail::None,
            );
        }
        let line = layout.lines.iter().find(|line| {
            let top = layout.line_top(line.view_line);
            top <= point.y && point.y < top + layout.line_height_of(line.view_line)
        });
        let in_gutter = point.x < layout.text_bounds.left();
        let Some(line) = line else {
            // Not on a line: a view zone, or below the last line.
            let below = layout.lines.last().is_some_and(|last| {
                point.y >= layout.line_top(last.view_line) + layout.line_height_of(last.view_line)
            });
            let position = below.then(|| self.position_for_point(point, window, cx));
            if below {
                let detail = if in_gutter {
                    margin(true, None, position.map(|position| position.row))
                } else {
                    MouseTargetDetail::ContentEmpty {
                        is_after_lines: true,
                        horizontal_distance_to_text: None,
                    }
                };
                return target(
                    MouseTargetKind::ContentEmpty { after_lines: true },
                    position,
                    detail,
                );
            }
            // The zone at this height, gutter side included.
            let zone = hits
                .filter(|((bounds, kind), _)| {
                    matches!(kind, EmbeddedKind::ViewZone | EmbeddedKind::ViewZoneMargin)
                        && bounds.top() <= point.y
                        && point.y < bounds.bottom()
                })
                .find_map(|((bounds, kind), view)| {
                    Some((self.widget_for_view(*kind, *view)?, *bounds))
                });
            let (position, detail) =
                self.zone_target(zone.map(|z| z.0), zone.map(|z| z.1), point, cx);
            let kind = if in_gutter {
                MouseTargetKind::GutterViewZone
            } else {
                MouseTargetKind::ContentViewZone
            };
            return target(kind, position, detail);
        };
        if in_gutter {
            let x = point.x - layout.bounds.left();
            let kind = if x < layout.glyph_margin_width {
                MouseTargetKind::GutterGlyphMargin
            } else if x < layout.glyph_margin_width + layout.line_numbers_width {
                MouseTargetKind::GutterLineNumbers
            } else {
                MouseTargetKind::GutterLineDecorations
            };
            let start = PointUtf16::new(line.row, line.columns.start);
            let mut target = target(kind, Some(start), margin(false, None, Some(line.row)));
            // Monaco: the whole line.
            let end = self.model().read(cx).line_len_utf16(line.row);
            target.range = Some(PointUtf16::new(line.row, 0)..PointUtf16::new(line.row, end));
            return target;
        }
        let position = self.position_for_point(point, window, cx);
        let end_x = layout.text_origin_x() + line.width();
        if point.x > end_x {
            target(
                MouseTargetKind::ContentEmpty { after_lines: false },
                Some(position),
                MouseTargetDetail::ContentEmpty {
                    is_after_lines: false,
                    horizontal_distance_to_text: Some(point.x - end_x),
                },
            )
        } else {
            let mut target = target(
                MouseTargetKind::ContentText,
                Some(position),
                MouseTargetDetail::None,
            );
            // Monaco `getInjectedTextAt`: strictly inside injected text.
            let x = point.x - layout.text_origin_x() - line.indent_x;
            let index = line.shaped.closest_index_for_x(x);
            let between = |range: &Range<usize>| {
                let (from, to) = (
                    line.shaped.x_for_index(range.start),
                    line.shaped.x_for_index(range.end),
                );
                from <= x && x < to && range.start <= index && index <= range.end
            };
            target.injected_text = line
                .injected
                .iter()
                .find(|injected| !injected.chip && between(&injected.range))
                .map(|injected| InjectedTextTarget {
                    decoration: injected.decoration,
                    attached_data: injected.attached_data.clone(),
                });
            target
        }
    }
}
