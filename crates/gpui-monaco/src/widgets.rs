//! Host views embedded in the editor (Monaco view zones, content widgets and
//! overlay widgets).

use std::cell::Cell;
use std::rc::Rc;

use gpui::{AnyView, Pixels, Point, px};
use text::{Anchor, PointUtf16};

/// Identifies a view zone, content widget or overlay widget added to a
/// [`CodeEditor`](crate::CodeEditor), e.g. for
/// [`CodeEditor::remove_widget`](crate::CodeEditor::remove_widget).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId(pub u64);

/// Height of a view zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneHeight {
    /// A multiple of the editor's line height.
    Lines(f32),
    /// A fixed height in pixels.
    Pixels(Pixels),
}

/// A host view inserted between lines, pushing text down (Monaco
/// `IViewZone`).
#[derive(Clone)]
pub struct ViewZone {
    /// Monaco `afterLineNumber`: the zone shows after this 1-based line; `0`
    /// puts it above the first line. It follows edits.
    pub after_line: u32,
    /// The zone's height.
    pub height: ZoneHeight,
    /// The view to render, laid out at the zone's size.
    pub view: AnyView,
}

/// Where a content widget prefers to show relative to its position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentWidgetPreference {
    /// Above the anchor's line, ending at its top.
    Above,
    /// Below the anchor's line, starting at its bottom.
    Below,
    /// With its top at the anchor's line top.
    Exact,
}

/// A host view anchored to a text position (Monaco `IContentWidget`).
#[derive(Clone)]
pub struct ContentWidget {
    /// The anchor (0-based row, UTF-16 column); it follows edits.
    pub position: PointUtf16,
    /// Tried in order; the first that fits in the editor is used, else the
    /// first. An empty list hides the widget, as in Monaco.
    pub preference: Vec<ContentWidgetPreference>,
    /// The view to render, laid out at its min-content size.
    pub view: AnyView,
    /// Monaco `allowEditorOverflow`: the widget may extend past the
    /// editor (into the rest of the window), is painted above everything
    /// else, and fits above or below its line against the window rather
    /// than the editor. Otherwise it's clipped to the text area.
    pub allow_editor_overflow: bool,
}

/// Monaco's clearance from the window's top and bottom edges for
/// overflowing content widgets (`_layoutBoxInPage`'s `TOP_PADDING` and
/// `BOTTOM_PADDING`).
pub(crate) const OVERFLOW_VERTICAL_PADDING: Pixels = px(22.);
/// Monaco's clearance from the window's left and right edges
/// (`_layoutHorizontalSegmentInPage`'s `LEFT_PADDING` and `RIGHT_PADDING`).
pub(crate) const OVERFLOW_HORIZONTAL_PADDING: Pixels = px(15.);

/// The shared card shape of Monaco's floating widgets (hover, parameter
/// hints, suggest list and details): VS Code's size and elevation tokens.
pub(crate) mod card {
    use gpui::{BoxShadow, Hsla, Pixels, hsla, point, px};

    /// `--vscode-cornerRadius-large` (`cornerRadius.large`, 8px).
    pub const CORNER_RADIUS: Pixels = px(8.);

    /// `--vscode-shadow-lg` (`0 0 12px rgba(0, 0, 0, 0.14)`), on the hover
    /// and parameter hints in every theme.
    pub fn shadow_lg() -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: hsla(0., 0., 0., 0.14),
            offset: point(px(0.), px(0.)),
            blur_radius: px(12.),
            spread_radius: px(0.),
            inset: false,
        }]
    }

    /// The suggest widget's `box-shadow: 0 2px 8px var(--vscode-widget-shadow)`;
    /// none when the theme has no `widget.shadow` (high contrast).
    pub fn widget_shadow(shadow: Option<Hsla>) -> Vec<BoxShadow> {
        shadow
            .map(|color| BoxShadow {
                color,
                offset: point(px(0.), px(2.)),
                blur_radius: px(8.),
                spread_radius: px(0.),
                inset: false,
            })
            .into_iter()
            .collect()
    }
}

/// Paint order of overflowing widgets, lowest first (Monaco's CSS
/// `z-index`es). They all paint above the editor and its overlay widgets.
pub(crate) mod layer {
    /// Host widgets.
    pub const DEFAULT: usize = 0;
    /// Inline messages (the read-only message).
    pub const MESSAGE: usize = 10;
    /// Rename, the post-edit widget, inspect tokens, the color picker and
    /// the inline suggestion toolbar.
    pub const WIDGET: usize = 20;
    /// Parameter hints (`z-index: 39`).
    pub const PARAMETER_HINTS: usize = 39;
    /// The suggest widget (`z-index: 40`).
    pub const SUGGEST: usize = 40;
    /// The hover (`z-index: 50`).
    pub const HOVER: usize = 50;
    /// Menus in Monaco's context view: the context menu and the code
    /// action list.
    pub const MENU: usize = 100;
}

/// The room an overflowing content widget has in the window around its
/// line, for widgets that size themselves to it (Monaco's hover and
/// suggest widget measure the space above and below the cursor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverflowRoom {
    /// From the window's top (less Monaco's padding) to the line's top.
    pub above: Pixels,
    /// From the line's bottom to the window's bottom (less the padding).
    pub below: Pixels,
    /// The window's width less the padding on both sides.
    pub width: Pixels,
    /// From the anchor to the window's right edge (less the padding).
    pub right: Pixels,
}

impl OverflowRoom {
    /// The tallest the widget can be and still fit on one side.
    pub fn max_height(&self) -> Pixels {
        self.above.max(self.below).max(px(0.))
    }
}

/// Where the editor publishes a widget's [`OverflowRoom`] before laying it
/// out each frame.
#[derive(Clone, Default)]
pub(crate) struct OverflowRoomCell(Rc<Cell<Option<OverflowRoom>>>);

impl OverflowRoomCell {
    pub fn get(&self) -> Option<OverflowRoom> {
        self.0.get()
    }

    pub fn set(&self, room: OverflowRoom) {
        self.0.set(Some(room));
    }

    /// `max` capped to the tallest the widget can be in the window (but at
    /// least `min`).
    pub fn cap_height(&self, max: Pixels, min: Pixels) -> Pixels {
        match self.get() {
            Some(room) => max.min(room.max_height()).max(min),
            None => max,
        }
    }

    /// `max` capped to the window's width (but at least `min`).
    pub fn cap_width(&self, max: Pixels, min: Pixels) -> Pixels {
        match self.get() {
            Some(room) => max.min(room.width).max(min),
            None => max,
        }
    }
}

/// The top of a content widget `height` tall anchored to the line at
/// `line_top`, within the editor's `viewport` (Monaco
/// `ContentWidget._prepareRenderWidget`): the first preference that fits,
/// else the first preference. `Exact` always fits; an empty list hides the
/// widget.
pub(crate) fn place_content_widget(
    preference: &[ContentWidgetPreference],
    line_top: Pixels,
    line_height: Pixels,
    height: Pixels,
    viewport: std::ops::Range<Pixels>,
) -> Option<Pixels> {
    let above = line_top - height;
    let below = line_top + line_height;
    let fits_above = above >= viewport.start;
    let fits_below = below + height <= viewport.end;
    let fitting = preference.iter().find(|p| match p {
        ContentWidgetPreference::Above => fits_above,
        ContentWidgetPreference::Below => fits_below,
        ContentWidgetPreference::Exact => true,
    });
    Some(match fitting.or(preference.first())? {
        ContentWidgetPreference::Above => above,
        ContentWidgetPreference::Below => below,
        ContentWidgetPreference::Exact => line_top,
    })
}

/// Where an overlay widget is pinned (Monaco
/// `OverlayWidgetPositionPreference`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayPosition {
    /// Top right, aligned to the right edge of the text area.
    TopRight,
    /// Bottom right, aligned to the right edge of the text area.
    BottomRight,
    /// Top, centered over the text area.
    TopCenter,
    /// Offset from the editor's top-left corner.
    Custom(Point<Pixels>),
}

/// A host view pinned to the editor (Monaco `IOverlayWidget`).
#[derive(Clone)]
pub struct OverlayWidget {
    /// Where the widget is pinned.
    pub position: OverlayPosition,
    /// The view to render, laid out at its min-content size.
    pub view: AnyView,
    /// Monaco `allowEditorOverflow`: the widget may extend past the editor
    /// (it's kept inside the window) and is painted above everything else.
    /// Otherwise it's clipped to the editor.
    pub allow_editor_overflow: bool,
}

/// A host view in the glyph margin beside a line (Monaco
/// `IGlyphMarginWidget`), such as a breakpoint or a revert button.
#[derive(Clone)]
pub struct GlyphMarginWidget {
    /// The 0-based line; it follows edits.
    pub row: u32,
    /// The view to render, laid out at the glyph margin's width and the
    /// line's height.
    pub view: AnyView,
}

#[derive(Clone)]
pub(crate) struct GlyphWidgetState {
    pub id: WidgetId,
    /// The line's start.
    pub anchor: Anchor,
    pub view: AnyView,
}

/// Stored view zone: `after_line` tracked with an anchor.
#[derive(Clone)]
pub(crate) struct ZoneState {
    pub id: WidgetId,
    /// Anchor at the start of the line after the zone; `None` for line 0.
    pub anchor: Option<Anchor>,
    pub height: ZoneHeight,
    pub view: AnyView,
    /// Spans the gutter too, like Monaco's zone widgets (peek views).
    pub full_width: bool,
    /// The wheel over the zone doesn't scroll the editor: Monaco's zone
    /// widgets (peek, the problem zone) are overlay widgets, outside its
    /// scrollable element, while plain view zones scroll it.
    pub captures_wheel: bool,
    /// Monaco `IViewZone.minWidthInPx`: the zone widens the horizontal
    /// scroll range to at least this (the diff editor's inline deleted
    /// lines).
    pub min_width: Pixels,
}

#[derive(Clone)]
pub(crate) struct ContentWidgetState {
    pub id: WidgetId,
    pub anchor: Anchor,
    pub preference: Vec<ContentWidgetPreference>,
    pub view: AnyView,
    pub allow_overflow: bool,
    /// Paint order among overflowing widgets (a [`layer`]).
    pub layer: usize,
    /// Where the editor publishes the room the widget has, when it
    /// overflows.
    pub room: OverflowRoomCell,
}

#[derive(Clone)]
pub(crate) struct OverlayWidgetState {
    pub id: WidgetId,
    pub position: OverlayPosition,
    pub view: AnyView,
    pub allow_overflow: bool,
    /// Paint order among overflowing widgets (a [`layer`]).
    pub layer: usize,
}

/// What cached zone rows were computed for: the view lines (pointer), the
/// text version and the zones generation.
pub(crate) type ZoneRowsKey = (usize, text::Version, u64);

/// All embedded views of one editor.
#[derive(Clone, Default)]
pub(crate) struct Widgets {
    pub zones: Vec<ZoneState>,
    pub content: Vec<ContentWidgetState>,
    pub overlays: Vec<OverlayWidgetState>,
    pub glyphs: Vec<GlyphWidgetState>,
    next_id: u64,
    /// Bumped when zones are added or removed.
    pub zones_generation: u64,
    /// Each zone's view line (`None` when folded away), cached for
    /// (`view_lines`, text version, `zones_generation`): editors can have
    /// thousands of zones (code lenses).
    pub zone_rows: Option<(ZoneRowsKey, Vec<Option<i64>>)>,
}

impl Widgets {
    pub fn next_id(&mut self) -> WidgetId {
        self.next_id += 1;
        WidgetId(self.next_id)
    }

    /// Sets an overflowing widget's paint order (a [`layer`]).
    pub fn set_layer(&mut self, id: WidgetId, layer: usize) {
        if let Some(widget) = self.content.iter_mut().find(|w| w.id == id) {
            widget.layer = layer;
        }
        if let Some(widget) = self.overlays.iter_mut().find(|w| w.id == id) {
            widget.layer = layer;
        }
    }

    /// Where a content widget learns the room it has in the window.
    pub fn room(&self, id: WidgetId) -> OverflowRoomCell {
        self.content
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.room.clone())
            .unwrap_or_default()
    }

    /// Makes the wheel over zone `id` leave the editor alone.
    pub fn set_zone_captures_wheel(&mut self, id: WidgetId) {
        if let Some(zone) = self.zones.iter_mut().find(|z| z.id == id) {
            zone.captures_wheel = true;
        }
    }

    pub fn remove(&mut self, id: WidgetId) -> bool {
        let before =
            self.zones.len() + self.content.len() + self.overlays.len() + self.glyphs.len();
        let zones = self.zones.len();
        self.zones.retain(|z| z.id != id);
        if self.zones.len() != zones {
            self.zones_generation += 1;
        }
        self.content.retain(|w| w.id != id);
        self.overlays.retain(|w| w.id != id);
        self.glyphs.retain(|w| w.id != id);
        before != self.zones.len() + self.content.len() + self.overlays.len() + self.glyphs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::ContentWidgetPreference::{Above, Below, Exact};
    use super::*;
    use gpui::px;

    fn place(preference: &[ContentWidgetPreference], line_top: f32) -> Option<Pixels> {
        // 20px lines, a 50px widget, a 0..200 viewport.
        place_content_widget(preference, px(line_top), px(20.), px(50.), px(0.)..px(200.))
    }

    #[test]
    fn content_widget_placement() {
        // The first preference that fits.
        assert_eq!(place(&[Above, Below], 100.), Some(px(50.)));
        assert_eq!(place(&[Above, Below], 10.), Some(px(30.)));
        assert_eq!(place(&[Below, Above], 160.), Some(px(110.)));
        // Nothing fits: the first preference, not always below.
        let tight =
            place_content_widget(&[Above, Below], px(30.), px(20.), px(50.), px(0.)..px(90.));
        assert_eq!(tight, Some(px(-20.)));
        // Exact always fits.
        assert_eq!(place(&[Exact, Below], 0.), Some(px(0.)));
        // No preference: hidden.
        assert_eq!(place(&[], 100.), None);
    }

    #[gpui::test]
    fn card_shape_tokens() {
        use crate::theme::Theme;
        assert_eq!(card::CORNER_RADIUS, px(8.));
        let lg = card::shadow_lg();
        assert_eq!(lg.len(), 1);
        assert_eq!(lg[0].blur_radius, px(12.));
        assert_eq!(lg[0].color.a, 0.14);
        // The suggest widget's shadow follows `widget.shadow`: none in high
        // contrast.
        let dark = card::widget_shadow(Theme::vs_dark().colors.widget_shadow);
        assert_eq!(dark.len(), 1);
        assert_eq!(dark[0].offset, gpui::point(px(0.), px(2.)));
        assert_eq!(dark[0].blur_radius, px(8.));
        assert!(card::widget_shadow(Theme::hc_black().colors.widget_shadow).is_empty());
    }
}
