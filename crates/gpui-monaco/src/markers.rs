//! Diagnostics markers (Monaco `monaco.editor.setModelMarkers`): problems
//! pushed by the host, rendered as squiggles, overview ruler marks and hover
//! messages, and navigable with F8 / Shift+F8.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, div, px, rgb,
};
use text::PointUtf16;

use crate::decorations::{DecorationId, DecorationOptions, DecorationUnderline, OverviewRulerLane};
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
    /// `file.rs(3, 5): message`, as Monaco lists it (1-based).
    pub(crate) fn label(&self) -> String {
        let name = self
            .uri
            .as_deref()
            .map(|uri| uri.rsplit('/').next().unwrap_or(uri).to_string())
            .unwrap_or_default();
        format!(
            "{name}({}, {}): {}",
            self.range.start.row + 1,
            self.range.start.column + 1,
            self.message
        )
    }

    pub(crate) fn location(&self) -> crate::providers::Location {
        crate::providers::Location {
            uri: self.uri.clone(),
            range: self.range.clone(),
        }
    }
}

impl MarkerSeverity {
    /// Monaco's default `editorError/Warning/Info.foreground` colors.
    pub fn color(self) -> Hsla {
        match self {
            MarkerSeverity::Error => rgb(0xf14c4c).into(),
            MarkerSeverity::Warning => rgb(0xcca700).into(),
            MarkerSeverity::Info => rgb(0x3794ff).into(),
            MarkerSeverity::Hint => rgb(0xeeeeee).into(),
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
    /// Unnecessary or deprecated code.
    pub tags: Vec<MarkerTag>,
    /// Other places the diagnostic refers to.
    pub related_information: Vec<RelatedInformation>,
}

impl Marker {
    fn has_tag(&self, tag: MarkerTag) -> bool {
        self.tags.contains(&tag)
    }

    /// The source and code, e.g. `rustc(E0308)`.
    pub(crate) fn source_label(&self) -> Option<String> {
        match (&self.source, &self.code) {
            (Some(source), Some(code)) => Some(format!("{source}({code})")),
            (Some(source), None) => Some(source.to_string()),
            (None, Some(code)) => Some(format!("({code})")),
            (None, None) => None,
        }
    }

    pub(crate) fn hover_text(&self) -> String {
        let mut text = self.message.to_string();
        match (&self.source, &self.code) {
            (Some(source), Some(code)) => text.push_str(&format!(" `{source}({code})`")),
            (Some(source), None) => text.push_str(&format!(" `{source}`")),
            (None, Some(code)) => text.push_str(&format!(" `({code})`")),
            (None, None) => {}
        }
        text
    }
}

/// Markers of one model, by owner (e.g. one per linter).
#[derive(Default)]
pub(crate) struct MarkerSet {
    by_owner: HashMap<SharedString, (Vec<Marker>, Vec<DecorationId>)>,
}

impl TextModel {
    /// Replaces the markers of `owner` (Monaco `setModelMarkers`).
    pub fn set_markers(
        &mut self,
        owner: impl Into<SharedString>,
        markers: Vec<Marker>,
        cx: &mut Context<Self>,
    ) {
        let owner = owner.into();
        let old = self
            .markers
            .by_owner
            .remove(&owner)
            .map(|(_, ids)| ids)
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
                        overview_ruler: (marker.severity >= MarkerSeverity::Warning)
                            .then_some((color, OverviewRulerLane::Right)),
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        z_index: 10 + marker.severity as i32,
                        validation: true,
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
        if !markers.is_empty() {
            self.markers.by_owner.insert(owner, (markers, ids));
        }
        cx.emit(crate::TextModelEvent::MarkersChanged);
    }

    /// The markers whose (current) range contains byte `offset`, most severe
    /// first.
    pub fn markers_at(&self, offset: usize) -> Vec<Marker> {
        let buffer = self.buffer();
        let mut found: Vec<Marker> = self
            .markers
            .by_owner
            .values()
            .flat_map(|(markers, ids)| markers.iter().zip(ids))
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
            .values()
            .flat_map(|(markers, ids)| {
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
}

/// The inline problem view shown by F8 (Monaco's marker navigation widget,
/// shown as a view zone below the line).
pub struct MarkerZone {
    marker: Marker,
    index: usize,
    total: usize,
    colors: crate::theme::EditorColors,
    editor: gpui::WeakEntity<CodeEditor>,
}

impl Render for MarkerZone {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let color = self.marker.severity.color();
        let title = format!(
            "{} of {} problem{}",
            self.index + 1,
            self.total,
            if self.total == 1 { "" } else { "s" }
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .border_t_2()
            .border_b_2()
            .border_color(color)
            .bg(self.colors.widget_background)
            .text_color(self.colors.widget_foreground)
            .text_size(px(12.))
            .px(px(8.))
            .py(px(4.))
            .gap(px(2.))
            .child(div().text_color(color).child(title))
            .child(self.marker.hover_text())
            .children(
                self.marker
                    .related_information
                    .iter()
                    .enumerate()
                    .map(|(ix, related)| {
                        let editor = self.editor.clone();
                        let location = related.location();
                        div()
                            .id(("related", ix))
                            .cursor_pointer()
                            .text_color(self.colors.input_option_active_border)
                            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                if let Some(editor) = editor.upgrade() {
                                    editor.update(cx, |editor, cx| {
                                        editor.open_location(&location, true, cx);
                                    });
                                }
                            })
                            .child(related.label())
                    }),
            )
    }
}

#[derive(Default)]
pub(crate) struct MarkerNavigation {
    zone: Option<(Entity<MarkerZone>, WidgetId)>,
    index: Option<usize>,
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

    /// Moves to the next (or previous) marker and shows it below its line
    /// (Monaco `editor.action.marker.next`, F8).
    pub fn go_to_marker(&mut self, next: bool, cx: &mut Context<Self>) {
        let markers = self.model().read(cx).markers();
        if markers.is_empty() {
            self.close_marker_zone(cx);
            return;
        }
        let head = self.cursors.primary().position;
        let index = if next {
            markers
                .iter()
                .position(|m| m.range.start > head)
                .unwrap_or(0)
        } else {
            markers
                .iter()
                .rposition(|m| m.range.start < head)
                .unwrap_or(markers.len() - 1)
        };
        self.show_marker_at(markers, index, cx);
    }

    /// Selects `marker` and shows it in the marker zone (Monaco's "View
    /// Problem" hover action).
    pub fn show_marker(&mut self, marker: &Marker, cx: &mut Context<Self>) {
        let markers = self.model().read(cx).markers();
        let index = markers.iter().position(|m| m == marker).or_else(|| {
            markers
                .iter()
                .position(|m| m.range.start == marker.range.start && m.message == marker.message)
        });
        if let Some(index) = index {
            self.show_marker_at(markers, index, cx);
        }
    }

    fn show_marker_at(&mut self, markers: Vec<Marker>, index: usize, cx: &mut Context<Self>) {
        let marker = markers[index].clone();
        self.close_marker_zone(cx);
        self.set_selections(
            &[crate::cursor::Selection::collapsed(marker.range.start)],
            cx,
        );
        let lines = 1.
            + (marker.hover_text().len() as f32 / 80.).ceil()
            + marker.related_information.len() as f32;
        let editor = cx.entity().downgrade();
        let zone = cx.new(|_| MarkerZone {
            marker: marker.clone(),
            index,
            total: markers.len(),
            colors: self.theme.colors.clone(),
            editor,
        });
        let id = self.add_view_zone(
            ViewZone {
                after_line: marker.range.start.row + 1,
                height: ZoneHeight::Lines(lines + 0.5),
                view: zone.clone().into(),
            },
            cx,
        );
        self.marker_navigation.zone = Some((zone, id));
        self.marker_navigation.index = Some(index);
    }

    /// Closes the marker zone opened by [`CodeEditor::go_to_marker`]. Returns
    /// whether one was open.
    pub fn close_marker_zone(&mut self, cx: &mut Context<Self>) -> bool {
        self.marker_navigation.index = None;
        match self.marker_navigation.zone.take() {
            Some((_, id)) => {
                self.remove_widget(id, cx);
                true
            }
            None => false,
        }
    }
}
