//! Host views embedded in the editor (Monaco view zones, content widgets and
//! overlay widgets).

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{AnyView, App, Pixels, Point, Size, Window, px};
use text::{Anchor, PointUtf16};

/// A view zone callback given a length in pixels (Monaco `IViewZone`'s
/// `onDomNodeTop` and `onComputedHeight`).
pub type ZoneCallback = Rc<dyn Fn(Pixels, &mut Window, &mut App)>;

/// Monaco `IContentWidget.beforeRender`: the size to place the widget by,
/// or `None` to measure its view.
pub type BeforeRender = Rc<dyn Fn(&mut Window, &mut App) -> Option<Size<Pixels>>>;

/// Monaco `IContentWidget.afterRender`: the preference the widget was
/// placed by, or `None` when it isn't shown.
pub type AfterRender = Rc<dyn Fn(Option<ContentWidgetPreference>, &mut Window, &mut App)>;

/// Identifies a view zone, content widget or overlay widget added to a
/// [`CodeEditor`](crate::CodeEditor), e.g. for
/// [`CodeEditor::remove_widget`](crate::CodeEditor::remove_widget).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId(pub u64);

/// Height of a view zone (Monaco `heightInLines` or `heightInPx`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneHeight {
    /// A multiple of the editor's line height (Monaco `heightInLines`).
    Lines(f32),
    /// A fixed height in pixels (Monaco `heightInPx`).
    Pixels(Pixels),
}

/// Which view position a model position means where several show it
/// (Monaco `PositionAffinity`): at a wrap boundary, or next to injected
/// text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PositionAffinity {
    /// The end of the previous view line at a wrap boundary.
    Left,
    /// The start of the next view line at a wrap boundary.
    Right,
    /// Monaco's default: the next view line at a wrap boundary.
    #[default]
    None,
    /// Before injected text at the position; a content widget at a line's
    /// start is left-aligned to the line.
    LeftOfInjectedText,
    /// After injected text at the position.
    RightOfInjectedText,
}

impl PositionAffinity {
    /// Whether a point at a wrap boundary shows at the end of the previous
    /// view line (Monaco `ModelLineProjection` with a left affinity).
    pub(crate) fn prefers_previous(self) -> bool {
        matches!(self, Self::Left | Self::LeftOfInjectedText)
    }
}

/// A host view inserted between lines, pushing text down (Monaco
/// `IViewZone`). Make one with [`ViewZone::new`], then set the other fields
/// as needed.
#[derive(Clone)]
pub struct ViewZone {
    /// Monaco `afterLineNumber`: the zone shows after this 1-based line; `0`
    /// puts it above the first line. It follows edits.
    pub after_line: u32,
    /// Monaco `afterColumn`, as a 0-based UTF-16 column: the zone shows
    /// after the view line (of a wrapped line) showing this column of
    /// `after_line`. `None`: after the line's last view line.
    pub after_column: Option<u32>,
    /// Monaco `afterColumnAffinity`: which view line `after_column` means
    /// at a wrap boundary.
    pub after_column_affinity: PositionAffinity,
    /// Monaco `showInHiddenAreas`: shows the zone even when the line after
    /// it is folded away (after the fold's first line).
    pub show_in_hidden_areas: bool,
    /// Monaco `ordinal`: orders zones after the same view line, lowest
    /// first, ties in the order they were added. Defaults to
    /// `after_column + 1` (Monaco's 1-based `afterColumn`), else 10000.
    pub ordinal: Option<i32>,
    /// Monaco `suppressMouseDown`: a mouse down on the zone focuses the
    /// editor and places the cursor before or after the zone (the half
    /// clicked), as on text. Otherwise the zone's view has the click.
    pub suppress_mouse_down: bool,
    /// The zone's height.
    pub height: ZoneHeight,
    /// Monaco `minWidthInPx`: the zone widens the horizontal scroll range to
    /// at least this.
    pub min_width: Pixels,
    /// The view to render beside the text, laid out at the zone's height
    /// and the scroll width: it scrolls horizontally with the text.
    pub view: AnyView,
    /// Monaco `marginDomNode`: a view for the gutter beside the zone, laid
    /// out at the gutter's width and the zone's height. It doesn't scroll
    /// horizontally.
    pub margin_view: Option<AnyView>,
    /// Monaco `onDomNodeTop`: told the zone's top relative to the editor's
    /// top, scrolling included, when the zone is laid out with a new top.
    /// A zone out of view or hidden is at Monaco's -1000000 less the scroll
    /// top.
    pub on_dom_node_top: Option<ZoneCallback>,
    /// Monaco `onComputedHeight`: told the zone's height in pixels (0 while
    /// it's hidden) when the zone is laid out with a new height.
    pub on_computed_height: Option<ZoneCallback>,
}

impl ViewZone {
    /// A zone after 1-based `after_line` (`0`: above the first line), with
    /// Monaco's defaults for everything else.
    pub fn new(after_line: u32, height: ZoneHeight, view: AnyView) -> Self {
        Self {
            after_line,
            after_column: None,
            after_column_affinity: PositionAffinity::None,
            show_in_hidden_areas: false,
            ordinal: None,
            suppress_mouse_down: false,
            height,
            min_width: px(0.),
            view,
            margin_view: None,
            on_dom_node_top: None,
            on_computed_height: None,
        }
    }

    /// Monaco `ViewZones._getZoneOrdinal`.
    pub(crate) fn effective_ordinal(&self) -> i64 {
        zone_ordinal(self.ordinal, self.after_column)
    }
}

/// Monaco `ViewZones._getZoneOrdinal`: `ordinal`, else the 1-based
/// `afterColumn`, else 10000.
fn zone_ordinal(ordinal: Option<i32>, after_column: Option<u32>) -> i64 {
    ordinal
        .map(i64::from)
        .unwrap_or_else(|| after_column.map_or(10_000, |column| i64::from(column) + 1))
}

/// Where a content widget prefers to show relative to its position (Monaco
/// `ContentWidgetPositionPreference`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentWidgetPreference {
    /// Above the anchor's line, ending at its top.
    Above,
    /// Below the anchor's line, starting at its bottom.
    Below,
    /// With its top at the anchor's line top.
    Exact,
}

/// A host view anchored to a text position (Monaco `IContentWidget`). Make
/// one with [`ContentWidget::new`], then set the other fields as needed.
#[derive(Clone)]
pub struct ContentWidget {
    /// The anchor (0-based row, UTF-16 column); it follows edits.
    pub position: PointUtf16,
    /// Monaco `secondaryPosition`: a position on the same line the widget
    /// should also touch if it can. It's shifted toward it, keeping a
    /// full-width character's width over `position`.
    pub secondary_position: Option<PointUtf16>,
    /// Tried in order; the first that fits in the editor is used, else the
    /// first. An empty list hides the widget, as in Monaco.
    pub preference: Vec<ContentWidgetPreference>,
    /// Monaco `positionAffinity`: which view position `position` means at a
    /// wrap boundary or next to injected text.
    pub position_affinity: Option<PositionAffinity>,
    /// The view to render, laid out at its min-content size, at most as
    /// wide as the text area (the window with `allow_editor_overflow`), as
    /// Monaco's `maxWidth`.
    pub view: AnyView,
    /// Monaco `allowEditorOverflow`: the widget may extend past the
    /// editor (into the rest of the window), is painted above everything
    /// else, and fits above or below its line against the window rather
    /// than the editor. Otherwise it's clipped to the text area.
    pub allow_editor_overflow: bool,
    /// Monaco `suppressMouseDown`: a mouse down on the widget focuses the
    /// editor rather than the widget.
    pub suppress_mouse_down: bool,
    /// Monaco `useDisplayNone`: the widget has no visual representation.
    /// Its view is never laid out or painted and takes no mouse; it's
    /// still placed (by `before_render`'s size, else as an empty box) and
    /// `after_render` is told where.
    pub use_display_none: bool,
    /// Monaco `beforeRender`: called before the widget is placed, each
    /// frame its line is in view. A size it returns is used instead of the
    /// view's measured size, to place the widget and lay its view out.
    pub before_render: Option<BeforeRender>,
    /// Monaco `afterRender`: told the preference the widget was placed by,
    /// or `None` when it isn't shown (no position or preference, or its
    /// line out of view). Called after the frame, when that changes.
    pub after_render: Option<AfterRender>,
}

impl ContentWidget {
    /// A widget at `position`, placed by `preference`, with Monaco's
    /// defaults for everything else.
    pub fn new(
        position: PointUtf16,
        preference: Vec<ContentWidgetPreference>,
        view: AnyView,
    ) -> Self {
        Self {
            position,
            secondary_position: None,
            preference,
            position_affinity: None,
            view,
            allow_editor_overflow: false,
            suppress_mouse_down: false,
            use_display_none: false,
            before_render: None,
            after_render: None,
        }
    }

    /// The render hooks to keep with the widget, if it has any.
    pub(crate) fn hooks(&self) -> Option<Rc<ContentWidgetHooks>> {
        (self.before_render.is_some() || self.after_render.is_some()).then(|| {
            Rc::new(ContentWidgetHooks {
                before_render: self.before_render.clone(),
                after_render: self.after_render.clone(),
                reported: Cell::new(None),
            })
        })
    }
}

/// A content widget's `beforeRender` and `afterRender`.
pub(crate) struct ContentWidgetHooks {
    before_render: Option<BeforeRender>,
    after_render: Option<AfterRender>,
    /// What `after_render` was last told (`None`: nothing yet).
    reported: Cell<Option<Option<ContentWidgetPreference>>>,
}

impl ContentWidgetHooks {
    /// The size `before_render` asks for.
    pub fn size(&self, window: &mut Window, cx: &mut App) -> Option<Size<Pixels>> {
        self.before_render
            .as_ref()
            .and_then(|before| before(window, cx))
    }

    /// Queues `after_render` with where the widget went this frame, unless
    /// that's what it was told last.
    pub fn report(&self, placed: Option<ContentWidgetPreference>, pending: &mut PendingCallbacks) {
        if let Some(after) = &self.after_render
            && self.reported.get() != Some(placed)
        {
            self.reported.set(Some(placed));
            pending.push(PendingCallback::AfterRender(after.clone(), placed));
        }
    }
}

/// A host callback due after this frame. Monaco calls them while it
/// renders; here they run once the frame is drawn, so they may update the
/// editor.
pub(crate) enum PendingCallback {
    /// A view zone's `onDomNodeTop` or `onComputedHeight`.
    Zone(ZoneCallback, Pixels),
    /// A content widget's `afterRender`.
    AfterRender(AfterRender, Option<ContentWidgetPreference>),
}

/// The callbacks due after a frame (empty, and unallocated, without any).
pub(crate) type PendingCallbacks = Vec<PendingCallback>;

/// Runs `pending` once the current frame is drawn.
pub(crate) fn run_after_frame(pending: PendingCallbacks, window: &mut Window, cx: &mut App) {
    if pending.is_empty() {
        return;
    }
    window.defer(cx, move |window, cx| {
        for callback in pending {
            match callback {
                PendingCallback::Zone(callback, value) => callback(value, window, cx),
                PendingCallback::AfterRender(callback, placed) => callback(placed, window, cx),
            }
        }
    });
}

/// Where a content widget shows, for
/// [`CodeEditor::layout_content_widget`](crate::CodeEditor::layout_content_widget)
/// (Monaco `IContentWidgetPosition`, which `layoutContentWidget` re-reads).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentWidgetPosition {
    /// The anchor; `None` hides the widget (Monaco's `null` position).
    pub position: Option<PointUtf16>,
    /// Monaco `secondaryPosition`.
    pub secondary_position: Option<PointUtf16>,
    /// Monaco `preference`; empty hides the widget.
    pub preference: Vec<ContentWidgetPreference>,
    /// Monaco `positionAffinity`.
    pub position_affinity: Option<PositionAffinity>,
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

/// How an overflowing content widget asks to be placed, set while it
/// renders (the suggest widget, whose details panel sits beside or around
/// the list).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WidgetAnchor {
    /// The side of the line it goes on, instead of its preferences (Monaco's
    /// `SuggestContentWidget` gives a single preference).
    pub preference: ContentWidgetPreference,
    /// The part of the widget placed against the line, relative to the
    /// widget's top left; the rest lies around it.
    pub anchor_box: gpui::Bounds<Pixels>,
    /// Another part beside `anchor_box`, relative to the widget's top left
    /// (the suggest widget's details panel). With it, only the two parts
    /// take the mouse, and the rest of the widget's box lets it through to
    /// the editor (Monaco's suggest list and details are separate widgets).
    pub beside: Option<gpui::Bounds<Pixels>>,
}

#[derive(Default)]
struct OverflowRoomState {
    room: Cell<Option<OverflowRoom>>,
    anchor: Cell<Option<WidgetAnchor>>,
}

/// Where the editor publishes a widget's [`OverflowRoom`] before laying it
/// out each frame, and where the widget may ask for a [`WidgetAnchor`].
#[derive(Clone, Default)]
pub(crate) struct OverflowRoomCell(Rc<OverflowRoomState>);

impl OverflowRoomCell {
    pub fn get(&self) -> Option<OverflowRoom> {
        self.0.room.get()
    }

    pub fn set(&self, room: OverflowRoom) {
        self.0.room.set(Some(room));
    }

    /// How the widget asked to be placed this frame, if it did.
    pub fn anchor(&self) -> Option<WidgetAnchor> {
        self.0.anchor.get()
    }

    /// Places the widget by `anchor` (or by its preferences and whole box
    /// with `None`).
    pub fn set_anchor(&self, anchor: Option<WidgetAnchor>) {
        self.0.anchor.set(anchor);
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
/// `line_top`, within the editor's `viewport`, and the preference used
/// (Monaco `ContentWidget._prepareRenderWidget`): the first preference that
/// fits, else the first preference. `Exact` always fits; an empty list
/// hides the widget.
pub(crate) fn place_content_widget(
    preference: &[ContentWidgetPreference],
    line_top: Pixels,
    line_height: Pixels,
    height: Pixels,
    viewport: Range<Pixels>,
) -> Option<(Pixels, ContentWidgetPreference)> {
    let above = line_top - height;
    let below = line_top + line_height;
    let fits_above = above >= viewport.start;
    let fits_below = below + height <= viewport.end;
    let fitting = preference.iter().find(|p| match p {
        ContentWidgetPreference::Above => fits_above,
        ContentWidgetPreference::Below => fits_below,
        ContentWidgetPreference::Exact => true,
    });
    let used = *fitting.or(preference.first())?;
    let top = match used {
        ContentWidgetPreference::Above => above,
        ContentWidgetPreference::Below => below,
        ContentWidgetPreference::Exact => line_top,
    };
    Some((top, used))
}

/// Where an overlay widget is pinned (Monaco
/// `OverlayWidgetPositionPreference`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayPosition {
    /// Top right: left of the minimap and twice the vertical scrollbar's
    /// width from the editor's right edge, as Monaco's `TOP_RIGHT_CORNER`.
    TopRight,
    /// Bottom right: as `TopRight`, and twice the horizontal scrollbar's
    /// height above the editor's bottom (`BOTTOM_RIGHT_CORNER`).
    BottomRight,
    /// Top, centered over the text area.
    TopCenter,
    /// Offset from the editor's top-left corner (Monaco
    /// `IOverlayWidgetPositionCoordinates`).
    Custom(Point<Pixels>),
    /// Monaco's `null` preference: the widget places itself. It's laid out
    /// at the editor's top-left corner with the editor's size available.
    Manual,
}

/// A host view pinned to the editor (Monaco `IOverlayWidget`). Make one
/// with [`OverlayWidget::new`], then set the other fields as needed.
#[derive(Clone)]
pub struct OverlayWidget {
    /// Where the widget is pinned.
    pub position: OverlayPosition,
    /// Monaco `stackOrdinal`: widgets with one stack, lowest first, with
    /// the others at the same corner (top to bottom at the right corners
    /// and the top center); without one they overlap.
    pub stack_ordinal: Option<i32>,
    /// The view to render, laid out at its min-content size.
    pub view: AnyView,
    /// Monaco `allowEditorOverflow`: the widget may extend past the editor
    /// (it's kept inside the window) and is painted above everything else.
    /// Otherwise it's clipped to the editor.
    pub allow_editor_overflow: bool,
    /// Monaco `getMinContentWidthInPx`: the widget widens the horizontal
    /// scroll range to at least this.
    pub min_content_width: Pixels,
}

impl OverlayWidget {
    /// A widget pinned at `position`, with Monaco's defaults for everything
    /// else.
    pub fn new(position: OverlayPosition, view: AnyView) -> Self {
        Self {
            position,
            stack_ordinal: None,
            view,
            allow_editor_overflow: false,
            min_content_width: px(0.),
        }
    }
}

/// Monaco `OverlayWidgets._renderWidget`'s corners, laid out in stacking
/// order (Monaco sorts by `stackOrdinal`, missing ones as 0): each widget's
/// origin within `editor`, the text area being `text`. `sizes[i]` is
/// widget `i`'s measured size. Right-corner widgets keep `right_inset`
/// (twice the vertical scrollbar width plus the minimap's) from the
/// editor's right edge; bottom ones keep `bottom_inset` (twice the
/// horizontal scrollbar height) from its bottom.
pub(crate) fn place_overlay_widgets(
    widgets: &[(OverlayPosition, Option<i32>)],
    sizes: &[Size<Pixels>],
    editor: gpui::Bounds<Pixels>,
    text: gpui::Bounds<Pixels>,
    right_inset: Pixels,
    bottom_inset: Pixels,
) -> Vec<Point<Pixels>> {
    let mut order: Vec<usize> = (0..widgets.len()).collect();
    order.sort_by_key(|&ix| widgets[ix].1.unwrap_or(0));
    // Where the next stacked widget goes, per corner (Monaco
    // `stackCoordinates`): top right, bottom right, top center.
    let mut stack = [px(0.); 3];
    let mut origins = vec![editor.origin; widgets.len()];
    let right = editor.right() - right_inset;
    for ix in order {
        let (position, stack_ordinal) = widgets[ix];
        let size = sizes[ix];
        let stacked = stack_ordinal.is_some();
        origins[ix] = match position {
            OverlayPosition::TopRight => {
                let top = if stacked { stack[0] } else { px(0.) };
                if stacked {
                    stack[0] += size.height;
                }
                gpui::point(right - size.width, editor.top() + top)
            }
            OverlayPosition::BottomRight => {
                let mut top = editor.size.height - size.height - bottom_inset;
                if stacked {
                    top -= stack[1];
                    stack[1] += size.height;
                }
                gpui::point(right - size.width, editor.top() + top)
            }
            OverlayPosition::TopCenter => {
                let top = if stacked { stack[2] } else { px(0.) };
                if stacked {
                    stack[2] += size.height;
                }
                gpui::point(
                    text.left() + (text.size.width - size.width) / 2.,
                    editor.top() + top,
                )
            }
            OverlayPosition::Custom(offset) => editor.origin + offset,
            OverlayPosition::Manual => editor.origin,
        };
    }
    origins
}

/// A host view in the glyph margin beside a range's lines (Monaco
/// `IGlyphMarginWidget`), such as a breakpoint or a revert button.
#[derive(Clone)]
pub struct GlyphMarginWidget {
    /// The range it's for (Monaco `IGlyphMarginWidgetPosition.range`); it
    /// follows edits. Monaco shows the widget once, on the range's first
    /// line, or on the first line in view while the range starts above
    /// the viewport (`_collectWidgetBasedGlyphRenderRequest`); its lane is
    /// taken on every line of the range.
    pub range: Range<PointUtf16>,
    /// The view to render, laid out a lane wide (the line height) and the
    /// line's height.
    pub view: AnyView,
    /// Its glyph margin lane (Monaco `IGlyphMarginWidgetPosition.lane`).
    pub lane: crate::decorations::GlyphMarginLane,
    /// Where widgets and glyph decorations share a lane, the highest shows,
    /// widgets before decorations on a tie (Monaco `zIndex`).
    pub z_index: i32,
}

impl GlyphMarginWidget {
    /// A widget beside line `row` (an empty range at its start), in the
    /// center lane at z-index 0.
    pub fn new(row: u32, view: impl Into<AnyView>) -> Self {
        let start = PointUtf16::new(row, 0);
        Self::with_range(start..start, view)
    }

    /// A widget for `range` (Monaco `getPosition().range`), in the center
    /// lane at z-index 0.
    pub fn with_range(range: Range<PointUtf16>, view: impl Into<AnyView>) -> Self {
        Self {
            range,
            view: view.into(),
            lane: Default::default(),
            z_index: 0,
        }
    }
}

/// Where a glyph margin widget goes (Monaco `IGlyphMarginWidgetPosition`),
/// for [`CodeEditor::layout_glyph_margin_widget`](crate::CodeEditor::layout_glyph_margin_widget).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphMarginWidgetPosition {
    /// The range it's for.
    pub range: Range<PointUtf16>,
    /// Its glyph margin lane.
    pub lane: crate::decorations::GlyphMarginLane,
    /// Its z-index.
    pub z_index: i32,
}

#[derive(Clone)]
pub(crate) struct GlyphWidgetState {
    pub id: WidgetId,
    /// The range's start (the line's, for gutter widgets).
    pub anchor: Anchor,
    /// The range's end (`anchor` for gutter widgets).
    pub end: Anchor,
    pub view: AnyView,
    pub lane: crate::decorations::GlyphMarginLane,
    pub z_index: i32,
}

/// Stored view zone: `after_line` tracked with an anchor.
#[derive(Clone)]
pub(crate) struct ZoneState {
    pub id: WidgetId,
    /// Anchor at the end of the line before the zone (at `after_column`
    /// with one); `None` for line 0.
    pub anchor: Option<Anchor>,
    /// Monaco `afterColumn` was given: the anchor is at that column.
    pub after_column: bool,
    pub after_column_affinity: PositionAffinity,
    pub show_in_hidden_areas: bool,
    /// Monaco `_getZoneOrdinal`.
    pub ordinal: i64,
    pub suppress_mouse_down: bool,
    pub height: ZoneHeight,
    pub view: AnyView,
    /// Monaco `marginDomNode`.
    pub margin_view: Option<AnyView>,
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
    /// Monaco `onDomNodeTop`, and the top it was last told.
    pub on_dom_node_top: Option<ZoneCallback>,
    pub reported_top: Option<Pixels>,
    /// Monaco `onComputedHeight`, and the height it was last told.
    pub on_computed_height: Option<ZoneCallback>,
    pub reported_height: Option<Pixels>,
}

impl ZoneState {
    fn has_callbacks(&self) -> bool {
        self.on_dom_node_top.is_some() || self.on_computed_height.is_some()
    }
}

#[derive(Clone)]
pub(crate) struct ContentWidgetState {
    pub id: WidgetId,
    /// `None`: hidden (Monaco's `null` position).
    pub anchor: Option<Anchor>,
    pub secondary: Option<Anchor>,
    pub affinity: Option<PositionAffinity>,
    pub preference: Vec<ContentWidgetPreference>,
    pub view: AnyView,
    pub allow_overflow: bool,
    pub suppress_mouse_down: bool,
    /// Paint order among overflowing widgets (a [`layer`]).
    pub layer: usize,
    /// Where the editor publishes the room the widget has, when it
    /// overflows.
    pub room: OverflowRoomCell,
    /// A box in the window it's placed against instead of its position's
    /// line, for an overflowing widget (Monaco's `IAnchor`, such as the
    /// code action menu at a clicked hover link).
    pub window_anchor: Option<gpui::Bounds<Pixels>>,
    /// Monaco `useDisplayNone`.
    pub display_none: bool,
    /// Monaco `beforeRender` and `afterRender`.
    pub hooks: Option<Rc<ContentWidgetHooks>>,
}

#[derive(Clone)]
pub(crate) struct OverlayWidgetState {
    pub id: WidgetId,
    pub position: OverlayPosition,
    pub stack_ordinal: Option<i32>,
    pub min_content_width: Pixels,
    pub view: AnyView,
    pub allow_overflow: bool,
    /// Paint order among overflowing widgets (a [`layer`]).
    pub layer: usize,
    /// Placed beside the gutter, centered on a row, by the glyph hover
    /// (Monaco `GlyphHoverWidget._showAt`) instead of by `position`.
    pub beside_gutter: Option<BesideGutter>,
}

/// Where the glyph hover goes (Monaco `GlyphHoverWidget._showAt`): right of
/// the glyph margin, or of the line numbers, vertically centered on a
/// model row's first view line and kept inside the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BesideGutter {
    /// The 0-based model row.
    pub row: u32,
    /// Right of the line numbers too (Monaco's `'lineNo'`).
    pub after_line_numbers: bool,
}

/// Widget positions carried from one model to the next: each content
/// widget's (position, secondary position) and each glyph margin widget's
/// range, in widget order.
pub(crate) struct WidgetPositions {
    pub content: Vec<(Option<PointUtf16>, Option<PointUtf16>)>,
    pub glyphs: Vec<Range<PointUtf16>>,
    /// Widgets across the gutter (the inline edit indicator).
    pub gutter: Vec<u32>,
}

/// What cached zone rows were computed for: the view lines (their
/// generation, which changes with every fold, unlike a pointer to them), the
/// text version and the zones generation.
pub(crate) type ZoneRowsKey = (u64, text::Version, u64);

/// All embedded views of one editor.
#[derive(Clone, Default)]
pub(crate) struct Widgets {
    pub zones: Vec<ZoneState>,
    pub content: Vec<ContentWidgetState>,
    pub overlays: Vec<OverlayWidgetState>,
    pub glyphs: Vec<GlyphWidgetState>,
    /// Widgets across the whole gutter beside a line (glyph margin, line
    /// numbers and line decorations), above its glyphs, shown whenever
    /// the gutter has a width: Monaco's inline edit gutter indicator, an
    /// overlay over the gutter (`lane` and `z_index` are unused).
    pub gutter: Vec<GlyphWidgetState>,
    next_id: u64,
    /// Bumped when zones are added or removed.
    pub zones_generation: u64,
    /// How many zones have `onDomNodeTop` or `onComputedHeight`: layout
    /// reports nothing without any.
    pub zone_callbacks: usize,
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

    /// Places overflowing content widget `id` against `bounds` in the
    /// window instead of its position's line.
    pub fn set_window_anchor(&mut self, id: WidgetId, bounds: gpui::Bounds<Pixels>) {
        if let Some(widget) = self.content.iter_mut().find(|w| w.id == id) {
            widget.window_anchor = Some(bounds);
        }
    }

    /// Makes the wheel over zone `id` leave the editor alone.
    pub fn set_zone_captures_wheel(&mut self, id: WidgetId) {
        if let Some(zone) = self.zones.iter_mut().find(|z| z.id == id) {
            zone.captures_wheel = true;
        }
    }

    pub fn remove(&mut self, id: WidgetId) -> bool {
        let before = self.zones.len()
            + self.content.len()
            + self.overlays.len()
            + self.glyphs.len()
            + self.gutter.len();
        self.remove_zone(id);
        self.content.retain(|w| w.id != id);
        self.overlays.retain(|w| w.id != id);
        self.glyphs.retain(|w| w.id != id);
        self.gutter.retain(|w| w.id != id);
        before
            != self.zones.len()
                + self.content.len()
                + self.overlays.len()
                + self.glyphs.len()
                + self.gutter.len()
    }

    /// Adds a view zone.
    pub fn push_zone(&mut self, zone: ZoneState) {
        self.zone_callbacks += usize::from(zone.has_callbacks());
        self.zones.push(zone);
        self.zones_generation += 1;
    }

    /// Removes view zone `id`; returns whether there was one.
    pub fn remove_zone(&mut self, id: WidgetId) -> bool {
        let Some(ix) = self.zones.iter().position(|z| z.id == id) else {
            return false;
        };
        let zone = self.zones.remove(ix);
        self.zone_callbacks -= usize::from(zone.has_callbacks());
        self.zones_generation += 1;
        true
    }

    /// Drops every view zone (Monaco's go with the old view when the model
    /// changes). Returns whether there were any.
    pub fn clear_zones(&mut self) -> bool {
        if self.zones.is_empty() {
            return false;
        }
        self.zones.clear();
        self.zone_callbacks = 0;
        self.zones_generation += 1;
        self.zone_rows = None;
        true
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
            .map(|(top, _)| top)
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
        assert_eq!(tight, Some((px(-20.), Above)));
        // The preference used, for `afterRender`.
        let below =
            place_content_widget(&[Above, Below], px(10.), px(20.), px(50.), px(0.)..px(200.));
        assert_eq!(below, Some((px(30.), Below)));
        // Exact always fits.
        assert_eq!(place(&[Exact, Below], 0.), Some(px(0.)));
        // No preference: hidden.
        assert_eq!(place(&[], 100.), None);
    }

    #[test]
    fn zone_ordinals() {
        // Monaco `_getZoneOrdinal`: `ordinal`, else `afterColumn`, else
        // 10000 (columns are 1-based in Monaco, 0-based here).
        assert_eq!(zone_ordinal(None, None), 10_000);
        assert_eq!(zone_ordinal(None, Some(4)), 5);
        assert_eq!(zone_ordinal(Some(-3), Some(4)), -3);
    }

    #[test]
    fn overlay_corners_and_stacking() {
        use gpui::{Bounds, point, size};
        let editor = Bounds::new(point(px(100.), px(50.)), size(px(600.), px(400.)));
        let text = Bounds::new(point(px(160.), px(50.)), size(px(400.), px(400.)));
        let w = size(px(80.), px(20.));
        let place = |widgets: &[(OverlayPosition, Option<i32>)]| {
            place_overlay_widgets(
                widgets,
                &vec![w; widgets.len()],
                editor,
                text,
                px(28.) + px(40.),
                px(24.),
            )
        };
        use OverlayPosition::*;
        // Monaco `_renderWidget`: right = 2 × scrollbar + minimap; bottom
        // keeps 2 × the horizontal scrollbar.
        assert_eq!(
            place(&[(TopRight, None), (BottomRight, None)]),
            vec![
                point(px(700. - 68. - 80.), px(50.)),
                point(px(700. - 68. - 80.), px(450. - 24. - 20.)),
            ]
        );
        // Stacked widgets go one under the other by ordinal; unstacked ones
        // stay in the corner.
        assert_eq!(
            place(&[(TopRight, Some(2)), (TopRight, Some(1)), (TopRight, None)]),
            vec![
                point(px(552.), px(70.)),
                point(px(552.), px(50.)),
                point(px(552.), px(50.)),
            ]
        );
        assert_eq!(
            place(&[(TopCenter, Some(0)), (TopCenter, Some(0))]),
            vec![point(px(320.), px(50.)), point(px(320.), px(70.))]
        );
        // Custom and manual positions are relative to the editor.
        assert_eq!(
            place(&[(Custom(point(px(5.), px(6.))), None), (Manual, None)]),
            vec![point(px(105.), px(56.)), point(px(100.), px(50.))]
        );
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
