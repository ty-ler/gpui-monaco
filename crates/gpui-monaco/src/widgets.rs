//! Host views embedded in the editor (Monaco view zones, content widgets and
//! overlay widgets).

use gpui::{AnyView, Pixels, Point};
use text::{Anchor, PointUtf16};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId(pub u64);

/// Height of a view zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneHeight {
    Lines(f32),
    Pixels(Pixels),
}

/// A host view inserted between lines, pushing text down (Monaco
/// `IViewZone`).
#[derive(Clone)]
pub struct ViewZone {
    /// Monaco `afterLineNumber`: the zone shows after this 1-based line; `0`
    /// puts it above the first line. It follows edits.
    pub after_line: u32,
    pub height: ZoneHeight,
    pub view: AnyView,
}

/// Where a content widget prefers to show relative to its position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentWidgetPreference {
    Above,
    Below,
    Exact,
}

/// A host view anchored to a text position (Monaco `IContentWidget`).
#[derive(Clone)]
pub struct ContentWidget {
    pub position: PointUtf16,
    /// Tried in order; the first that fits in the editor is used.
    pub preference: Vec<ContentWidgetPreference>,
    pub view: AnyView,
}

/// Where an overlay widget is pinned (Monaco
/// `OverlayWidgetPositionPreference`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayPosition {
    TopRight,
    BottomRight,
    TopCenter,
    /// Offset from the editor's top-left corner.
    Custom(Point<Pixels>),
}

/// A host view pinned to the editor (Monaco `IOverlayWidget`).
#[derive(Clone)]
pub struct OverlayWidget {
    pub position: OverlayPosition,
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
