//! Diagnostics markers (Monaco `monaco.editor.setModelMarkers`): problems
//! pushed by the host, rendered as squiggles, overview ruler marks and hover
//! messages, and navigable with F8 / Shift+F8.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement, PathBuilder, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Window, canvas, div, point, px, rgb, rgba,
};
use text::PointUtf16;

use crate::codicons::{Codicon, icon as codicon};
use crate::decorations::{
    DecorationId, DecorationOptions, DecorationUnderline, OverviewRulerDecoration,
    OverviewRulerLane,
};
use crate::editor::CodeEditor;
use crate::model::TextModel;
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};

/// Monaco `MarkerSeverity`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MarkerSeverity {
    /// A hint; no squiggle, only a hover message.
    Hint,
    /// Information: blue squiggle.
    Info,
    /// A warning: yellow squiggle and overview ruler mark.
    Warning,
    /// An error: red squiggle and overview ruler mark.
    #[default]
    Error,
}

/// Monaco `MarkerTag`: extra meaning a diagnostic gives its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkerTag {
    /// Unused or unnecessary code: faded (`show_unused`).
    Unnecessary,
    /// Deprecated code: struck through (`show_deprecated`).
    Deprecated,
}

/// Monaco `IRelatedInformation`: another place a diagnostic refers to,
/// listed in its hover and marker zone as a link.
#[derive(Clone, Debug, PartialEq)]
pub struct RelatedInformation {
    /// The other document's URI; `None` for this one.
    pub uri: Option<SharedString>,
    /// The range there.
    pub range: Range<PointUtf16>,
    /// What's there.
    pub message: SharedString,
}

impl RelatedInformation {
    /// `file.rs(3, 5): `, Monaco's link before the message. `this_uri`
    /// names the file when the information is in the same model.
    pub(crate) fn location_label(&self, this_uri: Option<&str>) -> String {
        let name = self
            .uri
            .as_deref()
            .or(this_uri)
            .map(basename)
            .unwrap_or_default();
        format!(
            "{name}({}, {}): ",
            self.range.start.row + 1,
            self.range.start.column + 1,
        )
    }

    pub(crate) fn location(&self) -> crate::providers::Location {
        crate::providers::Location {
            uri: self.uri.clone(),
            range: self.range.clone(),
            ..Default::default()
        }
    }
}

/// A marker's code after its source: `(code)`, or with a target Monaco's
/// `a.code-link`, the code underlined in `link` between the parentheses,
/// opening the target through the editor's link openers.
pub(crate) fn code_element(
    code: &SharedString,
    target: Option<&SharedString>,
    link: Hsla,
    editor: gpui::WeakEntity<CodeEditor>,
) -> gpui::AnyElement {
    let Some(target) = target.cloned() else {
        return div().child(format!("({code})")).into_any_element();
    };
    div()
        .flex()
        .child("(")
        .child(
            div()
                .id("marker-code-link")
                .debug_selector(|| "marker-code-link".into())
                .role(gpui::Role::Link)
                .cursor_pointer()
                .underline()
                .text_color(link)
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = editor.upgrade() {
                        let target = target.clone();
                        editor.update(cx, |editor, cx| editor.open_link_with_openers(target, cx));
                    }
                })
                .child(code.clone()),
        )
        .child(")")
        .into_any_element()
}

/// The last path segment of `uri` (Monaco `basename`).
fn basename(uri: &str) -> String {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    path.rsplit('/').next().unwrap_or(path).to_string()
}

impl MarkerSeverity {
    /// Monaco's `vs-dark` `editorError/Warning/Info/Hint.foreground`. The
    /// editor draws markers in its theme's colors instead
    /// ([`EditorColors::marker_color`](crate::EditorColors::marker_color)).
    pub fn color(self) -> Hsla {
        match self {
            MarkerSeverity::Error => rgb(0xf14c4c).into(),
            MarkerSeverity::Warning => rgb(0xcca700).into(),
            MarkerSeverity::Info => rgb(0x59a4f9).into(),
            MarkerSeverity::Hint => rgba(0xeeeeeeb3).into(),
        }
    }
}

/// Monaco `IMarkerData`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marker {
    /// How serious it is; picks the color and whether it's marked in the overview ruler.
    pub severity: MarkerSeverity,
    /// The message, shown on hover and by marker navigation.
    pub message: SharedString,
    /// The range (0-based rows, UTF-16 columns). An empty range is widened to one
    /// character.
    pub range: Range<PointUtf16>,
    /// E.g. `rustc`; shown after the message.
    pub source: Option<SharedString>,
    /// E.g. a lint name; shown after the message with the source.
    pub code: Option<SharedString>,
    /// Where the code links to (Monaco's `code: { value, target }`), e.g. the
    /// lint's documentation: the hover and the marker zone show the code as
    /// a link that opens it through the editor's link openers.
    pub code_target: Option<SharedString>,
    /// Unnecessary or deprecated code.
    pub tags: Vec<MarkerTag>,
    /// Other places the diagnostic refers to.
    pub related_information: Vec<RelatedInformation>,
}

impl Marker {
    fn has_tag(&self, tag: MarkerTag) -> bool {
        self.tags.contains(&tag)
    }
}

/// Markers of one model, by owner (e.g. one per linter), in the order the
/// owners first set them (Monaco's `DoubleResourceMap` keeps insertion
/// order).
#[derive(Default)]
pub(crate) struct MarkerSet {
    by_owner: Vec<(SharedString, Vec<Marker>, Vec<DecorationId>)>,
}

/// Monaco `MarkerService._toMarker`: drops markers without a message and
/// moves an end before the start to the start's line.
fn sanitize(markers: Vec<Marker>) -> Vec<Marker> {
    markers
        .into_iter()
        .filter(|marker| !marker.message.is_empty())
        .map(|mut marker| {
            if marker.range.end.row < marker.range.start.row {
                marker.range.end.row = marker.range.start.row;
            }
            marker
        })
        .collect()
}

impl TextModel {
    /// Replaces the markers of `owner` (Monaco `setModelMarkers`). Markers
    /// without a message are dropped. Emits
    /// [`TextModelEvent::MarkersChanged`](crate::TextModelEvent::MarkersChanged)
    /// and the global [`on_did_change_markers`](crate::on_did_change_markers)
    /// unless `owner` had no markers and gets none.
    pub fn set_markers(
        &mut self,
        owner: impl Into<SharedString>,
        markers: Vec<Marker>,
        cx: &mut Context<Self>,
    ) {
        let owner = owner.into();
        let index = self.markers.by_owner.iter().position(|(o, ..)| *o == owner);
        // Monaco `changeOne`: removing nothing isn't a change.
        if index.is_none() && markers.is_empty() {
            return;
        }
        crate::registry::track_model(&cx.entity(), cx);
        let markers = sanitize(markers);
        let old = index
            .map(|i| std::mem::take(&mut self.markers.by_owner[i].2))
            .unwrap_or_default();
        let buffer = self.buffer();
        let decorations = markers
            .iter()
            .map(|marker| {
                let start = buffer.point_utf16_to_offset(
                    buffer.clip_point_utf16(marker.range.start, text::Bias::Left),
                );
                // Monaco never draws untagged hints on several lines, and
                // gives them room for their three dots.
                let tagged = !marker.tags.is_empty();
                let end_point = if marker.severity == MarkerSeverity::Hint && !tagged {
                    PointUtf16::new(marker.range.start.row, marker.range.start.column + 2)
                } else {
                    marker.range.end
                };
                let mut end = buffer
                    .point_utf16_to_offset(buffer.clip_point_utf16(end_point, text::Bias::Right));
                // Monaco widens empty markers to the word or character there.
                if end <= start {
                    end = buffer
                        .clip_offset(start + 1, text::Bias::Right)
                        .min(buffer.len());
                }
                let color = marker.severity.color();
                // Hints have no squiggle; an unnecessary one is underlined
                // only by themes with an `editorUnnecessaryCode.border` (see
                // the element).
                let underline = (marker.severity != MarkerSeverity::Hint)
                    .then_some(DecorationUnderline { color, wavy: true });
                (
                    start..end,
                    DecorationOptions {
                        underline,
                        // Monaco marks infos too.
                        overview_ruler: (marker.severity >= MarkerSeverity::Info)
                            .then(|| OverviewRulerDecoration::new(color, OverviewRulerLane::Right)),
                        // Monaco marks infos, warnings and errors in the
                        // minimap, in the theme's colors for the severity.
                        minimap: (marker.severity >= MarkerSeverity::Info)
                            .then(crate::MinimapDecoration::default),
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        z_index: 10 + marker.severity as i32,
                        validation: true,
                        severity: Some(marker.severity),
                        // Monaco: deprecated wins over unnecessary.
                        unnecessary: marker.has_tag(MarkerTag::Unnecessary)
                            && !marker.has_tag(MarkerTag::Deprecated),
                        deprecated: marker.has_tag(MarkerTag::Deprecated),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let ids = self.delta_decorations(None, &old, decorations, cx);
        match index {
            Some(i) if markers.is_empty() => {
                self.markers.by_owner.remove(i);
            }
            Some(i) => self.markers.by_owner[i] = (owner, markers, ids),
            None if markers.is_empty() => {}
            None => self.markers.by_owner.push((owner, markers, ids)),
        }
        cx.emit(crate::TextModelEvent::MarkersChanged);
        if let Some(uri) = self.uri().cloned() {
            crate::registry::markers_changed(uri, cx);
        }
    }

    /// Whether any owner has markers here.
    pub(crate) fn has_markers(&self) -> bool {
        !self.markers.by_owner.is_empty()
    }

    /// The markers of each owner as they were set (Monaco's marker service
    /// data, which doesn't follow edits), in owner order.
    pub(crate) fn markers_by_owner(&self) -> impl Iterator<Item = (&SharedString, &[Marker])> {
        self.markers
            .by_owner
            .iter()
            .map(|(owner, markers, _)| (owner, markers.as_slice()))
    }

    /// The markers whose (current) range contains byte `offset`, most severe
    /// first.
    pub fn markers_at(&self, offset: usize) -> Vec<Marker> {
        let buffer = self.buffer();
        let mut found: Vec<Marker> = self
            .markers
            .by_owner
            .iter()
            .flat_map(|(_, markers, ids)| markers.iter().zip(ids))
            .filter_map(|(marker, id)| {
                let range = &self.decoration(*id)?.range;
                (range.start <= offset && offset <= range.end).then(|| {
                    let mut marker = marker.clone();
                    marker.range = buffer.offset_to_point_utf16(range.start)
                        ..buffer.offset_to_point_utf16(range.end);
                    marker
                })
            })
            .collect();
        found.sort_by_key(|m| (std::cmp::Reverse(m.severity), m.range.start));
        found
    }

    /// All markers with their current ranges (they move with edits), sorted
    /// by position.
    pub fn markers(&self) -> Vec<Marker> {
        let buffer = self.buffer();
        let mut all: Vec<Marker> = self
            .markers
            .by_owner
            .iter()
            .flat_map(|(_, markers, ids)| {
                markers.iter().zip(ids).map(|(marker, id)| {
                    let mut marker = marker.clone();
                    if let Some(decoration) = self.decoration(*id) {
                        marker.range = buffer.offset_to_point_utf16(decoration.range.start)
                            ..buffer.offset_to_point_utf16(decoration.range.end);
                    }
                    marker
                })
            })
            .collect();
        all.sort_by_key(|m| (m.range.start, std::cmp::Reverse(m.severity)));
        all
    }

    /// The markers F8 visits (Monaco `MarkerList`): errors, warnings and
    /// infos, not hints, most severe first and then by range (Monaco's
    /// `problems.sortOrder: 'severity'`, as standalone Monaco has no such
    /// setting). Ranges follow edits; an empty one stays empty (not widened
    /// as its decoration is).
    pub(crate) fn navigable_markers(&self) -> Vec<Marker> {
        let buffer = self.buffer();
        let mut all: Vec<Marker> = self
            .markers
            .by_owner
            .iter()
            .flat_map(|(_, markers, ids)| markers.iter().zip(ids))
            .filter(|(marker, _)| marker.severity >= MarkerSeverity::Info)
            .map(|(marker, id)| {
                let mut marker = marker.clone();
                if let Some(decoration) = self.decoration(*id) {
                    let start = buffer.offset_to_point_utf16(decoration.range.start);
                    let end = if marker.range.is_empty() {
                        start
                    } else {
                        buffer.offset_to_point_utf16(decoration.range.end)
                    };
                    marker.range = start..end;
                }
                marker
            })
            .collect();
        // Monaco `MarkerSeverity.compare`, then `compareRangesUsingStarts`.
        all.sort_by_key(|m| (std::cmp::Reverse(m.severity), m.range.start, m.range.end));
        all
    }
}

/// Monaco `IMarker`: a marker with its owner and the uri of its model.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMarker {
    /// Who set it (Monaco `owner`).
    pub owner: SharedString,
    /// The model's uri (Monaco `resource`); `None` for a model without one.
    pub resource: Option<SharedString>,
    /// The marker as it was set: its range doesn't follow edits, as in
    /// Monaco's marker service (see [`TextModel::markers`] for the current
    /// ranges).
    pub marker: Marker,
}

/// Which markers [`model_markers`] returns (Monaco `getModelMarkers`'
/// filter).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkerFilter {
    /// Only this owner's.
    pub owner: Option<SharedString>,
    /// Only those of the model with this uri.
    pub resource: Option<SharedString>,
    /// At most this many; `None` or `Some(0)` for all.
    pub take: Option<usize>,
}

/// The markers of every live model matching `filter` (Monaco
/// `editor.getModelMarkers`), by model in creation order, then by owner in
/// the order they first set markers.
pub fn model_markers(cx: &App, filter: &MarkerFilter) -> Vec<ModelMarker> {
    let take = filter.take.filter(|&take| take > 0).unwrap_or(usize::MAX);
    let mut result = Vec::new();
    for model in crate::registry::models(cx) {
        let model = model.read(cx);
        let resource = model.uri();
        if filter
            .resource
            .as_ref()
            .is_some_and(|uri| resource != Some(uri))
        {
            continue;
        }
        for (owner, markers) in model.markers_by_owner() {
            if filter.owner.as_ref().is_some_and(|o| o != owner) {
                continue;
            }
            for marker in markers {
                if result.len() == take {
                    return result;
                }
                result.push(ModelMarker {
                    owner: owner.clone(),
                    resource: resource.cloned(),
                    marker: marker.clone(),
                });
            }
        }
    }
    result
}

/// Removes `owner`'s markers from every live model (Monaco
/// `editor.removeAllMarkers`), with one
/// [`on_did_change_markers`](crate::on_did_change_markers) event for all of
/// them.
pub fn remove_all_markers(cx: &mut App, owner: &str) {
    for model in crate::registry::models(cx) {
        if model
            .read(cx)
            .markers_by_owner()
            .any(|(o, _)| o.as_ref() == owner)
        {
            model.update(cx, |model, cx| {
                model.set_markers(owner.to_string(), Vec::new(), cx)
            });
        }
    }
}

/// Monaco `MessageWidget.getHeightInLines`: the body shows at most this
/// many lines and scrolls the rest.
const MAX_BODY_LINES: usize = 17;

/// The inline problem view shown by F8 / Alt+F8 and "View Problem" (Monaco
/// `MarkerNavigationWidget`, a peek view below the marker's line): a header
/// with the file name, "2 of 4 problems" and next/previous/close, and a
/// scrollable body with the message, its source and code, and related
/// information.
pub struct MarkerZone {
    marker: Marker,
    /// Monaco `_severity`: the frame, header and icon's, set when shown
    /// (`updateMarker` leaves them, and the title).
    severity: MarkerSeverity,
    /// Where the zone shows: the cursor, if it's in the marker, or its start.
    position: PointUtf16,
    /// Monaco `showStale`: the markers changed and none is at the zone's
    /// position any more.
    stale: bool,
    /// Monaco `MessageWidget.getAriaLabel`, the message's label.
    aria_label: SharedString,
    /// Monaco's `tabIndex = 0` body.
    focus_handle: gpui::FocusHandle,
    /// Whether the body had focus at the last render.
    body_focused: bool,
    /// Monaco `showAtMarker`'s `revealPositionInCenterIfOutsideViewport`,
    /// run once the zone is laid out, after `ZoneWidget.show`'s reveal.
    reveal: Option<PointUtf16>,
    index: usize,
    total: usize,
    /// The model's uri, for the title and same-file related information.
    uri: Option<SharedString>,
    colors: crate::theme::EditorColors,
    font_family: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    editor: gpui::WeakEntity<CodeEditor>,
    scroll: ScrollHandle,
    /// Where it was painted.
    bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<Pixels>>>>,
}

impl MarkerZone {
    /// Monaco `MessageWidget.update`'s line count: the message's lines, then
    /// a spacer and one line per related information.
    fn content_lines(marker: &Marker) -> usize {
        let mut lines = marker.message.split('\n').count();
        if !marker.related_information.is_empty() {
            lines += 1 + marker.related_information.len();
        }
        lines
    }

    /// Monaco `computeRequiredHeight`: 3 lines (header, arrow, frame and
    /// padding) plus the body, at most 17 lines of it.
    fn required_height_in_lines(marker: &Marker) -> f32 {
        3. + Self::content_lines(marker).min(MAX_BODY_LINES) as f32
    }

    /// Monaco `SeverityIcon.className`: the `error`, `warning` and `info`
    /// codicons.
    fn severity_icon(severity: MarkerSeverity) -> Codicon {
        match severity {
            MarkerSeverity::Error => Codicon::Error,
            MarkerSeverity::Warning => Codicon::Warning,
            MarkerSeverity::Info | MarkerSeverity::Hint => Codicon::Info,
        }
    }

    fn header_action(
        &self,
        id: &'static str,
        glyph: Codicon,
        label: &'static str,
        run: fn(&mut CodeEditor, &mut Context<CodeEditor>),
    ) -> gpui::AnyElement {
        let editor = self.editor.clone();
        div()
            .id(id)
            .px(px(4.))
            .rounded(px(3.))
            .cursor_pointer()
            .text_color(self.colors.peek_view_title_foreground)
            .hover(|style| self.colors.toolbar_hover(style))
            .tooltip(crate::tooltip::tooltip(label, &self.colors))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, run);
                }
            })
            .child(codicon(glyph))
            .into_any_element()
    }
}

impl Render for MarkerZone {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.body_focused = self.focus_handle.is_focused(window);
        // The second of Monaco's two reveals, from where the first left the
        // scroll position.
        if let Some(position) = self.reveal.take() {
            let editor = self.editor.clone();
            cx.defer(move |cx| {
                editor
                    .update(cx, |editor, cx| {
                        editor.reveal_position_in_center_if_outside_viewport(position, cx)
                    })
                    .ok();
            });
        }
        let colors = self.colors.clone();
        // Monaco `editorMarkerNavigation{Error,Warning,Info}.background` and
        // `.headerBackground`.
        let (frame, header_background) = colors.marker_navigation_colors(self.severity);
        let icon = colors.marker_color(self.severity);
        let line_height = self.line_height;
        // Monaco: the arrow is a third of a line tall, under the marker.
        let arrow_height = (line_height / 3.).round();
        let arrow_x = self
            .editor
            .upgrade()
            .and_then(|editor| {
                let editor = editor.read(cx);
                let left = editor.layout.as_ref()?.bounds.left();
                let position = editor.scrolled_visible_position(self.position)?;
                Some(position.x - left)
            })
            .unwrap_or(px(30.));
        let detail = format!(
            "{} of {} problem{}",
            self.index + 1,
            self.total,
            if self.total > 1 { "s" } else { "" }
        );
        let title = self.uri.as_deref().map(basename).unwrap_or_default();
        let link = colors.text_link_foreground;
        let painted = self.bounds.clone();

        // Monaco `MessageWidget`: one row per message line, the source and
        // code after the last, dimmed.
        let lines: Vec<String> = self
            .marker
            .message
            .split('\n')
            .map(str::to_string)
            .collect();
        let last = lines.len() - 1;
        // Monaco's `.message`: `role="alert"` (whose live region is
        // assertive, Monaco's `aria-live`), labelled for screen readers.
        let mut message = div()
            .id("marker-message")
            .role(gpui::Role::Alert)
            .aria_label(self.aria_label.clone())
            .flex()
            .flex_col();
        for (ix, line) in lines.into_iter().enumerate() {
            let mut row = div().h(line_height).flex().whitespace_nowrap().child(line);
            if ix == last && (self.marker.source.is_some() || self.marker.code.is_some()) {
                row = row.child(
                    div()
                        .pl(px(6.))
                        .flex()
                        .opacity(0.6)
                        .children(self.marker.source.clone())
                        .children(self.marker.code.as_ref().map(|code| {
                            code_element(
                                code,
                                self.marker.code_target.as_ref(),
                                colors.text_link_active_foreground,
                                self.editor.clone(),
                            )
                        })),
                );
            }
            message = message.child(row);
        }
        let related = (!self.marker.related_information.is_empty()).then(|| {
            div()
                .pt((line_height * 0.66).floor())
                .flex()
                .flex_col()
                .children(self.marker.related_information.iter().enumerate().map(
                    |(ix, related)| {
                        let editor = self.editor.clone();
                        let location = related.location();
                        div()
                            .h(line_height)
                            .flex()
                            .whitespace_nowrap()
                            .child(
                                div()
                                    .id(("marker-related", ix))
                                    .cursor_pointer()
                                    .text_color(link)
                                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                        cx.stop_propagation();
                                        if let Some(editor) = editor.upgrade() {
                                            editor.update(cx, |editor, cx| {
                                                editor.open_location(&location, true, cx);
                                            });
                                        }
                                    })
                                    .child(related.location_label(self.uri.as_deref())),
                            )
                            .child(related.message.clone())
                    },
                ))
        });

        let header = div()
            .h((line_height * 1.2).ceil())
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .bg(header_background)
            .text_size(px(13.))
            .child(
                div()
                    .text_color(icon)
                    .child(codicon(Self::severity_icon(self.severity))),
            )
            .when(!title.is_empty(), |this| {
                this.child(
                    div()
                        .text_color(colors.peek_view_title_foreground)
                        .child(title),
                )
            })
            .child(
                div()
                    .flex_1()
                    .text_color(colors.peek_view_title_description_foreground)
                    .child(detail),
            )
            .child(self.header_action(
                "marker-next",
                // `marker-navigation-next` / `-previous`: `arrow-down` and
                // `arrow-up`.
                Codicon::ArrowDown,
                "Go to Next Problem (Error, Warning, Info)",
                |editor, cx| editor.go_to_marker(true, cx),
            ))
            .child(self.header_action(
                "marker-previous",
                Codicon::ArrowUp,
                "Go to Previous Problem (Error, Warning, Info)",
                |editor, cx| editor.go_to_marker(false, cx),
            ))
            .child(
                self.header_action("marker-close", Codicon::Close, "Close", |editor, cx| {
                    editor.close_marker_zone(cx);
                }),
            );

        // Monaco's `ScrollableElement` around the description: both axes,
        // since lines don't wrap.
        let body = div()
            .id("marker-body")
            // Monaco `.marker-widget`: `tabIndex = 0`, `role="tooltip"`.
            .track_focus(&self.focus_handle)
            .role(gpui::Role::Tooltip)
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .bg(colors.marker_navigation_background)
            .text_color(colors.foreground)
            .font_family(self.font_family.clone())
            .text_size(self.font_size)
            .line_height(line_height)
            // The editor would scroll too: GPUI doesn't stop the wheel event.
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .pt(px(8.))
                    .pr(px(12.))
                    .pl(px(20.))
                    .flex()
                    .flex_col()
                    // Monaco `.marker-widget > .stale`.
                    .when(self.stale, |this| this.opacity(0.6).italic())
                    .child(message)
                    .children(related),
            );

        div()
            .size_full()
            .relative()
            // Clicks in the widget don't reach the text behind it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        painted.set(Some(bounds));
                        let x = bounds.left() + arrow_x;
                        let top = bounds.top();
                        let mut path = PathBuilder::fill();
                        path.move_to(point(x - arrow_height, top + arrow_height));
                        path.line_to(point(x, top));
                        path.line_to(point(x + arrow_height, top + arrow_height));
                        path.close();
                        if let Ok(path) = path.build() {
                            window.paint_path(path, frame);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .top(arrow_height)
                    .flex()
                    .flex_col()
                    // Monaco `frameWidth: 1`.
                    .border_t_1()
                    .border_b_1()
                    .border_color(frame)
                    .child(header)
                    .child(body),
            )
    }
}

/// Monaco `Range.containsPosition`: both ends included.
fn contains(range: &Range<PointUtf16>, position: PointUtf16) -> bool {
    range.start <= position && position <= range.end
}

/// Monaco `MessageWidget.getAriaLabel`: `"<line>, Error: msg at 3:5. "`.
fn aria_label(model: &TextModel, marker: &Marker) -> SharedString {
    let severity = match marker.severity {
        MarkerSeverity::Error => "Error",
        MarkerSeverity::Warning => "Warning",
        MarkerSeverity::Info => "Info",
        MarkerSeverity::Hint => "Hint",
    };
    let start = marker.range.start;
    let label = format!(
        "{severity}: {} at {}:{}. ",
        marker.message,
        start.row + 1,
        start.column + 1
    );
    if start.row < model.line_count() {
        format!("{}, {label}", model.line_content(start.row)).into()
    } else {
        label.into()
    }
}

/// Monaco `MarkerList` for the editor's model: the markers F8 visits and
/// the one shown.
struct MarkerList {
    markers: Vec<Marker>,
    /// Monaco `_nextIdx`, `None` for `-1`: the next move starts from the
    /// cursor.
    next: Option<usize>,
}

impl MarkerList {
    fn selected(&self) -> Option<&Marker> {
        self.markers.get(self.next?)
    }

    /// Monaco `MarkerList.move`: the first move takes the marker at or
    /// after `position` (before it, backwards), later ones step, so
    /// markers starting at the same place are each visited.
    fn step(&mut self, forward: bool, model: &TextModel, position: PointUtf16) {
        let len = self.markers.len();
        self.next = Some(match self.next {
            None => self.initial_index(forward, model, position),
            Some(ix) if forward => (ix + 1) % len,
            Some(ix) => (ix + len - 1) % len,
        });
    }

    /// Monaco `_initIdx` in one model: the first marker, in severity
    /// order, containing `position` (an empty one as its word) or after
    /// it.
    fn initial_index(&self, forward: bool, model: &TextModel, position: PointUtf16) -> usize {
        let last = self.markers.len() - 1;
        let found = self.markers.iter().enumerate().find_map(|(ix, marker)| {
            let mut range = marker.range.clone();
            if range.is_empty()
                && let Some(word) = model.word_at_position(range.start)
            {
                range = word.range;
            }
            let inside = contains(&range, position);
            (inside || position <= range.start).then_some((ix, !inside))
        });
        match found {
            None if forward => 0,
            None => last,
            // Went past it: backwards, the one before.
            Some((ix, true)) if !forward => ix.checked_sub(1).unwrap_or(last),
            Some((ix, _)) => ix,
        }
    }
}

/// Monaco `MarkerController`'s session: from the first move until the zone
/// closes.
#[derive(Default)]
pub(crate) struct MarkerNavigation {
    zone: Option<(Entity<MarkerZone>, WidgetId)>,
    list: Option<MarkerList>,
    /// Monaco `ZoneWidget.position`: where the zone shows, following edits.
    position: Option<text::Anchor>,
}

/// Monaco `MarkerList`'s `updateMarker` comparison.
fn same_markers(a: &[Marker], b: &[Marker]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| a.range == b.range && a.severity == b.severity && a.message == b.message)
}

impl CodeEditor {
    /// Whether diagnostics are drawn (Monaco `renderValidationDecorations`).
    pub fn shows_validation_decorations(&self) -> bool {
        match self.options.render_validation_decorations {
            crate::options::RenderValidationDecorations::Editable => !self.options.read_only,
            crate::options::RenderValidationDecorations::On => true,
            crate::options::RenderValidationDecorations::Off => false,
        }
    }

    /// Moves to the next (or previous) error, warning or info and shows it
    /// below its line (Monaco `editor.action.marker.next`, F8). Markers go
    /// most severe first, then by position; the first move takes the one
    /// at the cursor.
    pub fn go_to_marker(&mut self, next: bool, cx: &mut Context<Self>) {
        let model = self.model().clone();
        let model = model.read(cx);
        let markers = model.navigable_markers();
        if markers.is_empty() {
            return;
        }
        let list = self.marker_navigation.list.get_or_insert(MarkerList {
            markers: Vec::new(),
            next: None,
        });
        // Ranges follow edits; other changes reset the list when they come.
        if list.markers.len() != markers.len() {
            list.next = None;
        }
        list.markers = markers;
        let head = self.cursors.primary().position;
        list.step(next, model, head);
        if let Some(index) = list.next {
            self.show_marker_at(index, cx);
        }
    }

    /// Selects `marker` and shows it in the marker zone (Monaco's "View
    /// Problem" hover action). A hint shows the marker F8 would go to from
    /// its start.
    pub fn show_marker(&mut self, marker: &Marker, cx: &mut Context<Self>) {
        let model = self.model().clone();
        let model = model.read(cx);
        let markers = model.navigable_markers();
        if markers.is_empty() {
            return;
        }
        let same = |m: &Marker| {
            m.range.start == marker.range.start
                && m.severity == marker.severity
                && m.message == marker.message
        };
        let mut list = MarkerList {
            next: markers.iter().position(same),
            markers,
        };
        if list.next.is_none() {
            list.step(true, model, marker.range.start);
        }
        let index = list.next;
        self.marker_navigation.list = Some(list);
        if let Some(index) = index {
            self.show_marker_at(index, cx);
        }
    }

    /// Monaco `MarkerNavigationWidget.showAtMarker`.
    fn show_marker_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(list) = &self.marker_navigation.list else {
            return;
        };
        let marker = list.markers[index].clone();
        let total = list.markers.len();
        // The cursor, if it's in the marker, else its start.
        let head = self.cursors.primary().position;
        let position = if contains(&marker.range, head) {
            head
        } else {
            marker.range.start
        };
        self.remove_marker_zone(cx);
        // `ZoneWidget.show` selects the position.
        self.set_selections(&[crate::cursor::Selection::collapsed(position)], cx);
        // Monaco `ZoneWidget._getMaximumHeightInLines`: 80% of the editor,
        // at least 12 lines.
        let line_height = self.options.resolved_line_height();
        let editor_lines = self
            .layout_size()
            .map_or(0., |size| size.height / line_height);
        let max_lines = (editor_lines * 0.8).max(12.);
        let lines = MarkerZone::required_height_in_lines(&marker).min(max_lines);
        let editor = cx.entity().downgrade();
        let model = self.model().read(cx);
        let uri = model.uri().cloned();
        let line_count = model.line_count();
        let label = aria_label(model, &marker);
        let anchor = {
            let buffer = model.buffer();
            let position = buffer.clip_point_utf16(position, text::Bias::Left);
            buffer.anchor_before(buffer.point_utf16_to_offset(position))
        };
        let zone = cx.new(|cx| MarkerZone {
            marker: marker.clone(),
            severity: marker.severity,
            position,
            stale: false,
            aria_label: label.clone(),
            focus_handle: cx.focus_handle(),
            body_focused: false,
            reveal: Some(position),
            index,
            total,
            uri,
            colors: self.theme.colors.clone(),
            font_family: self.options.font_family.clone(),
            font_size: self.options.font_size,
            line_height,
            editor,
            scroll: ScrollHandle::new(),
            bounds: Default::default(),
        });
        // Monaco `ViewZoneDelegate`: after the position's line and column.
        let mut view_zone = ViewZone::new(
            position.row + 1,
            ZoneHeight::Lines(lines),
            zone.clone().into(),
        );
        view_zone.after_column = Some(position.column);
        let id = self.add_view_zone(view_zone, cx);
        // A zone widget in Monaco, outside the scrollable element.
        self.widgets.set_zone_captures_wheel(id);
        self.marker_navigation.zone = Some((zone, id));
        self.marker_navigation.position = Some(anchor);
        // `ZoneWidget.show` reveals the line and the zone under it (the last
        // line near the top); the zone then reveals the position.
        if position.row + 1 >= line_count {
            self.reveal_line_near_top(position.row, cx);
        } else {
            self.reveal_range(
                PointUtf16::new(position.row, 0)..PointUtf16::new(position.row + 1, 0),
                crate::RevealType::Simple,
                cx,
            );
        }
        // Monaco `MessageWidget.update`'s `aria.status`.
        self.announce(label, cx);
        // And `showAtMarker` focuses the editor.
        self.refocus = true;
    }

    /// Monaco `MarkerController`'s cursor listener: moving off the shown
    /// marker makes the next move start from the cursor again.
    pub(crate) fn marker_navigation_cursor_moved(&mut self, position: PointUtf16) {
        if let Some(list) = &mut self.marker_navigation.list
            && !list
                .selected()
                .is_some_and(|m| contains(&m.range, position))
        {
            list.next = None;
        }
    }

    /// Monaco `MarkerController`'s marker listener: when the markers
    /// change, the zone shows the marker now at its position, or turns
    /// stale (faded and italic) when there's none.
    pub(crate) fn marker_navigation_markers_changed(&mut self, cx: &mut Context<Self>) {
        if self.marker_navigation.list.is_none() {
            return;
        }
        let model = self.model().clone();
        let model = model.read(cx);
        let navigation = &mut self.marker_navigation;
        let Some(list) = &mut navigation.list else {
            return;
        };
        let markers = model.navigable_markers();
        if same_markers(&list.markers, &markers) {
            return;
        }
        list.markers = markers;
        list.next = None;
        let (Some((zone, _)), Some(anchor)) = (&navigation.zone, &navigation.position) else {
            return;
        };
        let buffer = model.buffer();
        let position = buffer.offset_to_point_utf16(buffer.resolve(anchor));
        let found = list
            .markers
            .iter()
            .find(|m| contains(&m.range, position))
            .map(|marker| (marker.clone(), aria_label(model, marker)));
        let zone = zone.clone();
        let announce = found.as_ref().map(|(_, label)| label.clone());
        zone.update(cx, |zone, cx| {
            match found {
                // Monaco `updateMarker`: the message only.
                Some((marker, label)) => {
                    zone.marker = marker;
                    zone.aria_label = label;
                    zone.stale = false;
                }
                None => zone.stale = true,
            }
            cx.notify();
        });
        if let Some(label) = announce {
            self.announce(label, cx);
        }
    }

    /// The marker zone's painted bounds, its height in lines and its body's
    /// scroll offset, for tests.
    #[doc(hidden)]
    pub fn marker_zone_for_tests(
        &self,
        cx: &App,
    ) -> Option<(gpui::Bounds<Pixels>, f32, gpui::Point<Pixels>)> {
        let (zone, _) = self.marker_navigation.zone.as_ref()?;
        let zone = zone.read(cx);
        let line_height = zone.line_height;
        let bounds = zone.bounds.get()?;
        Some((
            bounds,
            bounds.size.height / line_height,
            zone.scroll.offset(),
        ))
    }

    /// The marker zone's message, "1 of 2" (1-based), whether it's stale,
    /// its message's accessible label and its body's focus handle, for
    /// tests.
    #[doc(hidden)]
    pub fn marker_zone_state_for_tests(&self, cx: &App) -> Option<MarkerZoneState> {
        let (zone, _) = self.marker_navigation.zone.as_ref()?;
        let zone = zone.read(cx);
        Some(MarkerZoneState {
            message: zone.marker.message.clone(),
            index: zone.index + 1,
            total: zone.total,
            stale: zone.stale,
            aria_label: zone.aria_label.clone(),
            focus_handle: zone.focus_handle.clone(),
        })
    }

    /// Whether the marker zone shows (Monaco `markersNavigationVisible`).
    pub(crate) fn is_marker_zone_visible(&self) -> bool {
        self.marker_navigation.zone.is_some()
    }

    /// Removes the zone, keeping the session; the editor gets the focus
    /// back from its body.
    fn remove_marker_zone(&mut self, cx: &mut Context<Self>) -> bool {
        self.marker_navigation.position = None;
        match self.marker_navigation.zone.take() {
            Some((zone, id)) => {
                if zone.read(cx).body_focused {
                    self.refocus = true;
                }
                self.remove_widget(id, cx);
                true
            }
            None => false,
        }
    }

    /// Closes the marker zone opened by [`CodeEditor::go_to_marker`]. Returns
    /// whether one was open.
    pub fn close_marker_zone(&mut self, cx: &mut Context<Self>) -> bool {
        self.marker_navigation.list = None;
        self.remove_marker_zone(cx)
    }
}

/// What [`CodeEditor::marker_zone_state_for_tests`] reports.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct MarkerZoneState {
    pub message: SharedString,
    pub index: usize,
    pub total: usize,
    pub stale: bool,
    pub aria_label: SharedString,
    pub focus_handle: gpui::FocusHandle,
}
