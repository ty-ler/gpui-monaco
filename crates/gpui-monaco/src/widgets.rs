//! Host views embedded in the editor (Monaco view zones, content widgets and
//! overlay widgets).

use gpui::{AnyView, Pixels, Point};
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
}

/// Stored view zone: `after_line` tracked with an anchor.
#[derive(Clone)]
pub(crate) struct ZoneState {
    pub id: WidgetId,
    /// Anchor at the start of the line after the zone; `None` for line 0.
    pub anchor: Option<Anchor>,
    pub height: ZoneHeight,
    pub view: AnyView,
}

#[derive(Clone)]
pub(crate) struct ContentWidgetState {
    pub id: WidgetId,
    pub anchor: Anchor,
    pub preference: Vec<ContentWidgetPreference>,
    pub view: AnyView,
}

#[derive(Clone)]
pub(crate) struct OverlayWidgetState {
    pub id: WidgetId,
    pub position: OverlayPosition,
    pub view: AnyView,
}

/// All embedded views of one editor.
#[derive(Clone, Default)]
pub(crate) struct Widgets {
    pub zones: Vec<ZoneState>,
    pub content: Vec<ContentWidgetState>,
    pub overlays: Vec<OverlayWidgetState>,
    next_id: u64,
}

impl Widgets {
    pub fn next_id(&mut self) -> WidgetId {
        self.next_id += 1;
        WidgetId(self.next_id)
    }

    pub fn remove(&mut self, id: WidgetId) -> bool {
        let before = self.zones.len() + self.content.len() + self.overlays.len();
        self.zones.retain(|z| z.id != id);
        self.content.retain(|w| w.id != id);
        self.overlays.retain(|w| w.id != id);
        before != self.zones.len() + self.content.len() + self.overlays.len()
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
}
