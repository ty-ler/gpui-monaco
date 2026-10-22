//! Diagnostics markers (Monaco `monaco.editor.setModelMarkers`): problems
//! pushed by the host, rendered as squiggles, overview ruler marks and hover
//! messages, and navigable with F8 / Shift+F8.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, EntityId, Global, Hsla, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, PathBuilder, Pixels, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled, Window, canvas, div, point, px, rgb,
    rgba,
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
    /// A hint: three dots under its first two characters, or nothing when
    /// tagged.
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
                        // Monaco `open(code.target)`: no `allowCommands`.
                        editor.update(cx, |editor, cx| {
                            editor.open_link_with_openers(target, false, cx)
                        });
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
    /// The range (0-based rows, UTF-16 columns). As in Monaco, an empty
    /// range is drawn over the word there (one character wide at the end
    /// of a line), and a whole-line one, from column 0 to `u32::MAX`
    /// (Monaco's `Number.MAX_VALUE`), from the first non-whitespace
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

/// Monaco `MarkerDecorations.MAX_MARKER_DECORATIONS_LIMIT`: only the first
/// 5000 markers of a resource are decorated. A model's
/// [`MarkerLimit::is_markers_truncated`] says when it has more.
pub const MAX_DECORATED_MARKERS: usize = 5000;

/// How [`set_markers_for_uri_with_options`] and
/// [`TextModel::set_markers_with_options`] set markers (Monaco
/// `IMarkerChangeOptions`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkerChangeOptions {
    /// The owner (e.g. a language server) sent only part of its markers
    /// for the uri. A model with the uri reports it in
    /// [`MarkerLimit::is_markers_source_truncated`] until the owner's
    /// markers there are set again without it or removed.
    pub is_truncated: bool,
}

/// Monaco `IMarkerLimit`: whether a model shows all the markers of its uri
/// ([`marker_limits`], [`TextModel::marker_limit`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkerLimit {
    /// An owner's markers of the uri are truncated at their source: they
    /// were set with [`MarkerChangeOptions::is_truncated`].
    pub is_markers_source_truncated: bool,
    /// [`MAX_DECORATED_MARKERS`] when the uri has more markers than that,
    /// so only the first are decorated; `None` when all are.
    pub is_markers_truncated: Option<usize>,
}

/// Monaco `MarkerService`: every marker, by owner and resource uri, apart
/// from the models. Markers outlive their model (only an in-memory model's
/// go with it) and decorate a model created later with their uri.
///
/// Like Monaco's `DoubleResourceMap`, owners and resources keep the order
/// they were first set in; replacing markers keeps the place, and removing
/// them all gives it up.
#[derive(Default)]
struct MarkerService {
    /// Each owner's markers by resource.
    by_owner: Vec<(SharedString, Vec<ResourceMarkers>)>,
    /// Each resource's owners.
    by_resource: Vec<(SharedString, Vec<SharedString>)>,
}

/// A resource's markers of one owner (Monaco `IMarkers`).
struct ResourceMarkers {
    resource: SharedString,
    markers: Vec<Rc<Marker>>,
    /// The owner sent only part of them (Monaco `isTruncated`).
    is_truncated: bool,
}

/// An owner, a resource and its markers of the owner.
type MarkerEntry<'a> = (&'a SharedString, &'a SharedString, &'a Vec<Rc<Marker>>);

impl Global for MarkerService {}

impl MarkerService {
    fn set(
        &mut self,
        owner: SharedString,
        resource: SharedString,
        markers: Vec<Rc<Marker>>,
        is_truncated: bool,
    ) {
        let resources = match self.by_owner.iter().position(|(o, _)| *o == owner) {
            Some(ix) => &mut self.by_owner[ix].1,
            None => {
                self.by_owner.push((owner.clone(), Vec::new()));
                &mut self.by_owner.last_mut().unwrap().1
            }
        };
        match resources.iter_mut().find(|r| r.resource == resource) {
            Some(old) => {
                old.markers = markers;
                old.is_truncated = is_truncated;
            }
            None => resources.push(ResourceMarkers {
                resource: resource.clone(),
                markers,
                is_truncated,
            }),
        }
        let owners = match self.by_resource.iter().position(|(r, _)| *r == resource) {
            Some(ix) => &mut self.by_resource[ix].1,
            None => {
                self.by_resource.push((resource, Vec::new()));
                &mut self.by_resource.last_mut().unwrap().1
            }
        };
        if !owners.contains(&owner) {
            owners.push(owner);
        }
    }

    /// Removes `owner`'s markers of `resource`; whether there were any.
    fn delete(&mut self, owner: &str, resource: &str) -> bool {
        let Some(ix) = self.by_owner.iter().position(|(o, _)| o == owner) else {
            return false;
        };
        let resources = &mut self.by_owner[ix].1;
        let Some(rx) = resources.iter().position(|r| r.resource == resource) else {
            return false;
        };
        resources.remove(rx);
        if resources.is_empty() {
            self.by_owner.remove(ix);
        }
        if let Some(ix) = self.by_resource.iter().position(|(r, _)| r == resource) {
            let owners = &mut self.by_resource[ix].1;
            owners.retain(|o| o != owner);
            if owners.is_empty() {
                self.by_resource.remove(ix);
            }
        }
        true
    }

    /// The resources `owner` has markers for.
    fn resources_of(&self, owner: &str) -> Vec<SharedString> {
        self.by_owner
            .iter()
            .find(|(o, _)| o == owner)
            .map(|(_, resources)| resources.iter().map(|r| r.resource.clone()).collect())
            .unwrap_or_default()
    }

    /// Monaco `MarkerService.isSourceTruncated`: whether any owner's
    /// markers of `resource` are truncated.
    fn is_source_truncated(&self, resource: &str) -> bool {
        self.by_owner
            .iter()
            .flat_map(|(_, resources)| resources)
            .any(|r| r.resource == resource && r.is_truncated)
    }

    /// The owners with markers for `resource`.
    fn owners_of(&self, resource: &str) -> Vec<SharedString> {
        self.by_resource
            .iter()
            .find(|(r, _)| r == resource)
            .map(|(_, owners)| owners.clone())
            .unwrap_or_default()
    }

    /// Monaco `MarkerService.read`: by owner, then resource, in the order
    /// first set; a resource's by owner; an owner's by resource.
    fn read(&self, filter: &MarkerFilter) -> Vec<(&SharedString, &SharedString, &Rc<Marker>)> {
        let take = filter.take.filter(|&take| take > 0).unwrap_or(usize::MAX);
        let entries: Box<dyn Iterator<Item = MarkerEntry<'_>>> =
            match (&filter.owner, &filter.resource) {
                (_, Some(resource)) => Box::new(
                    self.by_resource
                        .iter()
                        .filter(move |(r, _)| r == resource)
                        .flat_map(|(_, owners)| owners)
                        .filter(|owner| filter.owner.as_ref().is_none_or(|o| o == *owner))
                        .filter_map(move |owner| {
                            let (owner, resources) =
                                self.by_owner.iter().find(|(o, _)| o == owner)?;
                            let r = resources.iter().find(|r| r.resource == *resource)?;
                            Some((owner, &r.resource, &r.markers))
                        }),
                ),
                (owner, None) => Box::new(
                    self.by_owner
                        .iter()
                        .filter(move |(o, _)| owner.as_ref().is_none_or(|owner| owner == o))
                        .flat_map(|(owner, resources)| {
                            resources
                                .iter()
                                .map(move |r| (owner, &r.resource, &r.markers))
                        }),
                ),
            };
        entries
            .flat_map(|(owner, resource, markers)| {
                markers.iter().map(move |marker| (owner, resource, marker))
            })
            .take(take)
            .collect()
    }
}

/// Monaco `MarkerService._toMarker`: drops markers without a message and
/// moves an end before the start to the start's line.
fn sanitize(markers: Vec<Marker>) -> Vec<Rc<Marker>> {
    markers
        .into_iter()
        .filter(|marker| !marker.message.is_empty())
        .map(|mut marker| {
            if marker.range.end.row < marker.range.start.row {
                marker.range.end.row = marker.range.start.row;
            }
            Rc::new(marker)
        })
        .collect()
}

/// Monaco `MarkerService.changeOne`: replaces `owner`'s markers of
/// `resource`, queueing an [`on_did_change_markers`](crate::on_did_change_markers)
/// event. Removing them clears their truncation too. Removing nothing
/// isn't a change; whether it changed.
fn change_one(
    owner: SharedString,
    resource: SharedString,
    markers: Vec<Marker>,
    options: MarkerChangeOptions,
    cx: &mut App,
) -> bool {
    let service = cx.default_global::<MarkerService>();
    if markers.is_empty() {
        if !service.delete(&owner, &resource) {
            return false;
        }
    } else {
        service.set(
            owner,
            resource.clone(),
            sanitize(markers),
            options.is_truncated,
        );
    }
    crate::registry::markers_changed(resource, cx);
    true
}

/// Redecorates the live models (but `except`) with one of `resources`
/// (Monaco `MarkerDecorationsService._handleMarkerChange`).
fn update_models(resources: &[SharedString], except: Option<EntityId>, cx: &mut App) {
    for model in crate::registry::models(cx) {
        if Some(model.entity_id()) != except && resources.contains(model.read(cx).resource()) {
            model.update(cx, |model, cx| model.update_marker_decorations(cx));
        }
    }
}

/// Monaco `MarkerDecorationsService._onModelRemoved`: the markers of an
/// `inmemory:`, `internal:` or `vscode:` model go with it; others stay
/// for the next model with the uri.
pub(crate) fn model_disposed(resource: &SharedString, cx: &mut App) {
    let transient = ["inmemory:", "internal:", "vscode:"]
        .iter()
        .any(|scheme| resource.starts_with(scheme));
    if !transient {
        return;
    }
    let Some(service) = cx.try_global::<MarkerService>() else {
        return;
    };
    for owner in service.owners_of(resource) {
        change_one(owner, resource.clone(), Vec::new(), Default::default(), cx);
    }
}

/// A decorated marker of a model (Monaco `MarkerDecorations._map`).
pub(crate) struct DecoratedMarker {
    /// The marker service's instance: unchanged markers keep their
    /// decorations when others change.
    marker: Rc<Marker>,
    id: DecorationId,
    /// For an empty marker, how far into its (widened) decoration it is,
    /// in bytes: F8 goes there.
    collapsed_at: Option<usize>,
}

impl TextModel {
    /// Replaces the markers of `owner` for this model's
    /// [`resource`](TextModel::resource) (Monaco `setModelMarkers`).
    /// Markers without a message are dropped. Emits
    /// [`TextModelEvent::MarkersChanged`](crate::TextModelEvent::MarkersChanged)
    /// and the global [`on_did_change_markers`](crate::on_did_change_markers)
    /// unless `owner` had no markers and gets none.
    ///
    /// As in Monaco, markers belong to the uri rather than the model: they
    /// outlive it (but for an `inmemory:` one) and decorate the next model
    /// with the uri. Only the first [`MAX_DECORATED_MARKERS`] of the uri
    /// are drawn ([`TextModel::marker_limit`] says when there are more);
    /// [`model_markers`] has them all.
    pub fn set_markers(
        &mut self,
        owner: impl Into<SharedString>,
        markers: Vec<Marker>,
        cx: &mut Context<Self>,
    ) {
        self.set_markers_with_options(owner, markers, MarkerChangeOptions::default(), cx);
    }

    /// [`TextModel::set_markers`] with Monaco's `IMarkerChangeOptions`,
    /// such as whether the owner sent only part of its markers (Monaco
    /// `IMarkerService.changeOne`'s `options`).
    pub fn set_markers_with_options(
        &mut self,
        owner: impl Into<SharedString>,
        markers: Vec<Marker>,
        options: MarkerChangeOptions,
        cx: &mut Context<Self>,
    ) {
        let resource = self.resource().clone();
        if !change_one(owner.into(), resource.clone(), markers, options, cx) {
            return;
        }
        crate::registry::track_model(&cx.entity(), cx);
        self.update_marker_decorations(cx);
        update_models(&[resource], Some(cx.entity_id()), cx);
    }

    /// Whether the model shows all the markers of its uri (Monaco
    /// `IMarkerDecorationsService.getMarkerLimits`); see [`marker_limits`]
    /// to ask by uri. [`on_did_change_marker_limit`](crate::on_did_change_marker_limit)
    /// reports changes.
    pub fn marker_limit(&self) -> MarkerLimit {
        self.marker_limit
    }

    /// Monaco `MarkerDecorationsService._updateDecorations`: decorates the
    /// first [`MAX_DECORATED_MARKERS`] markers of the model's resource,
    /// reading one more to tell whether there are more, and updates the
    /// model's [`MarkerLimit`].
    pub(crate) fn update_marker_decorations(&mut self, cx: &mut Context<Self>) {
        let service = cx.try_global::<MarkerService>();
        let mut markers: Vec<Rc<Marker>> = service
            .map(|service| {
                service
                    .read(&MarkerFilter {
                        resource: Some(self.resource().clone()),
                        take: Some(MAX_DECORATED_MARKERS + 1),
                        ..Default::default()
                    })
                    .into_iter()
                    .map(|(_, _, marker)| marker.clone())
                    .collect()
            })
            .unwrap_or_default();
        let is_marker_limit_exceeded = markers.len() > MAX_DECORATED_MARKERS;
        markers.truncate(MAX_DECORATED_MARKERS);
        let is_markers_source_truncated =
            service.is_some_and(|service| service.is_source_truncated(self.resource()));
        self.decorate_markers(markers, cx);
        self.update_marker_limit(is_markers_source_truncated, is_marker_limit_exceeded, cx);
    }

    /// Monaco `MarkerDecorations.updateMarkerLimit`: queues an
    /// [`on_did_change_marker_limit`](crate::on_did_change_marker_limit)
    /// event if either flag changed.
    fn update_marker_limit(
        &mut self,
        is_markers_source_truncated: bool,
        is_marker_limit_exceeded: bool,
        cx: &mut Context<Self>,
    ) {
        let limit = MarkerLimit {
            is_markers_source_truncated,
            is_markers_truncated: is_marker_limit_exceeded.then_some(MAX_DECORATED_MARKERS),
        };
        if limit == self.marker_limit {
            return;
        }
        let old = std::mem::replace(&mut self.marker_limit, limit);
        crate::registry::marker_limit_changed(cx.weak_entity(), old, cx);
    }

    /// Monaco `MarkerDecorations.update`: decorates `markers`, keeping the
    /// decorations of markers that didn't change, and emits
    /// [`TextModelEvent::MarkersChanged`](crate::TextModelEvent::MarkersChanged)
    /// if any did.
    fn decorate_markers(&mut self, markers: Vec<Rc<Marker>>, cx: &mut Context<Self>) {
        let key = |marker: &Rc<Marker>| Rc::as_ptr(marker) as usize;
        let new: std::collections::HashSet<usize> = markers.iter().map(key).collect();
        let old: std::collections::HashSet<usize> =
            self.markers.iter().map(|d| key(&d.marker)).collect();
        let added: Vec<Rc<Marker>> = markers
            .into_iter()
            .filter(|marker| !old.contains(&key(marker)))
            .collect();
        let removed: Vec<DecorationId> = self
            .markers
            .iter()
            .filter(|d| !new.contains(&key(&d.marker)))
            .map(|d| d.id)
            .collect();
        if added.is_empty() && removed.is_empty() {
            return;
        }
        let ranges: Vec<(Range<usize>, Option<usize>)> = added
            .iter()
            .map(|marker| self.marker_decoration_range(marker))
            .collect();
        let decorations = added
            .iter()
            .zip(&ranges)
            .map(|(marker, (range, _))| (range.clone(), marker_decoration_options(marker)))
            .collect();
        let ids = self.delta_decorations(None, &removed, decorations, cx);
        self.markers.retain(|d| new.contains(&key(&d.marker)));
        self.markers
            .extend(added.into_iter().zip(ids).zip(ranges).map(
                |((marker, id), (range, collapsed_at))| DecoratedMarker {
                    marker,
                    id,
                    collapsed_at: collapsed_at.map(|offset| offset.saturating_sub(range.start)),
                },
            ));
        cx.emit(crate::TextModelEvent::MarkersChanged);
    }

    /// Monaco `MarkerDecorations._createDecorationRange`, in bytes, with
    /// an empty marker's offset: an untagged hint covers two characters
    /// on its first line (room for its three dots); an empty marker
    /// widens to the word there, and stays empty (drawn one character
    /// wide) on an empty line or after the last non-whitespace
    /// character; a whole-line marker (column 0 to `u32::MAX`) starts at
    /// the first non-whitespace character.
    fn marker_decoration_range(&self, marker: &Marker) -> (Range<usize>, Option<usize>) {
        let raw = &marker.range;
        let mut range = raw.clone();
        if marker.severity == MarkerSeverity::Hint
            && !marker.has_tag(MarkerTag::Unnecessary)
            && !marker.has_tag(MarkerTag::Deprecated)
        {
            range.end = PointUtf16::new(range.start.row, range.start.column.saturating_add(2));
        }
        let mut range = self.validate_range(range);
        let mut collapsed = None;
        if range.is_empty() {
            let row = range.start.row;
            collapsed = Some(range.start);
            let max_column = self
                .line_last_non_whitespace_column(row)
                .unwrap_or_else(|| self.buffer().rope().line_len_utf16(row));
            if max_column > 0
                && range.end.column < max_column
                && let Some(word) = self.word_at_position(range.start)
            {
                range = word.range;
            }
        } else if raw.end.column == u32::MAX
            && raw.start.column == 0
            && range.start.row == range.end.row
        {
            let min_column = self
                .line_first_non_whitespace_column(raw.start.row)
                .unwrap_or(0);
            if min_column < range.end.column {
                range.start.column = min_column;
            }
        }
        let offset = |point| self.offset_at(point);
        (
            offset(range.start)..offset(range.end),
            collapsed.map(offset),
        )
    }

    /// Whether any marker is decorated here.
    pub(crate) fn has_markers(&self) -> bool {
        !self.markers.is_empty()
    }

    /// The decorated markers with their current ranges (they follow edits).
    fn live_markers(&self) -> impl Iterator<Item = (Marker, Option<usize>)> + '_ {
        let buffer = self.buffer();
        self.markers.iter().map(move |decorated| {
            let mut marker = (*decorated.marker).clone();
            let mut collapsed = None;
            if let Some(decoration) = self.decoration(decorated.id) {
                let range = &decoration.range;
                marker.range = buffer.offset_to_point_utf16(range.start)
                    ..buffer.offset_to_point_utf16(range.end);
                collapsed = decorated.collapsed_at.map(|at| {
                    buffer.clip_offset((range.start + at).min(range.end), text::Bias::Left)
                });
            }
            (marker, collapsed)
        })
    }

    /// The markers whose (current) range contains byte `offset`, most severe
    /// first.
    pub fn markers_at(&self, offset: usize) -> Vec<Marker> {
        let buffer = self.buffer();
        let mut found: Vec<Marker> = self
            .live_markers()
            .map(|(marker, _)| marker)
            .filter(|marker| {
                let start = buffer.point_utf16_to_offset(marker.range.start);
                let end = buffer.point_utf16_to_offset(marker.range.end);
                start <= offset && offset <= end
            })
            .collect();
        found.sort_by_key(|m| (std::cmp::Reverse(m.severity), m.range.start));
        found
    }

    /// Whether a decorated marker that `include` accepts intersects
    /// `range` (an empty one touching it doesn't), at its decoration's
    /// current place, without copying the markers as [`TextModel::markers`]
    /// does: the editor asks on every frame (`editorSelectionHasDiagnostics`).
    pub(crate) fn has_marker_intersecting(
        &self,
        range: Range<PointUtf16>,
        include: impl Fn(&Marker) -> bool,
    ) -> bool {
        let buffer = self.buffer();
        let start = buffer.point_utf16_to_offset(range.start);
        let end = buffer.point_utf16_to_offset(range.end);
        self.markers.iter().any(|decorated| {
            if !include(&decorated.marker) {
                return false;
            }
            let marker = match self.decoration(decorated.id) {
                Some(decoration) => decoration.range.clone(),
                None => {
                    let range = &decorated.marker.range;
                    buffer.point_utf16_to_offset(range.start)
                        ..buffer.point_utf16_to_offset(range.end)
                }
            };
            marker.start < end && start < marker.end
        })
    }

    /// The decorated markers (the first [`MAX_DECORATED_MARKERS`] of the
    /// model's uri; Monaco `getLiveMarkers`) with their current ranges
    /// (they move with edits, and empty ones are widened to the word),
    /// sorted by position.
    pub fn markers(&self) -> Vec<Marker> {
        let mut all: Vec<Marker> = self.live_markers().map(|(marker, _)| marker).collect();
        all.sort_by_key(|m| (m.range.start, std::cmp::Reverse(m.severity)));
        all
    }

    /// The markers F8 visits (Monaco `MarkerList`): errors, warnings and
    /// infos, not hints, most severe first and then by range (Monaco's
    /// `problems.sortOrder: 'severity'`, as standalone Monaco has no such
    /// setting). Ranges follow edits; an empty one stays empty, where it
    /// was set (not widened as its decoration is).
    pub(crate) fn navigable_markers(&self) -> Vec<Marker> {
        let buffer = self.buffer();
        let mut all: Vec<Marker> = self
            .live_markers()
            .filter(|(marker, _)| marker.severity >= MarkerSeverity::Info)
            .map(|(mut marker, collapsed)| {
                if let Some(at) = collapsed {
                    let at = buffer.offset_to_point_utf16(at);
                    marker.range = at..at;
                }
                marker
            })
            .collect();
        // Monaco `MarkerSeverity.compare`, then `compareRangesUsingStarts`.
        all.sort_by_key(|m| (std::cmp::Reverse(m.severity), m.range.start, m.range.end));
        all
    }
}

/// Monaco `MarkerDecorations._createDecorationOption`.
fn marker_decoration_options(marker: &Marker) -> DecorationOptions {
    let color = marker.severity.color();
    // Hints have no squiggle: an untagged one gets three dots, an
    // unnecessary one is underlined only by themes with an
    // `editorUnnecessaryCode.border` (see the element).
    let underline = (marker.severity != MarkerSeverity::Hint)
        .then_some(DecorationUnderline { color, wavy: true });
    DecorationOptions {
        underline,
        // Monaco marks infos too.
        overview_ruler: (marker.severity >= MarkerSeverity::Info)
            .then(|| OverviewRulerDecoration::new(color, OverviewRulerLane::Right)),
        // Monaco marks infos, warnings and errors in the minimap, in the
        // theme's colors for the severity.
        minimap: (marker.severity >= MarkerSeverity::Info).then(crate::MinimapDecoration::default),
        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
        // Monaco: an empty marker is drawn one character wide.
        show_if_collapsed: true,
        z_index: 10 + marker.severity as i32,
        validation: true,
        severity: Some(marker.severity),
        // Monaco: deprecated wins over unnecessary.
        unnecessary: marker.has_tag(MarkerTag::Unnecessary)
            && !marker.has_tag(MarkerTag::Deprecated),
        deprecated: marker.has_tag(MarkerTag::Deprecated),
        ..Default::default()
    }
}

/// Monaco `IMarker`: a marker with its owner and uri.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMarker {
    /// Who set it (Monaco `owner`).
    pub owner: SharedString,
    /// The uri of the model it was set on (Monaco `resource`): its
    /// [`TextModel::resource`] (the normalized uri), `inmemory://model/N`
    /// for a model without a uri.
    pub resource: SharedString,
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
    /// Only those of this uri (a model's [`TextModel::resource`]), compared
    /// in [`normalize_uri`](crate::normalize_uri)'s form.
    pub resource: Option<SharedString>,
    /// At most this many; `None` or `Some(0)` for all.
    pub take: Option<usize>,
}

/// The markers matching `filter` (Monaco `editor.getModelMarkers`), whether
/// or not a model with their uri is alive: by owner, then uri, in the
/// order they were first set; with a `resource`, by owner.
pub fn model_markers(cx: &App, filter: &MarkerFilter) -> Vec<ModelMarker> {
    let Some(service) = cx.try_global::<MarkerService>() else {
        return Vec::new();
    };
    let filter = MarkerFilter {
        resource: filter
            .resource
            .as_ref()
            .map(|resource| crate::normalize_uri(resource).into()),
        ..filter.clone()
    };
    service
        .read(&filter)
        .into_iter()
        .map(|(owner, resource, marker)| ModelMarker {
            owner: owner.clone(),
            resource: resource.clone(),
            marker: (**marker).clone(),
        })
        .collect()
}

/// Replaces the markers of `owner` for `uri` (Monaco `IMarkerService.changeOne`,
/// which `vscode-languageclient`'s diagnostic collections write to),
/// whether or not a model with the uri is open: [`model_markers`] lists
/// them, live models with the uri are redecorated, and a model with it
/// created later is decorated at once. `uri` is compared in
/// [`normalize_uri`](crate::normalize_uri)'s form. Markers without a
/// message are dropped; empty `markers` removes `owner`'s. See
/// [`TextModel::set_markers`] for a model's own.
pub fn set_markers_for_uri(
    cx: &mut App,
    owner: impl Into<SharedString>,
    uri: &str,
    markers: Vec<Marker>,
) {
    set_markers_for_uri_with_options(cx, owner, uri, markers, MarkerChangeOptions::default());
}

/// [`set_markers_for_uri`] with Monaco's `IMarkerChangeOptions`, such as
/// whether the owner sent only part of its markers for `uri` (Monaco
/// `IMarkerService.changeOne`'s `options`).
pub fn set_markers_for_uri_with_options(
    cx: &mut App,
    owner: impl Into<SharedString>,
    uri: &str,
    markers: Vec<Marker>,
    options: MarkerChangeOptions,
) {
    let resource: SharedString = crate::normalize_uri(uri).into();
    if change_one(owner.into(), resource.clone(), markers, options, cx) {
        update_models(&[resource], None, cx);
    }
}

/// Monaco `MarkerService.isSourceTruncated`: whether an owner's markers of
/// `uri` were set with [`MarkerChangeOptions::is_truncated`], whether or
/// not a model with the uri is open. `uri` is compared in
/// [`normalize_uri`](crate::normalize_uri)'s form.
pub fn is_source_truncated(cx: &App, uri: &str) -> bool {
    cx.try_global::<MarkerService>()
        .is_some_and(|service| service.is_source_truncated(&crate::normalize_uri(uri)))
}

/// The [`MarkerLimit`] of the live model with `uri` (Monaco
/// `IMarkerDecorationsService.getMarkerLimits`), `None` without one. `uri`
/// is compared in [`normalize_uri`](crate::normalize_uri)'s form.
pub fn marker_limits(cx: &App, uri: &str) -> Option<MarkerLimit> {
    let resource = crate::normalize_uri(uri);
    crate::registry::models(cx)
        .into_iter()
        .map(|model| model.read(cx))
        .find(|model| model.resource().as_ref() == resource)
        .map(TextModel::marker_limit)
}

/// Removes `owner`'s markers everywhere (Monaco `editor.removeAllMarkers`,
/// `IMarkerService.changeAll(owner, [])`), and so their truncation, with
/// one [`on_did_change_markers`](crate::on_did_change_markers) event for
/// all of them.
pub fn remove_all_markers(cx: &mut App, owner: &str) {
    let Some(service) = cx.try_global::<MarkerService>() else {
        return;
    };
    let resources = service.resources_of(owner);
    let owner = SharedString::from(owner.to_string());
    for resource in &resources {
        change_one(
            owner.clone(),
            resource.clone(),
            Vec::new(),
            Default::default(),
            cx,
        );
    }
    update_models(&resources, None, cx);
}

/// Monaco `MessageWidget.getHeightInLines`: the body shows at most this
/// many lines and scrolls the rest.
const MAX_BODY_LINES: usize = 17;

/// The marker zone's body, whose hover shows its scrollbars.
const MARKER_BODY_GROUP: &str = "marker-body";

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
    bounds: Rc<std::cell::Cell<Option<gpui::Bounds<Pixels>>>>,
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
        // Monaco's `.action-label`: 3px padding, 5px corners.
        div()
            .id(id)
            .debug_selector(move || id.into())
            .p(px(3.))
            .rounded(px(5.))
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
        // Monaco's `.marker-widget .descriptioncontainer .filename`:
        // `textLink.activeForeground`.
        let link = colors.text_link_active_foreground;
        let painted = self.bounds.clone();
        let extent = crate::scrollbars::extent(&self.scroll);
        let scroll = self.scroll.clone();
        let view = cx.entity().downgrade();

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
                                    .debug_selector(move || format!("marker-related-{ix}"))
                                    .cursor_pointer()
                                    .text_color(link)
                                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                        cx.stop_propagation();
                                        // Monaco `onDidSelectRelatedInformation`:
                                        // opens it, then closes the zone.
                                        if let Some(editor) = editor.upgrade() {
                                            editor.update(cx, |editor, cx| {
                                                editor.open_location(&location, true, cx);
                                                editor.close_marker_zone(cx);
                                            });
                                        }
                                    })
                                    .child(related.location_label(self.uri.as_deref())),
                            )
                            .child(related.message.clone())
                    },
                ))
        });

        // Monaco `PeekViewWidget`'s head: `ceil(lineHeight * 1.2)` tall, its
        // line height.
        let head_height = (line_height * 1.2).ceil();
        // `.peekview-title`: 13px, 20px in; the severity icon 4px before the
        // file name, and "1 of 2 problems" (`.dirname`) at `.9em` with a
        // `.5em` margin of its own size.
        let title = div()
            .id("marker-title")
            .debug_selector(|| "marker-title".into())
            .flex()
            .items_baseline()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .ml(px(20.))
            .text_size(px(13.))
            .child(
                div()
                    .debug_selector(|| "marker-severity-icon".into())
                    .flex_none()
                    .self_center()
                    .mr(px(4.))
                    .text_color(icon)
                    .child(codicon(Self::severity_icon(self.severity))),
            )
            .when(!title.is_empty(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_color(colors.peek_view_title_foreground)
                        .child(title),
                )
            })
            .child(
                div()
                    .debug_selector(|| "marker-detail".into())
                    .ml(px(13. * 0.9 * 0.5))
                    .text_size(px(13. * 0.9))
                    .flex_shrink_1()
                    .min_w_0()
                    .text_ellipsis()
                    .text_color(colors.peek_view_title_description_foreground)
                    .child(detail),
            );
        let header = div()
            .id("marker-head")
            .debug_selector(|| "marker-head".into())
            .h(head_height)
            .line_height(head_height)
            .overflow_hidden()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .bg(header_background)
            .child(title)
            // `.peekview-actions`: 2px from the right.
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .mr(px(2.))
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
                    .child(self.header_action(
                        "marker-close",
                        Codicon::Close,
                        "Close",
                        |editor, cx| {
                            editor.close_marker_zone(cx);
                        },
                    )),
            );

        // Monaco's `ScrollableElement` around the description: both axes,
        // since lines don't wrap.
        let body = div()
            .id("marker-body")
            .debug_selector(|| "marker-body".into())
            // Monaco `.marker-widget`: `tabIndex = 0`, `role="tooltip"`.
            .track_focus(&self.focus_handle)
            .role(gpui::Role::Tooltip)
            .size_full()
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
                    move |bounds, _, window, cx| {
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
                        crate::scrollbars::follow_extent(&scroll, extent, view, window, cx);
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
                    .child(
                        // Monaco's 6px scrollbars, and `.peekview-widget >
                        // .body { border-top: 1px }` in the frame color.
                        div()
                            .flex_1()
                            .min_h_0()
                            .border_t_1()
                            .border_color(frame)
                            .relative()
                            .group(MARKER_BODY_GROUP)
                            .child(body)
                            .children(crate::scrollbars::scrollbars(
                                "marker-body",
                                MARKER_BODY_GROUP.into(),
                                &self.scroll,
                                px(6.),
                                px(0.),
                                &colors,
                            )),
                    ),
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

/// A marker F8 visits, with its uri (Monaco `IMarker.resource`).
type ListedMarker = (SharedString, Marker);

/// Monaco `MarkerList`: the markers F8 visits and the one shown.
struct MarkerList {
    markers: Vec<ListedMarker>,
    /// Monaco `_nextIdx`, `None` for `-1`: the next move starts from the
    /// cursor.
    next: Option<usize>,
    /// Monaco's `undefined` resource filter: every uri's markers (F8,
    /// "in Files"), not only the model's (Alt+F8).
    in_files: bool,
}

impl MarkerList {
    fn selected(&self) -> Option<&ListedMarker> {
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

    /// Monaco `_initIdx`: from the model's first marker, the first one
    /// containing `position` (an empty one as its word) or after it. As
    /// in Monaco, the search goes on to the first marker of the next uri,
    /// compared with `position` too. Without markers in the model, the
    /// first marker of the uris after its own (backwards, the last one
    /// before).
    fn initial_index(&self, forward: bool, model: &TextModel, position: PointUtf16) -> usize {
        let len = self.markers.len();
        let last = len - 1;
        let resource = model.resource();
        let Some(first) = self.markers.iter().position(|(r, _)| r == resource) else {
            // Monaco `binarySearch2` for where the uri would sort. Monaco
            // goes nowhere when it sorts last; this wraps to the first.
            let ix = self
                .markers
                .partition_point(|(r, _)| r.as_ref() < resource.as_ref());
            return if forward {
                ix % len
            } else {
                (ix + len - 1) % len
            };
        };
        let mut found = None;
        for (ix, (r, marker)) in self.markers.iter().enumerate().skip(first) {
            let mut range = marker.range.clone();
            if r == resource
                && range.is_empty()
                && let Some(word) = model.word_at_position(range.start)
            {
                range = word.range;
            }
            let inside = contains(&range, position);
            if inside || position <= range.start {
                found = Some((ix, !inside));
                break;
            }
            if r != resource {
                break;
            }
        }
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
fn same_markers(a: &[ListedMarker], b: &[ListedMarker]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|((ra, a), (rb, b))| {
            ra == rb && a.range == b.range && a.severity == b.severity && a.message == b.message
        })
}

/// Monaco `MarkerList`'s `compareMarker`: by uri, then most severe first
/// (`problems.sortOrder: 'severity'`, as standalone Monaco has no such
/// setting), then by range.
fn compare_listed(a: &ListedMarker, b: &ListedMarker) -> std::cmp::Ordering {
    let ((ra, a), (rb, b)) = (a, b);
    ra.cmp(rb)
        .then_with(|| b.severity.cmp(&a.severity))
        .then_with(|| a.range.start.cmp(&b.range.start))
        .then_with(|| a.range.end.cmp(&b.range.end))
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

    /// Moves to the next (or previous) error, warning or info of the model
    /// and shows it below its line (Monaco `editor.action.marker.next`,
    /// Alt+F8). Markers go most severe first, then by position; the first
    /// move takes the one at the cursor.
    pub fn go_to_marker(&mut self, next: bool, cx: &mut Context<Self>) {
        self.navigate_markers(next, false, cx);
    }

    /// Moves to the next (or previous) error, warning or info of every uri
    /// (Monaco `editor.action.marker.nextInFiles`, F8): markers go by uri,
    /// then as [`CodeEditor::go_to_marker`]'s, and "2 of 5 problems"
    /// counts them all. A marker of another uri closes the zone and goes
    /// to the host's [editor openers](crate::register_editor_opener)
    /// (else [`CodeEditorEvent::OpenLocation`](crate::CodeEditorEvent::OpenLocation)),
    /// selecting its range; once an opener has set the editor's model to
    /// that uri, the editor shows the marker there.
    pub fn go_to_marker_in_files(&mut self, next: bool, cx: &mut Context<Self>) {
        self.navigate_markers(next, true, cx);
    }

    /// The markers a [`MarkerList`] visits (Monaco `MarkerList`'s
    /// `updateMarker`): the model's errors, warnings and infos with their
    /// current ranges and, `in_files`, other uris' as they were set.
    fn listed_markers(&self, in_files: bool, cx: &App) -> Vec<ListedMarker> {
        let model = self.model().read(cx);
        let resource = model.resource().clone();
        let mut markers: Vec<ListedMarker> = model
            .navigable_markers()
            .into_iter()
            .map(|marker| (resource.clone(), marker))
            .collect();
        if in_files && let Some(service) = cx.try_global::<MarkerService>() {
            markers.extend(
                service
                    .read(&MarkerFilter::default())
                    .into_iter()
                    .filter(|(_, r, marker)| {
                        **r != resource && marker.severity >= MarkerSeverity::Info
                    })
                    .map(|(_, r, marker)| (r.clone(), (**marker).clone())),
            );
            markers.sort_by(compare_listed);
        }
        markers
    }

    /// Monaco `MarkerController.navigate`.
    fn navigate_markers(&mut self, next: bool, in_files: bool, cx: &mut Context<Self>) {
        // Monaco `_getOrCreateModel`: a list of the other kind is replaced,
        // and the new one first moves from the cursor.
        let replaced = self
            .marker_navigation
            .list
            .as_ref()
            .is_some_and(|list| list.in_files != in_files);
        if replaced {
            self.close_marker_zone(cx);
        }
        let markers = self.listed_markers(in_files, cx);
        if markers.is_empty() {
            return;
        }
        let model = self.model().clone();
        let model = model.read(cx);
        let list = self.marker_navigation.list.get_or_insert(MarkerList {
            markers: Vec::new(),
            next: None,
            in_files,
        });
        // Ranges follow edits; other changes reset the list when they come.
        if list.markers.len() != markers.len() {
            list.next = None;
        }
        list.markers = markers;
        let head = self.cursors.primary().position;
        if replaced {
            list.step(true, model, head);
        }
        list.step(next, model, head);
        let Some(index) = list.next else {
            return;
        };
        let (resource, marker) = list.markers[index].clone();
        if &resource == model.resource() {
            self.show_marker_at(index, cx);
            return;
        }
        // Monaco: in another editor, opened at the marker, which then
        // closes its zone and navigates again.
        self.close_marker_zone(cx);
        let location = crate::providers::Location::new(Some(resource.clone()), marker.range);
        self.open_location_with_openers_then(
            location,
            false,
            move |editor, cx| {
                // Only once the opener showed the uri here: navigating
                // again from the old model would open it again.
                if *editor.model().read(cx).resource() == resource {
                    editor.close_marker_zone(cx);
                    editor.navigate_markers(next, in_files, cx);
                }
            },
            cx,
        );
    }

    /// Selects `marker` and shows it in the marker zone (Monaco's "View
    /// Problem" hover action). A hint shows the marker F8 would go to from
    /// its start.
    pub fn show_marker(&mut self, marker: &Marker, cx: &mut Context<Self>) {
        let markers = self.listed_markers(false, cx);
        if markers.is_empty() {
            return;
        }
        let model = self.model().clone();
        let model = model.read(cx);
        let same = |(_, m): &ListedMarker| {
            m.range.start == marker.range.start
                && m.severity == marker.severity
                && m.message == marker.message
        };
        let mut list = MarkerList {
            next: markers.iter().position(same),
            markers,
            in_files: false,
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
        let (_, marker) = list.markers[index].clone();
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
        let line_height = self.line_height();
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
        self.announce_status(label, cx);
        // And `showAtMarker` focuses the editor.
        self.refocus = true;
    }

    /// Monaco `MarkerController`'s cursor listener: moving off the shown
    /// marker makes the next move start from the cursor again.
    pub(crate) fn marker_navigation_cursor_moved(&mut self, position: PointUtf16) {
        if let Some(list) = &mut self.marker_navigation.list
            && !list
                .selected()
                .is_some_and(|(_, m)| contains(&m.range, position))
        {
            list.next = None;
        }
    }

    /// Monaco `MarkerController`'s marker listener: when the markers
    /// change, the zone shows the marker now at its position, or turns
    /// stale (faded and italic) when there's none.
    pub(crate) fn marker_navigation_markers_changed(&mut self, cx: &mut Context<Self>) {
        let Some(in_files) = self.marker_navigation.list.as_ref().map(|l| l.in_files) else {
            return;
        };
        let markers = self.listed_markers(in_files, cx);
        let model = self.model().clone();
        let model = model.read(cx);
        let navigation = &mut self.marker_navigation;
        let Some(list) = &mut navigation.list else {
            return;
        };
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
        // Monaco `MarkerList.find`: the model's marker there.
        let resource = model.resource();
        let found = list
            .markers
            .iter()
            .find(|(r, m)| r == resource && contains(&m.range, position))
            .map(|(_, marker)| marker)
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
            self.announce_status(label, cx);
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
