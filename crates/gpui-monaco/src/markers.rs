//! Diagnostics markers (Monaco `monaco.editor.setModelMarkers`): problems
//! pushed by the host, rendered as squiggles, overview ruler marks and hover
//! messages, and navigable with F8 / Shift+F8.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    AppContext as _, Context, Entity, Hsla, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, px, rgb,
};
use text::PointUtf16;

use crate::decorations::{DecorationId, DecorationOptions, DecorationUnderline, OverviewRulerLane};
use crate::editor::CodeEditor;
use crate::model::TextModel;
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};

/// Monaco `MarkerSeverity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MarkerSeverity {
    Hint,
    Info,
    Warning,
    Error,
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
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub severity: MarkerSeverity,
    pub message: SharedString,
    pub range: Range<PointUtf16>,
    /// E.g. `rustc`; shown after the message.
    pub source: Option<SharedString>,
    pub code: Option<SharedString>,
}

impl Marker {
    fn hover_text(&self) -> String {
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
                let mut end = buffer.point_utf16_to_offset(
                    buffer.clip_point_utf16(marker.range.end, text::Bias::Right),
                );
                // Monaco widens empty markers to the word or character there.
                if end <= start {
                    end = buffer
                        .clip_offset(start + 1, text::Bias::Right)
                        .min(buffer.len());
                }
                let color = marker.severity.color();
                (
                    start..end,
                    DecorationOptions {
                        underline: (marker.severity != MarkerSeverity::Hint)
                            .then_some(DecorationUnderline { color, wavy: true }),
                        overview_ruler: (marker.severity >= MarkerSeverity::Warning)
                            .then_some((color, OverviewRulerLane::Right)),
                        hover_message: Some(marker.hover_text().into()),
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        z_index: 10 + marker.severity as i32,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let ids = self.delta_decorations(None, &old, decorations, cx);
        if !markers.is_empty() {
            self.markers.by_owner.insert(owner, (markers, ids));
        }
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
    }
}

#[derive(Default)]
pub(crate) struct MarkerNavigation {
    zone: Option<(Entity<MarkerZone>, WidgetId)>,
    index: Option<usize>,
}

impl CodeEditor {
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
        let marker = markers[index].clone();
        self.close_marker_zone(cx);
        self.set_selections(
            &[crate::cursor::Selection::collapsed(marker.range.start)],
            cx,
        );
        let lines = 1. + (marker.hover_text().len() as f32 / 80.).ceil();
        let zone = cx.new(|_| MarkerZone {
            marker: marker.clone(),
            index,
            total: markers.len(),
            colors: self.theme.colors.clone(),
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
