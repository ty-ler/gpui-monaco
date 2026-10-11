//! The suggest widget (Monaco's `suggestWidget`, `suggestWidgetRenderer`,
//! `suggestWidgetDetails` and `suggestWidgetStatus`): the completion list
//! with match highlights, kind icons and label details, its scrollbar, the
//! "Loading..." and "No suggestions." messages, the details panel (toggled
//! with Ctrl+Space, east or west of the list when it fits, else above or
//! below it), the clickable status bar, and resizing both.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Context, DragMoveEvent, FontWeight, Global, HighlightStyle, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    Size, StatefulInteractiveElement, Styled, StyledText, Task, WeakEntity, Window, div, point, px,
    rgb, size,
};

use crate::codicons::{Codicon, icon as codicon};
use crate::completion::Session;
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_documentation};
use crate::options::{SuggestInsertMode, SuggestOptions};
use crate::providers::{CompletionItem, CompletionItemKind, CompletionItemTag};
use crate::theme::{EditorColors, Theme};
use crate::widgets::{ContentWidgetPreference, OverflowRoom, WidgetAnchor};

/// Rows shown at the default height (hardcoded in Monaco).
const DEFAULT_ROWS: f32 = 12.;
/// Monaco's default widget width.
const DEFAULT_WIDTH: f32 = 430.;
/// Monaco's minimum widget width.
const MIN_WIDTH: f32 = 220.;
/// The details panel's default width (Monaco `SuggestDetailsWidget._size`).
const DETAILS_WIDTH: f32 = 330.;
/// The widgets' border (Monaco's `borderWidth`).
const BORDER: f32 = 1.;
/// Monaco's `horizontalPadding` around the widgets.
const HORIZONTAL_PADDING: f32 = 14.;
/// Monaco's `verticalPadding` above and below the widgets.
const VERTICAL_PADDING: f32 = 22.;
/// Monaco `SuggestDetailsWidget.scrollDown`'s default step.
const DETAILS_SCROLL_STEP: f32 = 8.;
/// Monaco `SuggestDetailsWidget.pageDown`'s step.
const DETAILS_PAGE_STEP: f32 = 80.;
/// Monaco's list and details scrollbars (`verticalScrollbarSize`).
const SCROLLBAR_WIDTH: f32 = 10.;
/// Monaco `ScrollbarState`'s `MINIMUM_SLIDER_SIZE`.
const MIN_SLIDER: f32 = 20.;
/// How long Monaco's `ScrollbarVisibility.Auto` scrollbars stay after
/// scrolling (`HIDE_TIMEOUT`).
const SCROLLBAR_HIDE: Duration = Duration::from_millis(500);
/// Measuring the details panel's content may take a few frames when its
/// placement changes its width; give up after this many.
const MAX_DETAILS_MEASURES: u8 = 4;

/// How the focused details panel scrolls (Monaco `SuggestDetailsWidget`'s
/// `scrollUp`, `scrollDown`, `pageUp`, `pageDown`, `scrollTop` and
/// `scrollBottom`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetailsScroll {
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// The message shown while an explicit request is computing.
pub const LOADING_MESSAGE: &str = "Loading...";
/// The message shown when an explicit request found nothing.
pub const NO_SUGGESTIONS_MESSAGE: &str = "No suggestions.";

/// What the widget shows; rebuilt from the session on each change.
pub struct SuggestWidgetState {
    rows: Vec<(CompletionItem, Vec<Range<usize>>)>,
    selected: usize,
    /// Whether `selected` is focused (`suggest.selection_mode` may show
    /// the list without a focused row).
    focused: bool,
    /// Explain mode's detail and docs for the focused row.
    explanation: Option<(SharedString, String)>,
    /// "Loading..." or "No suggestions." instead of rows.
    message: Option<&'static str>,
    /// Monaco's `Frozen` state: nothing matches what was typed, so the
    /// last matches stay, without bold highlights.
    frozen: bool,
    /// Monaco's `Details` state: the details panel has the focus
    /// (`toggleSuggestionFocus`).
    details_focused: bool,
    /// The completion model shown (a new one per request), for what's
    /// measured once per model (`fitWidthToDetails`).
    model_id: u64,
}

impl SuggestWidgetState {
    pub(crate) fn from_session(session: &Session) -> Self {
        if session.model.filtered().is_empty() {
            return Self::message(NO_SUGGESTIONS_MESSAGE);
        }
        let rows = session
            .model
            .filtered()
            .iter()
            .map(|scored| {
                (
                    session.model.items()[scored.index].clone(),
                    scored.highlights.clone(),
                )
            })
            .collect();
        Self {
            rows,
            selected: session.selected,
            focused: session.focused,
            explanation: None,
            message: None,
            frozen: session.frozen,
            details_focused: false,
            model_id: session.id,
        }
    }

    pub(crate) fn message(message: &'static str) -> Self {
        Self {
            rows: Vec::new(),
            selected: 0,
            focused: false,
            explanation: None,
            message: Some(message),
            frozen: false,
            details_focused: false,
            model_id: 0,
        }
    }

    /// Shows explain mode's detail and docs for the focused row instead of
    /// its own.
    pub(crate) fn set_explanation(&mut self, explanation: Option<(SharedString, String)>) {
        self.explanation = explanation;
    }

    /// Marks the details panel as having the focus.
    pub(crate) fn set_details_focused(&mut self, focused: bool) {
        self.details_focused = focused;
    }

    /// The focused row's item.
    fn focused_item(&self) -> Option<&CompletionItem> {
        self.focused
            .then(|| self.rows.get(self.selected).map(|(item, _)| item))
            .flatten()
    }
}

/// Monaco's stored suggest UI state: whether details are expanded
/// (`expandSuggestionDocs`) and the size the user resized the list to
/// (`suggestWidget.size`). Kept for the app's lifetime.
#[derive(Default)]
pub(crate) struct SuggestUi {
    pub(crate) details_expanded: bool,
    pub(crate) size: Option<Size<Pixels>>,
}

impl Global for SuggestUi {}

/// Whether the details panel is expanded.
pub(crate) fn details_expanded(cx: &App) -> bool {
    cx.try_global::<SuggestUi>()
        .is_some_and(|ui| ui.details_expanded)
}

/// Monaco `canExpandCompletionItem`: docs, or a detail other than the
/// label.
pub(crate) fn can_expand(item: &CompletionItem) -> bool {
    item.documentation.is_some()
        || item
            .detail
            .as_ref()
            .is_some_and(|detail| *detail != item.label)
}

/// Monaco `CompletionItemKinds.toLabel`: the kind as screen readers read
/// it.
fn kind_label(kind: CompletionItemKind) -> &'static str {
    use CompletionItemKind::*;
    match kind {
        Method => "Method",
        Function => "Function",
        Constructor => "Constructor",
        Field => "Field",
        Variable => "Variable",
        Class => "Class",
        Struct => "Struct",
        Interface => "Interface",
        Module => "Module",
        Property => "Property",
        Event => "Event",
        Operator => "Operator",
        Unit => "Unit",
        Value => "Value",
        Constant => "Constant",
        Enum => "Enum",
        EnumMember => "Enum Member",
        Keyword => "Keyword",
        Text => "Text",
        Color => "Color",
        File => "File",
        Reference => "Reference",
        Customcolor => "Custom Color",
        Folder => "Folder",
        TypeParameter => "Type Parameter",
        User => "User",
        Issue => "Issue",
        Tool => "Tool",
        Snippet => "Snippet",
    }
}

/// Monaco's suggest list `getAriaLabel`: what screen readers read for a
/// focused suggestion, with its docs while the details panel shows them.
pub(crate) fn aria_label(item: &CompletionItem, details: bool) -> String {
    let kind = kind_label(item.kind);
    let label = &item.label;
    let mut text = match (&item.label_detail, &item.label_description) {
        (Some(detail), Some(description)) => format!("{label} {detail}, {description}, {kind}"),
        (Some(detail), None) => format!("{label} {detail}, {kind}"),
        (None, Some(description)) => format!("{label}, {description}, {kind}"),
        (None, None) => format!("{label}, {kind}"),
    };
    if details {
        let detail = item.detail.as_deref().unwrap_or("");
        let docs = item.documentation.as_ref().map_or("", |d| d.as_str());
        text = format!("{text}, docs: {detail}{docs}");
    }
    text
}

/// Sizes that depend on the editor.
pub(crate) struct SuggestWidgetMetrics {
    /// Row height (`suggest.line_height`, or the editor's).
    pub(crate) item_height: Pixels,
    /// `suggest.font_size`, or the editor's.
    pub(crate) font_size: Pixels,
    /// The height of the line the list is anchored to.
    pub(crate) line_height: Pixels,
    /// The editor's width, which `suggest.fit_width_to_details` doesn't
    /// widen the list past.
    pub(crate) editor_width: Option<Pixels>,
}

/// A resize sash being dragged: the list's (Monaco's `ResizableHTMLElement`
/// around the list) or the details panel's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sash {
    North,
    East,
    South,
    DetailsNorth,
    DetailsEast,
    DetailsSouth,
    DetailsWest,
}

/// Which scrollbar a slider drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollbarTarget {
    List,
    Details,
}

/// A scrollbar slider being dragged, and where in the slider it was
/// grabbed.
#[derive(Clone)]
struct SliderDrag {
    target: ScrollbarTarget,
    grab: Rc<Cell<Pixels>>,
}

/// The list's size and side (Monaco `SuggestWidget._layout`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ListLayout {
    /// Inside the border.
    pub(crate) size: Size<Pixels>,
    /// Above or below the cursor line.
    pub(crate) side: ContentWidgetPreference,
}

/// What [`layout_list`] works from.
pub(crate) struct ListLayoutInput {
    /// The size the user resized the list to, or the default (inside the
    /// border).
    pub(crate) wanted: Size<Pixels>,
    /// Whether `wanted` is a resized size.
    pub(crate) resized: bool,
    /// With `suggest.fit_width_to_details`, the width the widest
    /// suggestion and its inline details need.
    pub(crate) fit_width: Option<Pixels>,
    /// The editor's width.
    pub(crate) editor_width: Option<Pixels>,
    /// The suggestions listed.
    pub(crate) rows: usize,
    pub(crate) item_height: Pixels,
    pub(crate) status_height: Pixels,
    /// The room around the cursor line in the window.
    pub(crate) room: Option<OverflowRoom>,
    /// The side chosen when the session showed its list (Monaco keeps it
    /// while typing).
    pub(crate) locked: Option<ContentWidgetPreference>,
}

/// Monaco `SuggestWidget._layout` for a list of suggestions:
/// - The width is the wanted one (wider with `fitWidthToDetails` until the
///   list is resized), at least 220px and capped to the window.
/// - The height is the wanted one, at most every row and at least one, and
///   capped to the room on the side it goes. Like Monaco's `_cappedHeight`,
///   the wanted height comes back when more rows match again.
/// - The list goes above the cursor when it doesn't fit below and there's
///   more room above. That side is kept while the session lasts (unless a
///   row no longer fits there), so the list doesn't flip as it shrinks and
///   grows while typing.
/// - Whole rows show.
pub(crate) fn layout_list(input: &ListLayoutInput) -> ListLayout {
    let border = px(2. * BORDER);
    let item = input.item_height;
    let full = input.status_height + item * input.rows.max(1) as f32;
    let min = item + input.status_height;
    let (above, below, max_width) = match input.room {
        Some(room) => {
            let window_width = room.width + crate::widgets::OVERFLOW_HORIZONTAL_PADDING * 2.;
            (
                room.above - border,
                room.below - border,
                window_width - border - px(2. * HORIZONTAL_PADDING),
            )
        }
        None => (full, full, px(f32::MAX)),
    };
    let mut width = input.wanted.width.min(max_width);
    if let Some(fit) = input.fit_width.filter(|_| !input.resized) {
        let cap = max_width.min(input.editor_width.unwrap_or(max_width));
        width = width.max(fit.min(cap));
    }
    let width = width.max(px(MIN_WIDTH));
    let max_below = below.min(full);
    let max_above = above.min(full);
    let height = input
        .wanted
        .height
        .max(min)
        .min(max_above.max(max_below).min(full));
    let room_on = |side| match side {
        ContentWidgetPreference::Above => max_above,
        _ => max_below,
    };
    let side = match input.locked {
        Some(side) if room_on(side) >= min => side,
        _ if height > max_below && max_above > max_below => ContentWidgetPreference::Above,
        _ => ContentWidgetPreference::Below,
    };
    let height = height.min(room_on(side));
    let rows = ((height - input.status_height) / item).floor().max(1.);
    ListLayout {
        size: size(width, input.status_height + item * rows),
        side,
    }
}

/// Where the details panel goes around the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetailsSide {
    East,
    West,
    South,
    North,
}

/// The details panel's place (Monaco `SuggestDetailsOverlay._placeAtAnchor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DetailsLayout {
    /// Its border box, in the coordinates of `anchor`.
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) side: DetailsSide,
    /// The edges it can be resized from: north, east, south, west.
    pub(crate) sashes: [bool; 4],
}

/// Monaco `SuggestDetailsOverlay._placeAtAnchor`: places a details panel of
/// `wanted` size around the list's border box `anchor` in a window of
/// `window` size. East, then west, then below the list (above it when the
/// list is above the cursor), whichever fits first, else the least bad;
/// aligned with the list's top, or its bottom when there's no room
/// below, and capped to the room there.
pub(crate) fn place_details(
    anchor: Bounds<Pixels>,
    window: Size<Pixels>,
    wanted: Size<Pixels>,
    line_height: Pixels,
    prefer_align_at_top: bool,
) -> DetailsLayout {
    struct Placement {
        side: DetailsSide,
        top: Pixels,
        left: Pixels,
        fit: Pixels,
        max_top: Size<Pixels>,
        max_bottom: Size<Pixels>,
        min: Size<Pixels>,
    }
    let border = px(BORDER);
    let border2 = px(2. * BORDER);
    let horizontal = px(HORIZONTAL_PADDING);
    let vertical = px(VERTICAL_PADDING);
    let default_min = size(px(220.), line_height * 2.);
    let beside = |side: DetailsSide, width: Pixels, left: Pixels| {
        let max_top = size(width, window.height - anchor.top() - border2 - vertical);
        let max_bottom = size(width, anchor.bottom() - border2 - vertical);
        Placement {
            side,
            top: anchor.top(),
            left,
            fit: width - wanted.width,
            max_top,
            max_bottom,
            min: size(width.min(default_min.width), default_min.height),
        }
    };
    let east_width = window.width - (anchor.right() + border + horizontal);
    let east = beside(DetailsSide::East, east_width, anchor.right() - border);
    let west_width = anchor.left() - border - horizontal;
    let west = beside(
        DetailsSide::West,
        west_width,
        horizontal.max(anchor.left() - wanted.width - border),
    );
    let vertical_placement = if prefer_align_at_top {
        let max = size(
            anchor.size.width - border2,
            window.height - anchor.bottom() - vertical,
        );
        Placement {
            side: DetailsSide::South,
            top: anchor.bottom() - border,
            left: anchor.left(),
            fit: max.height - wanted.height,
            max_top: max,
            max_bottom: max,
            min: size(max.width, default_min.height),
        }
    } else {
        let max = size(anchor.size.width - border2, anchor.top() - vertical);
        Placement {
            side: DetailsSide::North,
            top: vertical.max(anchor.top() - wanted.height),
            left: anchor.left(),
            fit: max.height - wanted.height,
            max_top: max,
            max_bottom: max,
            min: size(max.width, default_min.height),
        }
    };
    let placements = [east, west, vertical_placement];
    let placement = match placements.iter().position(|p| p.fit >= px(0.)) {
        Some(ix) => &placements[ix],
        // The least bad: the first with the largest fit.
        None => placements
            .iter()
            .rev()
            .max_by(|a, b| {
                a.fit
                    .partial_cmp(&b.fit)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("three placements"),
    };
    let height = wanted
        .height
        .min(placement.max_top.height.max(placement.max_bottom.height));
    let (align_at_top, max) = if prefer_align_at_top {
        if height <= placement.max_top.height {
            (true, placement.max_top)
        } else {
            (false, placement.max_bottom)
        }
    } else if height <= placement.max_bottom.height {
        (false, placement.max_bottom)
    } else {
        (true, placement.max_top)
    };
    let mut top = placement.top;
    if placement.side == DetailsSide::North {
        top = anchor.top() - height + border;
    } else if !align_at_top && height > anchor.size.height {
        top = anchor.bottom() - border2 - height;
    }
    // `ResizableHTMLElement.layout`: within the maximum, then at least the
    // minimum.
    let width = wanted.width.min(max.width).max(placement.min.width);
    let height = height.min(max.height).max(placement.min.height);
    let sashes = if placement.side == DetailsSide::North {
        [true, false, false, true]
    } else {
        let east = placement.side == DetailsSide::East;
        [!align_at_top, east, align_at_top, !east]
    };
    DetailsLayout {
        bounds: Bounds::new(point(placement.left, top), size(width, height)),
        side: placement.side,
        sashes,
    }
}

/// Monaco `ScrollbarState`: a slider's offset and length along a track as
/// long as the visible part, or `None` when everything is visible.
fn slider(visible: Pixels, content: Pixels, position: Pixels) -> Option<(Pixels, Pixels)> {
    if content <= visible || visible <= px(0.) {
        return None;
    }
    let length = (visible * (visible / content))
        .floor()
        .max(px(MIN_SLIDER))
        .min(visible);
    let ratio = (visible - length) / (content - visible);
    Some((position * ratio, length))
}

/// The scroll position putting a slider's top at `slider_top` (the inverse
/// of [`slider`]).
fn slider_position(visible: Pixels, content: Pixels, slider_top: Pixels) -> Pixels {
    let Some((_, length)) = slider(visible, content, px(0.)) else {
        return px(0.);
    };
    let ratio = (visible - length) / (content - visible);
    if ratio <= 0. {
        return px(0.);
    }
    (slider_top / ratio).clamp(px(0.), content - visible)
}

/// The completion list, shown below or above the cursor while suggestions are
/// open.
pub struct SuggestWidget {
    editor: WeakEntity<CodeEditor>,
    state: SuggestWidgetState,
    theme: Arc<Theme>,
    style: MarkdownStyle,
    options: SuggestOptions,
    metrics: SuggestWidgetMetrics,
    /// First visible row.
    scroll: usize,
    /// The details panel's scroll position (Monaco's `_body.scrollTop`).
    details_scroll: gpui::ScrollHandle,
    /// The scroll position asked for, kept until layout can clamp it.
    details_scroll_top: Pixels,
    /// The room it has in the window (it overflows the editor).
    pub(crate) room: crate::widgets::OverflowRoomCell,
    /// Wheel movement not yet a whole row.
    wheel_remainder: Pixels,
    /// The side of the cursor line the list went on when it first showed
    /// suggestions; kept while it lives (one session), so it doesn't flip
    /// while typing.
    side: Option<ContentWidgetPreference>,
    /// `suggest.fit_width_to_details`' measured width, for a completion
    /// model (Monaco caches it per model).
    fit_width: Option<(u64, Pixels)>,
    /// The size the details panel was resized to (Monaco
    /// `SuggestDetailsOverlay._userSize`, kept while the session lasts).
    details_size: Option<Size<Pixels>>,
    /// The details panel's content height, measured when it was last
    /// painted.
    details_content: Rc<Cell<Option<Pixels>>>,
    /// Measurements since the content last changed.
    details_measures: Rc<Cell<u8>>,
    /// Where the rows were last painted, for clicks on the scrollbar.
    rows_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Monaco's `ScrollbarVisibility.Auto`: the scrollbars show for a
    /// while after scrolling (and while the mouse is over them).
    scrollbars_revealed: bool,
    hide_scrollbars: Option<Task<()>>,
    /// The scrollbar whose slider is being dragged.
    slider_drag: Option<ScrollbarTarget>,
    /// The details panel's last place, for tests.
    details_layout: Option<DetailsLayout>,
}

/// The kind's codicon (Monaco `CompletionItemKinds.toIcon`), in the
/// theme's `symbolIcon.*Foreground` color (the row's text color for kinds
/// without one).
fn kind_icon(kind: CompletionItemKind, colors: &EditorColors) -> (Codicon, Option<Hsla>) {
    (kind.icon(), colors.symbol_icons.for_completion_kind(kind))
}

/// Monaco's `ColorExtractor` pattern: hex, `rgb[a]()` and `hsl[a]()`.
const COLOR_PATTERN: &str = r"(#([\da-fA-F]{3}){1,2}|(rgb|hsl)a\(\s*(\d{1,3}%?\s*,\s*){3}(1|0?\.\d+)\)|(rgb|hsl)\(\s*\d{1,3}%?(\s*,\s*\d{1,3}%?){2}\s*\))";

/// The color a `Color` item shows as a swatch (Monaco `ColorExtractor`):
/// its label or detail if either is exactly a color, else a color at the
/// start or end of its docs.
pub(crate) fn completion_item_color(item: &CompletionItem) -> Option<Hsla> {
    use std::sync::OnceLock;
    static STRICT: OnceLock<regex::Regex> = OnceLock::new();
    static RELAXED: OnceLock<regex::Regex> = OnceLock::new();
    let strict =
        STRICT.get_or_init(|| regex::Regex::new(&format!("(?i)^{COLOR_PATTERN}$")).expect("valid"));
    let relaxed = RELAXED.get_or_init(|| regex::Regex::new(COLOR_PATTERN).expect("valid"));
    if strict.is_match(&item.label) {
        return parse_css_color(&item.label);
    }
    if let Some(detail) = item.detail.as_ref().filter(|d| strict.is_match(d)) {
        return parse_css_color(detail);
    }
    let docs = item.documentation.as_ref()?.as_str();
    let found = relaxed.find(docs)?;
    (found.start() == 0 || found.end() == docs.len())
        .then(|| parse_css_color(found.as_str()))
        .flatten()
}

/// Parses `#rgb`, `#rrggbb`, `rgb[a](…)` and `hsl[a](…)`.
fn parse_css_color(text: &str) -> Option<Hsla> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        let hex: String = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect()
        } else {
            hex.to_string()
        };
        let value = u32::from_str_radix(&hex, 16).ok()?;
        return Some(rgb(value).into());
    }
    let open = text.find('(')?;
    let function = text[..open].to_ascii_lowercase();
    let args: Vec<&str> = text[open + 1..text.len() - 1]
        .split(',')
        .map(str::trim)
        .collect();
    let number = |s: &str, scale: f32| -> Option<f32> {
        match s.strip_suffix('%') {
            Some(percent) => Some(percent.parse::<f32>().ok()? / 100.),
            None => Some(s.parse::<f32>().ok()? / scale),
        }
    };
    let alpha = args.get(3).map_or(Some(1.), |a| a.parse::<f32>().ok())?;
    if function.starts_with("rgb") {
        let [r, g, b] = [args[0], args[1], args[2]].map(|c| number(c, 255.));
        Some(
            gpui::Rgba {
                r: r?,
                g: g?,
                b: b?,
                a: alpha,
            }
            .into(),
        )
    } else {
        let h = args[0].trim_end_matches('%').parse::<f32>().ok()? / 360.;
        Some(gpui::hsla(
            h,
            number(args[1], 100.)?,
            number(args[2], 100.)?,
            alpha,
        ))
    }
}

/// UTF-16 ranges in `text` to byte ranges.
fn utf16_ranges_to_bytes(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut boundaries = Vec::with_capacity(text.len() + 1);
    for (ix, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            boundaries.push(ix);
        }
    }
    boundaries.push(text.len());
    ranges
        .iter()
        .filter_map(|r| {
            let start = *boundaries.get(r.start)?;
            let end = *boundaries.get(r.end.min(boundaries.len() - 1))?;
            (start < end).then_some(start..end)
        })
        .collect()
}

impl SuggestWidget {
    pub(crate) fn new(
        editor: WeakEntity<CodeEditor>,
        state: SuggestWidgetState,
        theme: Arc<Theme>,
        style: MarkdownStyle,
        options: SuggestOptions,
        metrics: SuggestWidgetMetrics,
        cx: &App,
    ) -> Self {
        let mut this = Self {
            editor,
            state,
            theme,
            style,
            options,
            metrics,
            scroll: 0,
            details_scroll: gpui::ScrollHandle::new(),
            details_scroll_top: px(0.),
            room: Default::default(),
            wheel_remainder: px(0.),
            side: None,
            fit_width: None,
            details_size: None,
            details_content: Default::default(),
            details_measures: Default::default(),
            rows_bounds: Default::default(),
            scrollbars_revealed: false,
            hide_scrollbars: None,
            slider_drag: None,
            details_layout: None,
        };
        this.keep_selected_visible(cx);
        this
    }

    pub(crate) fn set_state(
        &mut self,
        state: SuggestWidgetState,
        metrics: SuggestWidgetMetrics,
        cx: &mut Context<Self>,
    ) {
        // Monaco shows another item's details from the top.
        if state.focused_item().map(|item| &item.label)
            != self.state.focused_item().map(|item| &item.label)
            || state.selected != self.state.selected
        {
            self.scroll_details(DetailsScroll::Top);
        }
        // The details panel's content may have changed: measure it again.
        self.details_measures.set(0);
        self.state = state;
        self.metrics = metrics;
        let scroll = self.scroll;
        self.keep_selected_visible(cx);
        if self.scroll != scroll {
            self.reveal_scrollbars(cx);
        }
        cx.notify();
    }

    pub(crate) fn message(&self) -> Option<&'static str> {
        self.state.message
    }

    /// What screen readers read for the widget: its message, or the focused
    /// suggestion's [`aria_label`] (with docs when `resolved` and the details
    /// panel shows).
    pub(crate) fn announcement(&self, resolved: bool, cx: &App) -> Option<String> {
        if let Some(message) = self.state.message {
            return Some(message.to_string());
        }
        let item = self.state.focused_item()?;
        Some(aria_label(item, resolved && self.details_shown(cx)))
    }

    /// Scrolls the details panel (Monaco `SuggestDetailsWidget`'s scroll
    /// methods), within its content once it's laid out.
    pub(crate) fn scroll_details(&mut self, scroll: DetailsScroll) {
        let max = self.details_scroll.max_offset().y;
        let laid_out = self.details_scroll.bounds().size.height > px(0.);
        // Where the panel is, after any wheel scrolling.
        let current = if laid_out {
            -self.details_scroll.offset().y
        } else {
            self.details_scroll_top
        };
        let top = match scroll {
            DetailsScroll::Up => current - px(DETAILS_SCROLL_STEP),
            DetailsScroll::Down => current + px(DETAILS_SCROLL_STEP),
            DetailsScroll::PageUp => current - px(DETAILS_PAGE_STEP),
            DetailsScroll::PageDown => current + px(DETAILS_PAGE_STEP),
            DetailsScroll::Top => px(0.),
            DetailsScroll::Bottom => {
                if laid_out {
                    max
                } else {
                    px(f32::MAX)
                }
            }
        };
        let top = if laid_out { top.min(max) } else { top };
        self.set_details_scroll_top(top);
    }

    fn set_details_scroll_top(&mut self, top: Pixels) {
        self.details_scroll_top = top.max(px(0.));
        self.details_scroll
            .set_offset(point(px(0.), -self.details_scroll_top));
    }

    /// The details panel's scroll position.
    pub(crate) fn details_scroll_top(&self) -> Pixels {
        self.details_scroll_top
    }

    /// The details panel's text, for copying (Monaco `suggestWidgetCopy`
    /// copies the selected text): the detail, then the docs' markdown.
    pub(crate) fn details_text(&self) -> Option<String> {
        let (detail, docs) = self.details_content()?;
        let parts: Vec<String> = detail
            .map(|d| d.to_string())
            .into_iter()
            .chain(docs.map(|d| d.as_str().to_string()))
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    fn status_bar_height(&self) -> Pixels {
        if self.options.show_status_bar && self.state.message.is_none() {
            self.metrics.item_height
        } else {
            px(0.)
        }
    }

    /// The list's size and side (see [`layout_list`]): the resized size or
    /// Monaco's default of 12 rows, fitted to the room in the window.
    fn list_layout(&self, cx: &App) -> ListLayout {
        let item_height = self.metrics.item_height;
        let status = self.status_bar_height();
        let stored = cx.try_global::<SuggestUi>().and_then(|ui| ui.size);
        let default = size(px(DEFAULT_WIDTH), status + item_height * DEFAULT_ROWS);
        layout_list(&ListLayoutInput {
            wanted: stored.unwrap_or(default),
            resized: stored.is_some(),
            fit_width: self
                .fit_width
                .filter(|(model, _)| *model == self.state.model_id)
                .map(|(_, width)| width),
            editor_width: self.metrics.editor_width,
            rows: self.state.rows.len(),
            item_height,
            status_height: status,
            room: self.room.get(),
            locked: self.side,
        })
    }

    /// The list's size: its border box's width, and the height of its rows
    /// and status bar.
    pub(crate) fn list_size(&self, cx: &App) -> Size<Pixels> {
        self.list_layout(cx).size
    }

    /// Which side of the cursor line the list is on.
    pub(crate) fn list_side(&self, cx: &App) -> ContentWidgetPreference {
        self.list_layout(cx).side
    }

    /// Where the rows and the details panel's content were last painted.
    pub(crate) fn painted_bounds(
        &self,
        cx: &App,
    ) -> Option<(Bounds<Pixels>, Option<Bounds<Pixels>>)> {
        let rows = self.rows_bounds.get()?;
        let details = self
            .details_shown(cx)
            .then(|| self.details_scroll.bounds())
            .filter(|bounds| bounds.size.height > px(0.));
        Some((rows, details))
    }

    /// Where the details panel was last placed: its side and its border
    /// box relative to the list's.
    pub(crate) fn details_placement(&self) -> Option<(DetailsSide, Bounds<Pixels>)> {
        self.details_layout
            .map(|layout| (layout.side, layout.bounds))
    }

    /// Monaco `SuggestWidget._measureContentWidth` (for
    /// `fitWidthToDetails`): the widest label with its inline details in the
    /// suggest font, and the row's icon, read-more button, gaps, padding
    /// and scrollbar.
    fn measure_fit_width(&self, window: &mut Window) -> Pixels {
        let font = gpui::font(self.style.code_font.clone());
        let font_size = self.metrics.font_size;
        let mut widest = px(0.);
        for (item, _) in &self.state.rows {
            let mut text = item.label.to_string();
            if item.label_detail.is_some() || item.label_description.is_some() {
                text.push_str(item.label_detail.as_deref().unwrap_or(""));
                text.push_str(item.label_description.as_deref().unwrap_or(""));
            } else {
                text.push_str(item.detail.as_deref().unwrap_or(""));
            }
            let text = text.replace(['\n', '\r'], " ");
            let run = gpui::TextRun {
                len: text.len(),
                font: font.clone(),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let width = window
                .text_system()
                .shape_line(text.into(), font_size, &[run], None)
                .width;
            widest = widest.max(width);
        }
        widest + self.metrics.item_height * 2. + px(2. * HORIZONTAL_PADDING + 20.)
    }

    /// Monaco's list consumes the wheel (`alwaysConsumeMouseWheel`) and
    /// scrolls by it.
    fn scroll_wheel(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let item_height = self.metrics.item_height;
        let delta = event.delta.pixel_delta(item_height).y + self.wheel_remainder;
        let rows = (delta / item_height).trunc();
        self.wheel_remainder = delta - item_height * rows;
        let max = self.state.rows.len().saturating_sub(self.visible_rows(cx));
        let scroll = (self.scroll as f32 - rows).clamp(0., max as f32) as usize;
        if scroll != self.scroll {
            self.scroll = scroll;
            self.reveal_scrollbars(cx);
            cx.notify();
        }
    }

    /// Shows the scrollbars for a while (Monaco `ScrollableElement._reveal`
    /// on scrolling, then hiding after `HIDE_TIMEOUT`).
    fn reveal_scrollbars(&mut self, cx: &mut Context<Self>) {
        self.scrollbars_revealed = true;
        self.hide_scrollbars = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SCROLLBAR_HIDE).await;
            this.update(cx, |this, cx| {
                this.scrollbars_revealed = false;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Whether the scrollbars show without the mouse over them.
    pub(crate) fn scrollbars_revealed(&self) -> bool {
        self.scrollbars_revealed || self.slider_drag.is_some()
    }

    /// Scrolls the list to put the slider's top at `slider_top` along a
    /// track `visible` tall.
    fn scroll_list_to_slider(&mut self, slider_top: Pixels, cx: &mut Context<Self>) {
        let item_height = self.metrics.item_height;
        let visible_rows = self.visible_rows(cx);
        let visible = item_height * visible_rows as f32;
        let content = item_height * self.state.rows.len() as f32;
        let position = slider_position(visible, content, slider_top);
        let max = self.state.rows.len().saturating_sub(visible_rows);
        let scroll = ((position / item_height).round() as usize).min(max);
        if scroll != self.scroll {
            self.scroll = scroll;
            cx.notify();
        }
        self.reveal_scrollbars(cx);
    }

    /// Scrolls the details panel to put its slider's top at `slider_top`.
    fn scroll_details_to_slider(&mut self, slider_top: Pixels, cx: &mut Context<Self>) {
        let visible = self.details_scroll.bounds().size.height;
        let content = visible + self.details_scroll.max_offset().y;
        let position = slider_position(visible, content, slider_top);
        self.set_details_scroll_top(position);
        self.reveal_scrollbars(cx);
        cx.notify();
    }

    /// A Monaco scrollbar along a track `visible` tall at the right edge:
    /// a slider in `scrollbarSlider.background` (`.hoverBackground` under
    /// the mouse, `.activeBackground` while dragged) that can be dragged,
    /// and a track that scrolls to where it's clicked. Like Monaco's
    /// `ScrollbarVisibility.Auto`, it shows while the mouse is over `group`
    /// or for a while after scrolling.
    fn render_scrollbar(
        &self,
        target: ScrollbarTarget,
        group: &'static str,
        visible: Pixels,
        content: Pixels,
        position: Pixels,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let (top, length) = slider(visible, content, position)?;
        let colors = &self.theme.colors;
        let dragging = self.slider_drag == Some(target);
        let grab = Rc::new(Cell::new(px(0.)));
        let (track_id, slider_id) = match target {
            ScrollbarTarget::List => ("suggest-scrollbar", "suggest-slider"),
            ScrollbarTarget::Details => ("suggest-details-scrollbar", "suggest-details-slider"),
        };
        let shown = self.scrollbars_revealed();
        Some(
            div()
                .id(track_id)
                .absolute()
                .top_0()
                .right_0()
                .w(px(SCROLLBAR_WIDTH))
                .h(visible)
                .when(!shown, |this| {
                    this.invisible().group_hover(group, |style| style.visible())
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        // Monaco centers the slider where the track is
                        // clicked.
                        let track_top = match target {
                            ScrollbarTarget::List => this.rows_bounds.get().map(|b| b.top()),
                            ScrollbarTarget::Details => Some(this.details_scroll.bounds().top()),
                        };
                        if let Some(track_top) = track_top {
                            let slider_top = event.position.y - track_top - length / 2.;
                            match target {
                                ScrollbarTarget::List => this.scroll_list_to_slider(slider_top, cx),
                                ScrollbarTarget::Details => {
                                    this.scroll_details_to_slider(slider_top, cx)
                                }
                            }
                        }
                    }),
                )
                .child(
                    div()
                        .id(slider_id)
                        .absolute()
                        .left_0()
                        .top(top)
                        .w_full()
                        .h(length)
                        .block_mouse_except_scroll()
                        .bg(if dragging {
                            colors.scrollbar_slider_active
                        } else {
                            colors.scrollbar_slider
                        })
                        .when(!dragging, |this| {
                            let hover = colors.scrollbar_slider_hover;
                            this.hover(move |style| style.bg(hover))
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_drag(
                            SliderDrag {
                                target,
                                grab: grab.clone(),
                            },
                            move |drag, offset, _, cx| {
                                drag.grab.set(offset.y);
                                cx.new(|_| gpui::Empty)
                            },
                        ),
                ),
        )
    }

    /// Moves a scrollbar's slider as it's dragged; `track` is where its
    /// track starts.
    fn drag_slider(
        &mut self,
        event: &DragMoveEvent<SliderDrag>,
        track: Pixels,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx).clone();
        self.slider_drag = Some(drag.target);
        let slider_top = event.event.position.y - track - drag.grab.get();
        match drag.target {
            ScrollbarTarget::List => self.scroll_list_to_slider(slider_top, cx),
            ScrollbarTarget::Details => self.scroll_details_to_slider(slider_top, cx),
        }
    }

    /// The first row shown.
    pub(crate) fn first_visible_row(&self) -> usize {
        self.scroll
    }

    /// Rows that fit in the list.
    pub(crate) fn visible_rows(&self, cx: &App) -> usize {
        let height = self.list_size(cx).height - self.status_bar_height();
        ((height / self.metrics.item_height).floor() as usize).max(1)
    }

    fn keep_selected_visible(&mut self, cx: &App) {
        let visible = self.visible_rows(cx);
        let selected = self.state.selected;
        if selected < self.scroll {
            self.scroll = selected;
        } else if selected >= self.scroll + visible {
            self.scroll = selected + 1 - visible;
        }
        self.scroll = self
            .scroll
            .min(self.state.rows.len().saturating_sub(visible));
    }

    fn render_row(&self, ix: usize, show_details: bool) -> impl IntoElement + use<> {
        let colors = &self.theme.colors;
        let (item, highlights) = &self.state.rows[ix];
        let is_selected = self.state.focused && ix == self.state.selected;
        let item_height = self.metrics.item_height;
        let label: SharedString = item.label.clone();
        // Monaco strikes deprecated items through, dimmed, without match
        // highlights.
        let deprecated = item.tags.contains(&CompletionItemTag::Deprecated);
        let highlight_color = if is_selected {
            colors.suggest_focus_highlight_foreground
        } else {
            colors.suggest_highlight_foreground
        };
        let label_highlights: Vec<(Range<usize>, HighlightStyle)> = if deprecated {
            vec![(
                0..label.len(),
                HighlightStyle {
                    strikethrough: Some(gpui::StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..Default::default()
                },
            )]
        } else {
            utf16_ranges_to_bytes(&label, highlights)
                .into_iter()
                .map(|range| {
                    (
                        range,
                        HighlightStyle {
                            color: Some(highlight_color),
                            font_weight: (!self.state.frozen).then_some(FontWeight::BOLD),
                            ..Default::default()
                        },
                    )
                })
                .collect()
        };
        // Monaco's `string-label` rows: a plain label, whose detail shows on
        // the right of the focused row only.
        let string_label = item.label_detail.is_none() && item.label_description.is_none();
        // The right side: a label description on every row, or the detail
        // on the focused row while the details panel is closed.
        let right_text = match &item.label_description {
            Some(description) => Some(description.clone()),
            None => item
                .detail
                .clone()
                .filter(|_| is_selected && !show_details)
                .map(|d| SharedString::from(d.replace(['\n', '\r'], " "))),
        }
        .filter(|_| self.options.show_inline_details);
        let read_more =
            is_selected && !show_details && !self.options.show_status_bar && can_expand(item);
        // `.fit-width-to-details`: the label keeps its width and the right
        // side gives way.
        let fit = self.options.fit_width_to_details;
        let icon = self.options.show_icons.then(|| {
            let mut icon = div()
                .w(item_height)
                .flex_none()
                .flex()
                .justify_center()
                .items_center();
            if item.kind == CompletionItemKind::Color
                && let Some(color) = completion_item_color(item)
            {
                icon = icon.child(
                    div()
                        .size(px(9.))
                        .border_1()
                        .border_color(gpui::black())
                        .bg(color),
                );
            } else {
                let (glyph, glyph_color) = kind_icon(item.kind, &self.theme.colors);
                // `editorSuggestWidget.selectedIconForeground` on the
                // focused row.
                let glyph_color = match self.theme.colors.suggest_selected_icon_foreground {
                    Some(color) if is_selected => Some(color),
                    _ => glyph_color,
                };
                if let Some(glyph_color) = glyph_color {
                    icon = icon.text_color(glyph_color);
                }
                // `.suggest-icon`: a 16px codicon.
                icon = icon.child(codicon(glyph));
            }
            icon
        });
        let editor = self.editor.clone();
        let editor_for_more = self.editor.clone();
        div()
            .id(("suggestion", ix))
            .group("suggest-row")
            .h(item_height)
            .pl(px(2.))
            .pr(px(10.))
            .flex()
            .items_center()
            .overflow_hidden()
            .when(is_selected, |this| {
                let this = this
                    .bg(colors.suggest_selected_background)
                    .text_color(colors.suggest_selected_foreground);
                // `editorSuggestWidget.focusOutline` (high contrast).
                match colors.suggest_focus_outline {
                    Some(outline) => this.border_1().border_color(outline),
                    None => this,
                }
            })
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |editor, cx| {
                        editor.select_suggestion(ix, cx);
                        editor.accept_suggestion(cx);
                    });
                }
            })
            .children(icon)
            .child(
                // `.left`: the label, then its `.signature-label`, cut off
                // with an ellipsis.
                div()
                    .flex_1()
                    .min_w_0()
                    .when(fit, |this| this.flex_shrink_0())
                    .flex()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .when(deprecated, |this| this.opacity(0.66))
                    .child(
                        div()
                            .min_w_0()
                            .when(!string_label, |this| this.flex_shrink_0().max_w_full())
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(StyledText::new(label).with_highlights(label_highlights)),
                    )
                    .when_some(item.label_detail.clone(), |this, detail| {
                        this.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .opacity(0.6)
                                .child(detail),
                        )
                    }),
            )
            .when_some(right_text, |this, text| {
                // `.details-label`: 85% of the font, cut off with an
                // ellipsis.
                this.child(
                    div()
                        .min_w_0()
                        .flex_shrink(4.)
                        .when(!fit, |this| this.max_w(gpui::relative(0.7)))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .ml(self.metrics.font_size * 0.85 * 1.1)
                        .text_size(self.metrics.font_size * 0.85)
                        .when(!is_selected, |this| this.opacity(0.7))
                        .child(text),
                )
            })
            .when(read_more, |this| {
                this.child(
                    div()
                        .id(("read-more", ix))
                        .ml(px(4.))
                        .w(px(18.))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .opacity(0.)
                        .group_hover("suggest-row", |style| style.opacity(1.))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor_for_more.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.toggle_suggestion_details(cx);
                                });
                            }
                        })
                        // `suggest-more-info`: `chevron-right` at 14px.
                        .child(codicon(Codicon::ChevronRight).size(px(14.))),
                )
            })
    }

    /// Whether the details panel shows: expanded, and the focused item has
    /// something to show.
    pub(crate) fn details_shown(&self, cx: &App) -> bool {
        self.state.message.is_none()
            && details_expanded(cx)
            && (self.state.explanation.is_some()
                || self.state.focused_item().is_some_and(can_expand))
    }

    /// The status bar's actions (Monaco's `suggestWidgetStatusbarMenu`), as
    /// (label, keybinding label, what clicking does), left then right.
    fn status_bar_actions(&self, show_details: bool) -> [Vec<StatusAction>; 2] {
        let mac = cfg!(target_os = "macos");
        let (primary, alternative) = match self.options.insert_mode {
            SuggestInsertMode::Insert => ("Insert", "Replace"),
            SuggestInsertMode::Replace => ("Replace", "Insert"),
        };
        let mut left = Vec::new();
        let mut right = Vec::new();
        match self.state.focused_item() {
            // `selectNextSuggestion`'s "Select", while nothing is focused.
            None => left.push(StatusAction {
                label: "Select",
                key: if mac { "↓" } else { "DownArrow" },
                run: StatusCommand::Select,
            }),
            Some(item) => {
                left.push(StatusAction {
                    label: primary,
                    key: if mac { "⏎" } else { "Enter" },
                    run: StatusCommand::Accept,
                });
                // Monaco shows the other mode only when the item's insert
                // and replace ranges differ.
                if item
                    .replace_range
                    .as_ref()
                    .is_some_and(|replace| item.range.as_ref() != Some(replace))
                {
                    left.push(StatusAction {
                        label: alternative,
                        key: if mac { "⇧⏎" } else { "Shift+Enter" },
                        run: StatusCommand::AcceptAlternative,
                    });
                }
                if can_expand(item) {
                    right.push(StatusAction {
                        label: if show_details {
                            "Show Less"
                        } else {
                            "Show More"
                        },
                        key: if mac { "⌃Space" } else { "Ctrl+Space" },
                        run: StatusCommand::ToggleDetails,
                    });
                }
            }
        }
        [left, right]
    }

    /// The status bar text, for tests: the left actions, then the right.
    pub(crate) fn status_bar_text(&self, cx: &App) -> Option<(String, String)> {
        if !self.options.show_status_bar || self.state.message.is_some() {
            return None;
        }
        let [left, right] = self.status_bar_actions(self.details_shown(cx));
        let join = |actions: Vec<StatusAction>| {
            actions
                .iter()
                .map(|a| format!("{} ({})", a.label, a.key))
                .collect::<Vec<_>>()
                .join(", ")
        };
        Some((join(left), join(right)))
    }

    /// Monaco `SuggestWidgetStatus`: the status bar's action bars, whose
    /// text-only actions run their commands when clicked.
    fn render_status_bar(&self, show_details: bool) -> impl IntoElement + use<> {
        let colors = &self.theme.colors;
        let [left, right] = self.status_bar_actions(show_details);
        let editor = self.editor.clone();
        let group = move |name: &'static str, actions: Vec<StatusAction>| {
            let count = actions.len();
            div().flex().overflow_hidden().whitespace_nowrap().children(
                actions.into_iter().enumerate().map(|(ix, action)| {
                    let editor = editor.clone();
                    // `.action-item:not(:last-of-type) .action-label::after
                    // { content: ', ' }`.
                    let separator = if ix + 1 < count { ", " } else { "" };
                    div()
                        .id((name, ix))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| action.run.run(editor, cx));
                            }
                        })
                        .child(format!("{} ({}){separator}", action.label, action.key))
                }),
            )
        };
        div()
            .id("suggest-status-bar")
            .flex_none()
            .h(self.metrics.item_height)
            .px(px(4.))
            .flex()
            .items_center()
            .justify_between()
            .overflow_hidden()
            .border_t_1()
            .border_color(colors.suggest_border)
            .text_size(self.metrics.font_size * 0.8)
            .text_color(colors.suggest_status_foreground)
            // `.left { padding-right: 8px }`.
            .child(group("suggest-status-left", left).pr(px(8.)))
            .child(group("suggest-status-right", right))
    }

    /// The details panel's detail and docs for the focused row: its own,
    /// or explain mode's.
    pub(crate) fn details_content(
        &self,
    ) -> Option<(
        Option<SharedString>,
        Option<crate::providers::Documentation>,
    )> {
        let item = self.state.focused_item()?;
        Some(match &self.state.explanation {
            Some((detail, docs)) => (
                Some(detail.clone()),
                Some(crate::providers::MarkdownString::new(docs.clone()).into()),
            ),
            None => (
                item.detail.clone().filter(|detail| *detail != item.label),
                item.documentation.clone(),
            ),
        })
    }

    /// Monaco `SuggestDetailsWidget`: the detail (the type) in the editor
    /// font, and the docs below it, scrolling inside the panel, with a close
    /// button.
    fn render_details_body(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let colors = &self.theme.colors;
        let (detail, docs) = self.details_content()?;
        // Monaco `SuggestDetailsWidget`: the suggest font size, and its line
        // height over it.
        let style = MarkdownStyle {
            font_size: self.metrics.font_size,
            line_height: f32::from(self.metrics.item_height) / f32::from(self.metrics.font_size),
            foreground: colors.suggest_foreground,
            ..self.style.clone()
        };
        let detail = detail.map(|detail| -> SharedString {
            // Monaco caps the detail at 100000 characters.
            if detail.len() > 100_000 {
                let mut end = 100_000;
                while !detail.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}…", &detail[..end]).into()
            } else {
                detail
            }
        });
        let has_detail = detail.is_some();
        let editor = self.editor.clone();
        Some(
            div()
                .relative()
                .w_full()
                .flex_none()
                .child(
                    div()
                        .id("suggest-details-close")
                        .absolute()
                        // `.codicon-close { top: 6px; right: 2px }`.
                        .top(px(6.))
                        .right(px(2.))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.toggle_suggestion_details(cx);
                                });
                            }
                        })
                        // `suggest-details-close`: `close` at 14px.
                        .child(codicon(Codicon::Close).size(px(14.))),
                )
                .when_some(detail, |this, detail| {
                    // `.header > .type`: the editor font, dimmed. Monaco wraps
                    // it (`auto-wrap`) unless a line after the first is
                    // indented; then each line stays on one line, cut off
                    // with an ellipsis.
                    let indented = detail
                        .split('\n')
                        .skip(1)
                        .any(|line| line.starts_with(char::is_whitespace));
                    this.child(
                        div()
                            .font_family(self.style.code_font.clone())
                            .opacity(0.7)
                            .pt(px(4.))
                            .pb(px(if docs.is_some() { 12. } else { 4. }))
                            .pl(px(5.))
                            .mr(px(24.))
                            .overflow_hidden()
                            .when(indented, |this| this.whitespace_nowrap().text_ellipsis())
                            .child(detail.replace('\r', "")),
                    )
                })
                .when_some(docs, |this, docs| {
                    // `.docs { padding: 4px 5px }`; beside the close button
                    // when there's no type (`.no-type .docs`).
                    this.child(
                        div()
                            .px(px(5.))
                            .py(px(4.))
                            .when(!has_detail, |this| this.mr(px(24.)))
                            .child(render_documentation(&docs, &style, &self.theme, cx)),
                    )
                }),
        )
    }

    /// Resizes the list or the details panel as a sash is dragged. The
    /// list's size is kept for later sessions (Monaco's `suggestWidget.size`,
    /// each edge changing only its own dimension); the details panel's for
    /// this session.
    fn resize(&mut self, sash: Sash, event: &DragMoveEvent<Sash>, cx: &mut Context<Self>) {
        let bounds = event.bounds;
        let position = event.event.position;
        let border = px(2. * BORDER);
        match sash {
            Sash::North | Sash::East | Sash::South => {
                let item_height = self.metrics.item_height;
                let status = self.status_bar_height();
                let stored = cx.try_global::<SuggestUi>().and_then(|ui| ui.size);
                let wanted = stored.unwrap_or_else(|| {
                    size(px(DEFAULT_WIDTH), status + item_height * DEFAULT_ROWS)
                });
                let size = match sash {
                    Sash::East => size(
                        (position.x - bounds.left()).max(px(MIN_WIDTH)),
                        wanted.height,
                    ),
                    Sash::South => size(
                        wanted.width,
                        (position.y - bounds.top() - border).max(item_height + status),
                    ),
                    _ => size(
                        wanted.width,
                        (bounds.bottom() - position.y - border).max(item_height + status),
                    ),
                };
                cx.default_global::<SuggestUi>().size = Some(size);
                self.keep_selected_visible(cx);
            }
            _ => {
                let current = bounds.size;
                self.details_size = Some(match sash {
                    Sash::DetailsEast => size(position.x - bounds.left(), current.height),
                    Sash::DetailsWest => size(bounds.right() - position.x, current.height),
                    Sash::DetailsSouth => size(current.width, position.y - bounds.top()),
                    _ => size(current.width, bounds.bottom() - position.y),
                });
            }
        }
        cx.notify();
    }
}

/// A command in the suggest status bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusCommand {
    /// `selectNextSuggestion`.
    Select,
    /// `acceptSelectedSuggestion`.
    Accept,
    /// `acceptAlternativeSelectedSuggestion`.
    AcceptAlternative,
    /// `toggleSuggestionDetails`.
    ToggleDetails,
}

impl StatusCommand {
    fn run(self, editor: &mut CodeEditor, cx: &mut Context<CodeEditor>) {
        match self {
            StatusCommand::Select => {
                editor.suggest_navigate(1, cx);
            }
            StatusCommand::Accept => {
                editor.accept_suggestion(cx);
            }
            StatusCommand::AcceptAlternative => {
                editor.accept_alternative_suggestion(cx);
            }
            StatusCommand::ToggleDetails => editor.toggle_suggestion_details(cx),
        }
    }
}

/// An action in the suggest status bar (Monaco's
/// `TextOnlyMenuEntryActionViewItem`: "Title (keybinding)").
struct StatusAction {
    label: &'static str,
    key: &'static str,
    run: StatusCommand,
}

/// A 4px resize handle along an edge (Monaco's `Sash`).
fn sash(id: &'static str, sash: Sash) -> gpui::Stateful<gpui::Div> {
    let handle = div()
        .id(id)
        .absolute()
        .on_drag(sash, |_, _, _, cx| cx.new(|_| gpui::Empty));
    match sash {
        Sash::North | Sash::DetailsNorth => handle
            .top(px(-2.))
            .left_0()
            .right_0()
            .h(px(4.))
            .cursor_row_resize(),
        Sash::South | Sash::DetailsSouth => handle
            .bottom(px(-2.))
            .left_0()
            .right_0()
            .h(px(4.))
            .cursor_row_resize(),
        Sash::East | Sash::DetailsEast => handle
            .right(px(-2.))
            .top_0()
            .bottom_0()
            .w(px(4.))
            .cursor_col_resize(),
        Sash::DetailsWest => handle
            .left(px(-2.))
            .top_0()
            .bottom_0()
            .w(px(4.))
            .cursor_col_resize(),
    }
}

impl Render for SuggestWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.slider_drag = None;
        }
        let colors = self.theme.colors.clone();
        let item_height = self.metrics.item_height;
        let border2 = px(2. * BORDER);
        // `suggest.css`: `border-radius: var(--vscode-cornerRadius-large)` and
        // `box-shadow: 0 2px 8px var(--vscode-widget-shadow)`.
        let base = div()
            .rounded(crate::widgets::card::CORNER_RADIUS)
            .shadow(crate::widgets::card::widget_shadow(colors.widget_shadow))
            .bg(colors.suggest_background)
            .border_1()
            .border_color(colors.suggest_border)
            .text_color(colors.suggest_foreground)
            .text_size(self.metrics.font_size)
            .line_height(item_height);
        if let Some(message) = self.state.message {
            // Monaco: half the default width, one row, always below.
            let message_size = size(px(DEFAULT_WIDTH / 2.), item_height + border2);
            self.room.set_anchor(Some(WidgetAnchor {
                preference: ContentWidgetPreference::Below,
                anchor_box: Bounds::new(point(px(0.), px(0.)), message_size),
            }));
            self.details_layout = None;
            return div()
                .child(
                    base.w(message_size.width)
                        .h(message_size.height)
                        .pl(px(22.))
                        .flex()
                        .items_center()
                        .child(message),
                )
                .into_any_element();
        }
        if self.options.fit_width_to_details
            && self
                .fit_width
                .is_none_or(|(model, _)| model != self.state.model_id)
        {
            self.fit_width = Some((self.state.model_id, self.measure_fit_width(window)));
        }
        let layout = self.list_layout(cx);
        self.side = Some(layout.side);
        let list_outer = size(layout.size.width, layout.size.height + border2);
        let visible = self.visible_rows(cx);
        let count = self.state.rows.len();
        self.scroll = self.scroll.min(count.saturating_sub(visible));
        let show_details = self.details_shown(cx);
        let end = (self.scroll + visible).min(count);
        let rows: Vec<_> = (self.scroll..end)
            .map(|ix| self.render_row(ix, show_details).into_any_element())
            .collect();
        let list_scrollbar = self.render_scrollbar(
            ScrollbarTarget::List,
            "suggest-list",
            item_height * visible as f32,
            item_height * count as f32,
            item_height * self.scroll as f32,
            cx,
        );
        let rows_bounds = self.rows_bounds.clone();
        let list = base
            // The rows' bounds, for clicks on the scrollbar's track.
            .on_children_prepainted(move |bounds, _, _| rows_bounds.set(bounds.first().copied()))
            .id("suggest-list")
            .group("suggest-list")
            .flex_none()
            .w(list_outer.width)
            .h(list_outer.height)
            .flex()
            .flex_col()
            .child(
                div()
                    .id("suggest-rows")
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    // The rows use the editor's font (`ItemRenderer`'s
                    // `configureFont`).
                    .font_family(self.style.code_font.clone())
                    .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll_wheel(event, cx)))
                    .on_drag_move(
                        cx.listener(|this, event: &DragMoveEvent<SliderDrag>, _, cx| {
                            if event.drag(cx).target == ScrollbarTarget::List {
                                let top = event.bounds.top();
                                this.drag_slider(event, top, cx);
                            }
                        }),
                    )
                    .children(rows)
                    .children(list_scrollbar),
            )
            .when(self.options.show_status_bar, |this| {
                this.child(self.render_status_bar(show_details))
            })
            // Monaco enables the (north, east) sashes above the cursor and
            // (east, south) below it.
            .child(sash("suggest-sash-east", Sash::East))
            .child(match layout.side {
                ContentWidgetPreference::Above => sash("suggest-sash-north", Sash::North),
                _ => sash("suggest-sash-south", Sash::South),
            })
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<Sash>, _, cx| {
                let sash = *event.drag(cx);
                if matches!(sash, Sash::North | Sash::East | Sash::South) {
                    this.resize(sash, event, cx);
                }
            }));
        let list_rect = Bounds::new(point(px(0.), px(0.)), list_outer);
        let details = show_details
            .then(|| self.render_details_body(cx))
            .flatten()
            .map(|body| {
                let placement = self.place_details(list_outer, layout.side);
                (body.into_any_element(), placement)
            });
        self.details_layout = details.as_ref().map(|(_, placement)| *placement);
        let root = match &details {
            Some((_, placement)) => list_rect.union(&placement.bounds),
            None => list_rect,
        };
        let list_origin = list_rect.origin - root.origin;
        self.room.set_anchor(Some(WidgetAnchor {
            preference: layout.side,
            anchor_box: Bounds::new(list_origin, list_outer),
        }));
        let details = details.map(|(body, placement)| {
            self.render_details(body, placement, placement.bounds.origin - root.origin, cx)
        });
        div()
            .relative()
            .w(root.size.width)
            .h(root.size.height)
            .child(list.absolute().left(list_origin.x).top(list_origin.y))
            .children(details)
            .into_any_element()
    }
}

impl SuggestWidget {
    /// Places the details panel around a list of border box size `list`
    /// on `side` of the cursor line (see [`place_details`]), relative to the
    /// list's top left: at its measured size, or the size it was resized
    /// to.
    fn place_details(&self, list: Size<Pixels>, side: ContentWidgetPreference) -> DetailsLayout {
        let border2 = px(2. * BORDER);
        let content = self
            .details_content
            .get()
            .map_or(list.height, |height| height + border2);
        let wanted = self
            .details_size
            .unwrap_or_else(|| size(px(DETAILS_WIDTH), content));
        let Some(room) = self.room.get() else {
            // Not laid out in a window yet: east, as tall as the list.
            return DetailsLayout {
                bounds: Bounds::new(
                    point(list.width - px(BORDER), px(0.)),
                    size(wanted.width, list.height),
                ),
                side: DetailsSide::East,
                sashes: [false, true, true, false],
            };
        };
        // The window, the list's place in it and the line, from the room
        // the editor published (as the element places the list).
        let horizontal = crate::widgets::OVERFLOW_HORIZONTAL_PADDING;
        let vertical = crate::widgets::OVERFLOW_VERTICAL_PADDING;
        let window_width = room.width + horizontal * 2.;
        let anchor_x = window_width - horizontal - room.right;
        let line_top = room.above + vertical;
        let line_height = self.metrics.line_height;
        let window_height = line_top + line_height + room.below + vertical;
        let list_x = anchor_x
            .min(window_width - horizontal - list.width)
            .max(horizontal);
        let list_y = match side {
            ContentWidgetPreference::Above => line_top - list.height,
            _ => line_top + line_height,
        };
        let anchor = Bounds::new(point(list_x, list_y), list);
        let mut placement = place_details(
            anchor,
            size(window_width, window_height),
            wanted,
            self.metrics.item_height,
            side != ContentWidgetPreference::Above,
        );
        placement.bounds.origin -= anchor.origin;
        placement
    }

    /// The details panel at `origin` in the widget, `placement.bounds` in
    /// size, with its scrollbar and the sashes its placement enables.
    fn render_details(
        &self,
        body: gpui::AnyElement,
        placement: DetailsLayout,
        origin: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = &self.theme.colors;
        let visible = self.details_scroll.bounds().size.height;
        let scrollbar = self.render_scrollbar(
            ScrollbarTarget::Details,
            "suggest-details",
            visible,
            visible + self.details_scroll.max_offset().y,
            -self.details_scroll.offset().y,
            cx,
        );
        let content = self.details_content.clone();
        let measures = self.details_measures.clone();
        let handle = self.details_scroll.clone();
        let entity = cx.entity_id();
        let [north, east, south, west] = placement.sashes;
        div()
            .id("suggest-details")
            .group("suggest-details")
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .w(placement.bounds.size.width)
            .h(placement.bounds.size.height)
            // `.suggest-details { border-radius: var(--vscode-cornerRadius-large) }`.
            .rounded(crate::widgets::card::CORNER_RADIUS)
            .bg(colors.suggest_background)
            .border_1()
            // Monaco outlines the focused details panel.
            .border_color(if self.state.details_focused {
                colors.focus_border
            } else {
                colors.suggest_border
            })
            .text_color(colors.suggest_foreground)
            .text_size(self.metrics.font_size)
            .line_height(self.metrics.item_height)
            .child(
                div()
                    // Measures the content, which sizes the panel next frame
                    // (Monaco lays the panel out at its content's height).
                    // (A scroll container reports its children through its
                    // handle.)
                    .on_children_prepainted(move |_, _, cx| {
                        let Some(height) = handle.bounds_for_item(0).map(|b| b.size.height) else {
                            return;
                        };
                        let changed = content
                            .get()
                            .is_none_or(|last| (last - height).abs() > px(0.5));
                        if changed && measures.get() < MAX_DETAILS_MEASURES {
                            measures.set(measures.get() + 1);
                            content.set(Some(height));
                            cx.notify(entity);
                        }
                    })
                    .id("suggest-details-body")
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.details_scroll)
                    .child(body),
            )
            .children(scrollbar)
            .when(north, |this| {
                this.child(sash("suggest-details-sash-north", Sash::DetailsNorth))
            })
            .when(east, |this| {
                this.child(sash("suggest-details-sash-east", Sash::DetailsEast))
            })
            .when(south, |this| {
                this.child(sash("suggest-details-sash-south", Sash::DetailsSouth))
            })
            .when(west, |this| {
                this.child(sash("suggest-details-sash-west", Sash::DetailsWest))
            })
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<Sash>, _, cx| {
                let sash = *event.drag(cx);
                if matches!(
                    sash,
                    Sash::DetailsNorth | Sash::DetailsEast | Sash::DetailsSouth | Sash::DetailsWest
                ) {
                    this.resize(sash, event, cx);
                }
            }))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<SliderDrag>, _, cx| {
                    if event.drag(cx).target == ScrollbarTarget::Details {
                        let top = event.bounds.top() + px(BORDER);
                        this.drag_slider(event, top, cx);
                    }
                }),
            )
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_css_colors() {
        let hsla = |c: Option<Hsla>| c.map(gpui::Rgba::from);
        let rgba = |r: f32, g: f32, b: f32, a: f32| Some(gpui::Rgba { r, g, b, a });
        assert_eq!(hsla(parse_css_color("#f00")), rgba(1., 0., 0., 1.));
        assert_eq!(hsla(parse_css_color("#0000ff")), rgba(0., 0., 1., 1.));
        assert_eq!(
            hsla(parse_css_color("rgba(255, 0, 0, 0.5)")),
            rgba(1., 0., 0., 0.5)
        );
        assert_eq!(
            hsla(parse_css_color("rgb(0%, 100%, 0%)")),
            rgba(0., 1., 0., 1.)
        );
        let hsl = parse_css_color("hsl(120, 100%, 50%)").unwrap();
        assert!((hsl.h - 1. / 3.).abs() < 1e-4 && hsl.s == 1. && hsl.l == 0.5);
    }

    #[test]
    fn extracts_completion_item_colors() {
        let item = |label: &str, detail: Option<&str>, docs: Option<&str>| CompletionItem {
            detail: detail.map(|d| d.to_string().into()),
            documentation: docs.map(|d| d.to_string().into()),
            ..CompletionItem::new(label.to_string(), CompletionItemKind::Color)
        };
        assert!(completion_item_color(&item("#FFF", None, None)).is_some());
        assert!(completion_item_color(&item("red", Some("#ff0000"), None)).is_some());
        // Docs: only at the start or end.
        assert!(completion_item_color(&item("red", None, Some("The color #ff0000"))).is_some());
        assert!(completion_item_color(&item("red", None, Some("a #ff0000 b"))).is_none());
        // Not exactly a color.
        assert!(completion_item_color(&item("#ff0000 x", None, None)).is_none());
    }

    /// A room of `above` and `below` pixels in a 1000px-wide window.
    fn room(above: f32, below: f32) -> OverflowRoom {
        OverflowRoom {
            above: px(above),
            below: px(below),
            width: px(970.),
            right: px(500.),
        }
    }

    /// 20px rows, the default size, no status bar.
    fn input(rows: usize, room: OverflowRoom) -> ListLayoutInput {
        ListLayoutInput {
            wanted: size(px(DEFAULT_WIDTH), px(240.)),
            resized: false,
            fit_width: None,
            editor_width: Some(px(800.)),
            rows,
            item_height: px(20.),
            status_height: px(0.),
            room: Some(room),
            locked: None,
        }
    }

    /// Monaco `SuggestWidget._layout`.
    #[test]
    fn list_layout_uses_the_room_above_and_below() {
        use ContentWidgetPreference::{Above, Below};
        // Room below: 12 rows below.
        let layout = layout_list(&input(50, room(100., 600.)));
        assert_eq!(layout.side, Below);
        assert_eq!(layout.size, size(px(DEFAULT_WIDTH), px(240.)));
        // Not enough below and more above: above, at the wanted height.
        let layout = layout_list(&input(50, room(600., 100.)));
        assert_eq!(layout.side, Above);
        assert_eq!(layout.size.height, px(240.));
        // Little room either way: the bigger side, capped to whole rows.
        let layout = layout_list(&input(50, room(150., 90.)));
        assert_eq!(layout.side, Above);
        assert_eq!(layout.size.height, px(140.));
        // Few rows fit below even with more room above.
        let layout = layout_list(&input(3, room(600., 100.)));
        assert_eq!(layout.side, Below);
        assert_eq!(layout.size.height, px(60.));
        // At least one row and 220px.
        let mut tiny = input(0, room(10., 10.));
        tiny.wanted.width = px(100.);
        let layout = layout_list(&tiny);
        assert_eq!(layout.size, size(px(MIN_WIDTH), px(20.)));
        // The width is capped to the window, less Monaco's padding.
        let mut wide = input(5, room(100., 600.));
        wide.wanted.width = px(2000.);
        assert_eq!(layout_list(&wide).size.width, px(1000. - 2. - 28.));
    }

    /// Monaco keeps the side while typing: the list shrinking or growing
    /// doesn't flip it.
    #[test]
    fn list_layout_keeps_its_side_while_typing() {
        use ContentWidgetPreference::{Above, Below};
        // 50 rows don't fit below: above.
        let first = layout_list(&input(50, room(600., 100.)));
        assert_eq!(first.side, Above);
        // Typing leaves 3 rows, which would fit below: still above.
        let mut fewer = input(3, room(600., 100.));
        fewer.locked = Some(first.side);
        let layout = layout_list(&fewer);
        assert_eq!(layout.side, Above);
        assert_eq!(layout.size.height, px(60.));
        // And growing back restores the wanted height (`_cappedHeight`).
        let mut more = input(50, room(600., 100.));
        more.locked = Some(Above);
        assert_eq!(layout_list(&more).size.height, px(240.));
        // Below stays below even when more rows would fit above, capped to
        // the room below.
        let mut below = input(50, room(600., 100.));
        below.locked = Some(Below);
        let layout = layout_list(&below);
        assert_eq!(layout.side, Below);
        assert_eq!(layout.size.height, px(80.));
        // Unless not even a row fits there anymore.
        let mut gone = input(50, room(600., 10.));
        gone.locked = Some(Below);
        assert_eq!(layout_list(&gone).side, Above);
    }

    /// `suggest.fitWidthToDetails`.
    #[test]
    fn list_layout_fits_the_width_to_details() {
        let mut fit = input(5, room(100., 600.));
        fit.fit_width = Some(px(600.));
        assert_eq!(layout_list(&fit).size.width, px(600.));
        // Capped to the editor.
        fit.fit_width = Some(px(900.));
        assert_eq!(layout_list(&fit).size.width, px(800.));
        // Never narrower than wanted.
        fit.fit_width = Some(px(100.));
        assert_eq!(layout_list(&fit).size.width, px(DEFAULT_WIDTH));
        // A resized list keeps its size.
        fit.fit_width = Some(px(600.));
        fit.resized = true;
        assert_eq!(layout_list(&fit).size.width, px(DEFAULT_WIDTH));
    }

    /// Monaco `SuggestDetailsOverlay._placeAtAnchor`.
    #[test]
    fn details_go_east_then_west_then_below() {
        let window = size(px(1000.), px(800.));
        let wanted = size(px(330.), px(100.));
        let line = px(20.);
        let at = |x: f32, y: f32| Bounds::new(point(px(x), px(y)), size(px(430.), px(242.)));
        // Room on the right: east, aligned with the list's top, overlapping
        // its border.
        let east = place_details(at(100., 100.), window, wanted, line, true);
        assert_eq!(east.side, DetailsSide::East);
        assert_eq!(east.bounds.origin, point(px(529.), px(100.)));
        assert_eq!(east.bounds.size, wanted);
        assert_eq!(east.sashes, [false, true, true, false]);
        // No room on the right: west, its right edge against the list.
        let west = place_details(at(500., 100.), window, wanted, line, true);
        assert_eq!(west.side, DetailsSide::West);
        assert_eq!(west.bounds.right(), px(499.));
        assert_eq!(west.sashes, [false, false, true, true]);
        // Neither: below the list (above it when the list is above the
        // cursor), as wide as the list.
        let narrow = size(px(700.), px(800.));
        let south = place_details(at(140., 100.), narrow, wanted, line, true);
        assert_eq!(south.side, DetailsSide::South);
        assert_eq!(south.bounds.origin, point(px(140.), px(341.)));
        assert_eq!(south.bounds.size.width, px(428.));
        let north = place_details(at(140., 400.), narrow, wanted, line, false);
        assert_eq!(north.side, DetailsSide::North);
        assert_eq!(north.bounds.bottom(), px(401.));
        assert_eq!(north.sashes, [true, false, false, true]);
        // Too tall for the room below the list's top: aligned with its
        // bottom instead, capped to the room above that.
        let tall = size(px(330.), px(760.));
        let up = place_details(at(100., 500.), window, tall, line, true);
        assert_eq!(up.side, DetailsSide::East);
        assert_eq!(up.bounds.bottom(), px(740.));
        assert_eq!(up.bounds.size.height, px(718.));
        assert_eq!(up.sashes, [true, true, false, false]);
    }

    /// Monaco `ScrollbarState`.
    #[test]
    fn scrollbar_sliders() {
        // Everything visible: no slider.
        assert_eq!(slider(px(100.), px(100.), px(0.)), None);
        // A quarter visible: a quarter-length slider, moving along the rest.
        assert_eq!(slider(px(100.), px(400.), px(0.)), Some((px(0.), px(25.))));
        assert_eq!(
            slider(px(100.), px(400.), px(300.)),
            Some((px(75.), px(25.)))
        );
        // At least 20px long.
        assert_eq!(
            slider(px(100.), px(10000.), px(0.)),
            Some((px(0.), px(20.)))
        );
        // Back from the slider to the scroll position, clamped.
        assert_eq!(slider_position(px(100.), px(400.), px(75.)), px(300.));
        assert_eq!(slider_position(px(100.), px(400.), px(500.)), px(300.));
        assert_eq!(slider_position(px(100.), px(400.), px(-5.)), px(0.));
    }

    /// Monaco's suggest list `getAriaLabel`.
    #[test]
    fn aria_labels() {
        let mut item = CompletionItem::new("push", CompletionItemKind::Method);
        assert_eq!(aria_label(&item, false), "push, Method");
        item.label_detail = Some("(value)".into());
        assert_eq!(aria_label(&item, false), "push (value), Method");
        item.label_description = Some("Vec".into());
        assert_eq!(aria_label(&item, false), "push (value), Vec, Method");
        item.label_detail = None;
        assert_eq!(aria_label(&item, false), "push, Vec, Method");
        item.detail = Some("fn(T)".into());
        item.documentation = Some("Appends.".to_string().into());
        assert_eq!(
            aria_label(&item, true),
            "push, Vec, Method, docs: fn(T)Appends."
        );
    }
}
