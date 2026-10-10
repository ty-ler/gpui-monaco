//! Mouse hit testing (Monaco's `IMouseTarget` and `MouseTargetFactory`)
//! and the mouse events hosts subscribe to (`onMouseDown`, `onMouseUp`,
//! `onMouseMove`, `onMouseLeave`, `onContextMenu`), e.g. to toggle
//! breakpoints in the glyph margin.

use gpui::{App, Modifiers, MouseButton, Pixels, Point, Window};
use text::PointUtf16;

use crate::editor::CodeEditor;

/// What an embedded view's bounds belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedKind {
    /// A view zone between lines.
    ViewZone,
    /// A content widget.
    ContentWidget,
    /// An overlay widget.
    OverlayWidget,
}

/// The part of the editor under the mouse (Monaco `MouseTargetType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseTargetKind {
    /// The glyph margin, where breakpoint-style glyphs go.
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

/// What the mouse is over (Monaco `IMouseTarget`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseTarget {
    /// The kind of target.
    pub kind: MouseTargetKind,
    /// The text position: the nearest one in text, the line start in the
    /// gutter, the header's row in sticky scroll; `None` elsewhere.
    pub position: Option<PointUtf16>,
    /// The mouse position in window coordinates.
    pub point: Point<Pixels>,
}

/// A mouse event over the editor (Monaco `IEditorMouseEvent`).
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// What's under `point` (window coordinates) as of the last frame
    /// (Monaco's mouse target).
    pub fn mouse_target(&self, point: Point<Pixels>, window: &mut Window, cx: &App) -> MouseTarget {
        let target = |kind, position| MouseTarget {
            kind,
            position,
            point,
        };
        let Some(layout) = self.layout.as_ref() else {
            return target(MouseTargetKind::Unknown, None);
        };
        if !layout.bounds.contains(&point) {
            return target(MouseTargetKind::Unknown, None);
        }
        // Widgets first, top-most (last painted) first.
        for (bounds, kind) in layout.widget_bounds.iter().zip(&layout.widget_kinds).rev() {
            if !bounds.contains(&point) {
                continue;
            }
            let kind = match kind {
                EmbeddedKind::ContentWidget => MouseTargetKind::ContentWidget,
                EmbeddedKind::OverlayWidget => MouseTargetKind::OverlayWidget,
                EmbeddedKind::ViewZone => MouseTargetKind::ContentViewZone,
            };
            return target(kind, None);
        }
        if layout.minimap.is_some_and(|m| m.bounds.contains(&point)) {
            return target(MouseTargetKind::Minimap, None);
        }
        if layout
            .vertical_scrollbar
            .is_some_and(|s| s.bar.contains(&point))
        {
            return target(MouseTargetKind::OverviewRuler, None);
        }
        if layout
            .horizontal_scrollbar
            .is_some_and(|s| s.bar.contains(&point))
        {
            return target(MouseTargetKind::Scrollbar, None);
        }
        if let Some(line) = layout
            .sticky_lines
            .iter()
            .find(|l| l.bounds.contains(&point))
        {
            return target(
                MouseTargetKind::StickyScroll,
                Some(PointUtf16::new(line.row, 0)),
            );
        }
        let line = layout.lines.iter().find(|line| {
            let top = layout.line_top(line.view_line);
            top <= point.y && point.y < top + layout.metrics.line_height
        });
        let in_gutter = point.x < layout.text_bounds.left();
        let Some(line) = line else {
            // Not on a line: a view zone, or below the last line.
            let below = layout.lines.last().is_some_and(|last| {
                point.y >= layout.line_top(last.view_line) + layout.metrics.line_height
            });
            let kind = match (below, in_gutter) {
                (true, _) => MouseTargetKind::ContentEmpty { after_lines: true },
                (false, true) => MouseTargetKind::GutterViewZone,
                (false, false) => MouseTargetKind::ContentViewZone,
            };
            return target(
                kind,
                below.then(|| self.position_for_point(point, window, cx)),
            );
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
            return target(kind, Some(PointUtf16::new(line.row, line.columns.start)));
        }
        let position = self.position_for_point(point, window, cx);
        let end_x = layout.text_origin_x() + line.width();
        let kind = if point.x > end_x {
            MouseTargetKind::ContentEmpty { after_lines: false }
        } else {
            MouseTargetKind::ContentText
        };
        target(kind, Some(position))
    }
}
