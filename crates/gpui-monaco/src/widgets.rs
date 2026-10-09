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
    /// Tried in order; the first that fits in the editor is used.
    pub preference: Vec<ContentWidgetPreference>,
    /// The view to render, laid out at its min-content size.
    pub view: AnyView,
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
