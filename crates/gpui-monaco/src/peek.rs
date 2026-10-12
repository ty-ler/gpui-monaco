//! The peek view: navigation results in a view zone, with a preview editor
//! and a results list grouped by file.
//!
//! Ports Monaco's `src/vs/editor/contrib/gotoSymbol/browser/peek/`
//! (`referencesController.ts`, `referencesWidget.ts`, `referencesTree.ts`),
//! the peek view and zone widget frames (`contrib/peekView/`,
//! `contrib/zoneWidget/`) and the parts of the workbench list it uses
//! (keyboard navigation, type-to-navigate, the scrollbar) (MIT).

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, ClickEvent, Context, DragMoveEvent, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement,
    PathBuilder, Pixels, Render, ScrollWheelEvent, SharedString, StatefulInteractiveElement,
    Styled, Subscription, WeakEntity, Window, canvas, div, point, px,
};
use text::PointUtf16;

use crate::actions::*;
use crate::api::DecorationsCollection;
use crate::codicons::{Codicon, icon as codicon};
use crate::decorations::DecorationOptions;
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::model::{TextModel, model_for_uri};
use crate::mouse_target::MouseTargetKind;
use crate::options::LocationKind;
use crate::providers::Location;
use crate::theme::{EditorColors, Theme};
use crate::view::reveal::{RevealType, ScrollType};
use crate::widgets::{WidgetId, ZoneHeight};

/// The key context of the results list.
pub const PEEK_KEY_CONTEXT: &str = "PeekView";

/// Monaco `LayoutData`'s defaults: the preview's share of the width.
const PREVIEW_RATIO: f32 = 0.7;
/// Monaco `LayoutData.heightInLines`.
const HEIGHT_IN_LINES: f32 = 18.;
/// Monaco's split view minimums: the preview's and the list's width.
const MIN_PREVIEW_WIDTH: f32 = 200.;
const MIN_LIST_WIDTH: f32 = 100.;
/// Monaco `ZoneWidget._getResizeBounds`: the height sash keeps the zone
/// strictly between these many lines.
const MIN_LINES: f32 = 5.;
const MAX_LINES: f32 = 35.;
/// Monaco's references tree rows (`Delegate.getHeight`, `.ref-tree {
/// line-height: 23px }`).
const ROW_HEIGHT: Pixels = px(23.);
/// The workbench tree's indent per level and twistie (`.monaco-tl-twistie`:
/// 16px wide with 6px right padding).
const TREE_INDENT: Pixels = px(8.);
const TWISTIE_WIDTH: Pixels = px(22.);
/// Monaco's list scrollbar (`ScrollableElement`'s default
/// `verticalScrollbarSize`) and its minimum slider length.
const LIST_SCROLLBAR_WIDTH: Pixels = px(10.);
const MIN_SLIDER: Pixels = px(20.);
/// Monaco `TypeNavigationController`: typed characters add to the word
/// until a pause this long.
const TYPE_NAVIGATION_DELAY: Duration = Duration::from_millis(800);

/// Monaco's `peekViewLayout` storage: the last ratio and height, used by
/// the next peek view in any editor.
#[derive(Clone, Copy)]
struct PeekLayout {
    ratio: f32,
    height_in_lines: f32,
}

impl Default for PeekLayout {
    fn default() -> Self {
        Self {
            ratio: PREVIEW_RATIO,
            height_in_lines: HEIGHT_IN_LINES,
        }
    }
}

impl gpui::Global for PeekLayout {}

/// Something dragged in the peek view: the sash between the preview and
/// the list, the height sash along the bottom (Monaco's split view and
/// zone widget sashes), or the list's scrollbar slider.
#[derive(Clone, Copy)]
enum PeekDrag {
    Split,
    Height,
    Slider,
}

#[derive(Default)]
pub(crate) struct PeekState {
    zone: Option<WidgetId>,
    view: Option<Entity<PeekView>>,
}

/// The results in one document.
struct Group {
    uri: Option<SharedString>,
    model: Option<Entity<TextModel>>,
    /// Ranges as found; `highlights` tracks them through edits once the
    /// preview exists.
    ranges: Vec<Range<PointUtf16>>,
    highlights: Option<DecorationsCollection>,
    collapsed: bool,
}

impl Group {
    fn range(&self, ix: usize, cx: &gpui::App) -> Range<PointUtf16> {
        self.highlights
            .as_ref()
            .and_then(|highlights| highlights.range(ix, cx))
            .unwrap_or_else(|| self.ranges[ix].clone())
    }
}

/// One row of the results list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    File(usize),
    Reference(usize, usize),
}

/// The peek view's content (Monaco `ReferenceWidget`).
pub(crate) struct PeekView {
    editor: WeakEntity<CodeEditor>,
    /// The command that found the results, for the title; `None` for
    /// locations given to `goToLocations`.
    kind: Option<LocationKind>,
    anchor: PointUtf16,
    groups: Vec<Group>,
    /// The result shown in the preview: (group, reference).
    selected: (usize, usize),
    /// The list's focused (and selected) row: Monaco's tree has
    /// `selectionNavigation`, so they move together. It can be a file.
    focused: Row,
    preview: Option<Entity<CodeEditor>>,
    no_preview: Entity<TextModel>,
    preview_theme: Arc<Theme>,
    options: crate::options::EditorOptions,
    colors: EditorColors,
    high_contrast: bool,
    focus: FocusHandle,
    focus_pending: bool,
    /// Monaco `peekWidgetDefaultFocus: editor`: focus the preview instead
    /// of the list when it opens.
    focus_preview: bool,
    /// The preview's share of the width.
    ratio: f32,
    /// The zone's height in lines, which sizes the list.
    height_in_lines: f32,
    /// The list's scroll offset.
    scroll_top: Pixels,
    /// The mouse is over the list (Monaco shows its slider then).
    list_hovered: bool,
    /// Where a slider drag grabbed it, from the slider's top.
    slider_grab: Pixels,
    /// A height sash drag's start: the pointer's y and the height in lines
    /// (Monaco `ZoneWidget._initSash`'s `data`).
    sash_start: Option<(Pixels, f32)>,
    /// The sash showing `sash.hoverBorder` for the mouse resting on it.
    sash_hover: crate::sash::SashHover,
    /// Monaco `TypeNavigationController`: the word typed so far and when
    /// its last character came.
    typed: Option<(String, Instant)>,
    /// The zone's top in the window at the last paint.
    zone_top: Rc<Cell<Pixels>>,
    /// The rows rendered last (the list is virtualized).
    rendered: Range<usize>,
    _subscriptions: Vec<Subscription>,
}

/// Monaco `ReferencesModel.nearestReference`: the result in the most
/// similar uri, nearest to `position`.
fn nearest(locations: &[Location], uri: Option<&SharedString>, position: PointUtf16) -> usize {
    let prefix = |other: Option<&SharedString>| -> usize {
        match (other, uri) {
            (Some(a), Some(b)) => a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count(),
            (None, None) => usize::MAX,
            _ => 0,
        }
    };
    (0..locations.len())
        .min_by_key(|&ix| {
            let location = &locations[ix];
            // Monaco `OneReference.range`: a link's target selection range.
            let start = location.selection_range().start;
            let distance = start.row.abs_diff(position.row) as u64 * 100
                + start.column.abs_diff(position.column) as u64;
            (std::cmp::Reverse(prefix(location.uri.as_ref())), distance)
        })
        .unwrap_or(0)
}

/// The peek title and its description for a reference in `uri` (Monaco
/// `ReferenceWidget._revealReference`): its name and folder, but
/// "References" whatever the kind for an `inmemory:` model, which a model
/// without a URI is in Monaco.
fn peek_title(uri: Option<&str>) -> (String, String) {
    match uri {
        Some(uri) if !uri.starts_with("inmemory:") => uri_label(uri),
        _ => ("References".to_string(), String::new()),
    }
}

/// The last path segment of `uri` and the folder before it.
fn uri_label(uri: &str) -> (String, String) {
    let path = uri.split_once("://").map_or(uri, |(_, rest)| rest);
    let path = path.trim_end_matches('/');
    match path.rsplit_once('/') {
        Some((dir, name)) => (percent_decode(name), percent_decode(dir)),
        None => (percent_decode(path), String::new()),
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Monaco `FilePreview.preview`: a little text before the match (from a
/// word start, at most about 8 characters), the match, and the rest of the
/// line.
fn line_preview(line: &str, start: u32, end: Option<u32>) -> (String, String, String) {
    let byte = |column: u32| -> usize {
        let mut units = 0;
        for (ix, c) in line.char_indices() {
            if units >= column {
                return ix;
            }
            units += c.len_utf16() as u32;
        }
        line.len()
    };
    let start_byte = byte(start);
    let end_byte = end.map_or(line.len(), byte).max(start_byte);
    // Back up 8 UTF-16 units, then to the start of that word.
    let mut before_byte = byte(start.saturating_sub(8));
    while before_byte > 0 {
        let prev = line[..before_byte].chars().next_back().unwrap();
        if !(prev.is_alphanumeric() || prev == '_') {
            break;
        }
        before_byte -= prev.len_utf8();
    }
    (
        line[before_byte..start_byte].trim_start().to_string(),
        line[start_byte..end_byte].to_string(),
        line[end_byte..].trim_end().to_string(),
    )
}

/// Monaco's `ReferencesModel` titles (`generic.title` for `goToLocations`).
fn kind_title(kind: Option<LocationKind>) -> &'static str {
    let Some(kind) = kind else {
        return "Locations";
    };
    match kind {
        LocationKind::Definition => "Definitions",
        LocationKind::Declaration => "Declarations",
        LocationKind::TypeDefinition => "Type Definitions",
        LocationKind::Implementation => "Implementations",
        LocationKind::References => "References",
    }
}

/// The rows of the results list (Monaco `ReferenceWidget._onNewModel`):
/// with one file, its references are the tree's roots, without a file
/// row; otherwise each file, then its references unless collapsed.
fn list_rows(groups: &[(usize, bool)]) -> Vec<Row> {
    if let [(count, _)] = groups {
        return (0..*count).map(|r| Row::Reference(0, r)).collect();
    }
    let mut rows = Vec::new();
    for (g, (count, collapsed)) in groups.iter().enumerate() {
        rows.push(Row::File(g));
        if !collapsed {
            rows.extend((0..*count).map(|r| Row::Reference(g, r)));
        }
    }
    rows
}

/// Monaco `TypeNavigationController.onInput`: from the focused row (the
/// next one when a new word starts), the first whose label starts with
/// `word`, or contains it when it's longer than a character (`matchesPrefix`,
/// then a single `matchesFuzzy2` match), ignoring case.
fn type_navigation_target(
    labels: &[String],
    focused: usize,
    word: &str,
    new_word: bool,
) -> Option<usize> {
    let word = word.to_lowercase();
    let start = focused + usize::from(new_word);
    (0..labels.len())
        .map(|i| (start + i) % labels.len())
        .find(|&ix| {
            let label = labels[ix].to_lowercase();
            label.starts_with(&word) || (word.chars().count() > 1 && label.contains(&word))
        })
}

/// Monaco `List.focusNextPage`: the row to focus, and the list's new scroll
/// offset, for PageDown with `focused` among `count` rows of `row_height`
/// in a `height` tall list scrolled to `scroll_top`.
fn page_down(
    focused: usize,
    count: usize,
    scroll_top: Pixels,
    height: Pixels,
    row_height: Pixels,
) -> (usize, Pixels) {
    let max_scroll = (row_height * count as f32 - height).max(px(0.));
    let index_at = |y: Pixels| ((y / row_height).floor() as usize).min(count.saturating_sub(1));
    let mut scroll = scroll_top;
    loop {
        // The last row at least partly in view.
        let last = index_at(scroll + height)
            .saturating_sub(1)
            .max(index_at(scroll));
        if last > focused {
            return (last, scroll);
        }
        let mut next = scroll + height;
        if last > focused {
            next -= row_height;
        }
        let next = next.min(max_scroll);
        if next == scroll {
            return (last.max(focused), scroll);
        }
        scroll = next;
    }
}

/// Monaco `List.focusPreviousPage`, the reverse of [`page_down`].
fn page_up(
    focused: usize,
    count: usize,
    scroll_top: Pixels,
    height: Pixels,
    row_height: Pixels,
) -> (usize, Pixels) {
    let index_at = |y: Pixels| ((y / row_height).floor() as usize).min(count.saturating_sub(1));
    let mut scroll = scroll_top;
    loop {
        // The first row fully in view.
        let first = if scroll <= px(0.) {
            index_at(scroll)
        } else {
            (index_at(scroll - px(1.)) + 1).min(count.saturating_sub(1))
        };
        if focused > first {
            return (first, scroll);
        }
        let next = (scroll - height).max(px(0.));
        if next == scroll {
            return (first.min(focused), scroll);
        }
        scroll = next;
    }
}

impl PeekView {
    #[allow(clippy::too_many_arguments)]
    fn new(
        editor: &Entity<CodeEditor>,
        main_model: Entity<TextModel>,
        theme: Arc<Theme>,
        options: crate::options::EditorOptions,
        kind: Option<LocationKind>,
        anchor: PointUtf16,
        locations: Vec<Location>,
        selected: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        let main_uri = main_model.read(cx).uri().cloned();
        let mut groups: Vec<Group> = Vec::new();
        let mut selected_pos = (0, 0);
        for (ix, location) in locations.into_iter().enumerate() {
            if groups.last().is_none_or(|g| g.uri != location.uri) {
                let model = if location.uri.is_none() || location.uri == main_uri {
                    Some(main_model.clone())
                } else {
                    location
                        .uri
                        .as_deref()
                        .and_then(|uri| model_for_uri(cx, uri))
                };
                groups.push(Group {
                    uri: location.uri.clone(),
                    model,
                    ranges: Vec::new(),
                    highlights: None,
                    collapsed: false,
                });
            }
            let group = groups.len() - 1;
            if ix == selected {
                selected_pos = (group, groups[group].ranges.len());
            }
            // Monaco `OneReference.range`: a link's target selection range.
            groups[group].ranges.push(location.selection_range());
        }
        let mut preview_theme = (*theme).clone();
        preview_theme.colors.background = theme.colors.peek_view_editor_background;
        // Monaco `peekViewEditorGutter.background` and the sticky scroll's
        // `peekViewEditorStickyScroll*.background`.
        preview_theme.colors.gutter_background = theme.colors.peek_view_editor_gutter_background;
        preview_theme.colors.sticky_scroll_background =
            Some(theme.colors.peek_view_editor_sticky_scroll_background);
        preview_theme.colors.sticky_scroll_gutter_background = Some(
            theme
                .colors
                .peek_view_editor_sticky_scroll_gutter_background,
        );
        // Monaco `ReferenceWidget._fillBody`'s preview options, over the
        // outer editor's (an `EmbeddedCodeEditorWidget`).
        let mut preview_options = options.clone();
        preview_options.scroll_beyond_last_line = false;
        preview_options.vertical_scrollbar_size = px(14.);
        preview_options.scrollbar.horizontal = crate::options::ScrollbarVisibility::Auto;
        preview_options.scrollbar.use_shadows = true;
        preview_options.scrollbar.vertical_has_arrows = false;
        preview_options.scrollbar.horizontal_has_arrows = false;
        preview_options.scrollbar.always_consume_mouse_wheel = true;
        preview_options.overview_ruler_lanes = 2;
        preview_options.minimap.enabled = false;
        let subscriptions = vec![cx.observe(editor, |_, _, cx| cx.notify())];
        cx.on_release(|this: &mut Self, cx| {
            for group in &mut this.groups {
                if let Some(mut highlights) = group.highlights.take() {
                    highlights.clear(cx);
                }
            }
        })
        .detach();
        Self {
            editor: editor.downgrade(),
            kind,
            anchor,
            groups,
            selected: selected_pos,
            focused: Row::Reference(selected_pos.0, selected_pos.1),
            preview: None,
            no_preview: cx.new(|_| TextModel::new_internal("no preview available")),
            colors: theme.colors.clone(),
            high_contrast: theme.high_contrast,
            preview_theme: Arc::new(preview_theme),
            options: preview_options,
            focus: cx.focus_handle(),
            focus_pending: true,
            focus_preview: false,
            ratio: cx
                .try_global::<PeekLayout>()
                .map_or(PREVIEW_RATIO, |layout| layout.ratio),
            height_in_lines: HEIGHT_IN_LINES,
            scroll_top: px(0.),
            list_hovered: false,
            slider_grab: px(0.),
            sash_start: None,
            sash_hover: Default::default(),
            typed: None,
            zone_top: Default::default(),
            rendered: 0..0,
            _subscriptions: subscriptions,
        }
    }

    fn rows(&self) -> Vec<Row> {
        let groups: Vec<_> = self
            .groups
            .iter()
            .map(|g| (g.ranges.len(), g.collapsed))
            .collect();
        list_rows(&groups)
    }

    fn references(&self) -> Vec<(usize, usize)> {
        self.groups
            .iter()
            .enumerate()
            .flat_map(|(g, group)| (0..group.ranges.len()).map(move |r| (g, r)))
            .collect()
    }

    pub(crate) fn reference_count(&self) -> usize {
        self.groups.iter().map(|g| g.ranges.len()).sum()
    }

    /// The selected result.
    pub(crate) fn selected_location(&self, cx: &gpui::App) -> Location {
        let (g, r) = self.selected;
        Location {
            uri: self.groups[g].uri.clone(),
            range: self.groups[g].range(r, cx),
            ..Default::default()
        }
    }

    /// The zone widget's parts, from the zone's height (Monaco
    /// `ZoneWidget._showImpl` and `PeekViewWidget._doLayout`): the arrow's
    /// height (a third of a line, above and below the frame), the header's
    /// (1.2 lines) and the body's.
    fn layout_heights(&self) -> (Pixels, Pixels, Pixels) {
        let line_height = self.options.resolved_line_height();
        let arrow = (line_height / 3.).round();
        let container = line_height * self.height_in_lines - arrow * 2.;
        let head = (line_height * 1.2).ceil();
        // Less the body's 1px top border.
        let body = (container - head - px(1.)).round().max(px(0.));
        (arrow, head, body)
    }

    /// The list's height: the body's.
    fn list_height(&self) -> Pixels {
        self.layout_heights().2
    }

    fn max_scroll(&self, rows: usize) -> Pixels {
        (ROW_HEIGHT * rows as f32 - self.list_height()).max(px(0.))
    }

    fn set_scroll_top(&mut self, top: Pixels, rows: usize, cx: &mut Context<Self>) {
        let top = top.clamp(px(0.), self.max_scroll(rows));
        if top != self.scroll_top {
            self.scroll_top = top;
            cx.notify();
        }
    }

    /// Scrolls the list so row `ix` shows (Monaco `list.reveal`, minimal).
    fn reveal_row(&mut self, ix: usize, rows: usize, cx: &mut Context<Self>) {
        let top = ROW_HEIGHT * ix as f32;
        let height = self.list_height();
        let scroll = if top < self.scroll_top {
            top
        } else if top + ROW_HEIGHT > self.scroll_top + height {
            top + ROW_HEIGHT - height
        } else {
            return;
        };
        self.set_scroll_top(scroll, rows, cx);
    }

    /// Creates the preview editor (it needs a window) and its highlights.
    fn ensure_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview.is_some() {
            return;
        }
        let model = self.groups[self.selected.0]
            .model
            .clone()
            .unwrap_or_else(|| self.no_preview.clone());
        let theme = self.preview_theme.clone();
        let options = self.options.clone();
        let preview = cx.new(|cx| {
            let mut editor = CodeEditor::new(model, window, cx);
            editor.in_peek = true;
            editor.update_options(options, cx);
            editor.set_theme(theme, cx);
            editor
        });
        let owner = preview.entity_id();
        let highlight = DecorationOptions {
            background: Some(self.colors.peek_view_editor_match_highlight_background),
            border: self.colors.peek_view_editor_match_highlight_border,
            ..Default::default()
        };
        for group in &mut self.groups {
            if let Some(model) = &group.model {
                let mut highlights = DecorationsCollection::for_owner(model.clone(), owner);
                highlights.set(
                    group
                        .ranges
                        .iter()
                        .map(|range| (range.clone(), highlight.clone()))
                        .collect(),
                    cx,
                );
                group.highlights = Some(highlights);
            }
        }
        // A double click in the preview opens the result (Monaco), unless
        // `stablePeek`; with Ctrl, Cmd or Alt, to the side.
        self._subscriptions
            .push(cx.subscribe(&preview, |this, _, event, cx| {
                if let CodeEditorEvent::MouseDown(mouse) = event
                    && mouse.click_count == 2
                    && mouse.target.kind == MouseTargetKind::ContentText
                    && !this.stable_peek(cx)
                {
                    let side =
                        mouse.modifiers.control || mouse.modifiers.platform || mouse.modifiers.alt;
                    this.open_selected(true, side, cx);
                }
            }));
        self.preview = Some(preview);
        self.reveal_selected(cx);
    }

    /// Shows the selected result in the preview (Monaco `_revealReference`):
    /// smoothly within the same model, at once in another.
    fn reveal_selected(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.preview.clone() else {
            return;
        };
        let (g, r) = self.selected;
        let model = self.groups[g]
            .model
            .clone()
            .unwrap_or_else(|| self.no_preview.clone());
        let range = if self.groups[g].model.is_some() {
            Some(self.groups[g].range(r, cx))
        } else {
            None
        };
        preview.update(cx, |preview, cx| {
            let scroll_type = if preview.model() == &model {
                ScrollType::Smooth
            } else {
                preview.set_model(model, cx);
                ScrollType::Immediate
            };
            if let Some(range) = range {
                let start = range.start;
                preview.set_selections(&[crate::cursor::Selection::collapsed(start)], cx);
                preview.reveal_range_with_scroll_type(
                    start..start,
                    RevealType::Center,
                    scroll_type,
                    cx,
                );
            }
        });
        cx.notify();
    }

    /// Focuses (and selects) `row`; a reference also shows in the preview
    /// (Monaco's tree with `selectionNavigation`, whose open event is
    /// `show`).
    fn focus_row(&mut self, row: Row, cx: &mut Context<Self>) {
        self.focused = row;
        if let Row::Reference(g, r) = row {
            self.groups[g].collapsed = false;
            self.selected = (g, r);
            self.reveal_selected(cx);
        }
        let rows = self.rows();
        if let Some(ix) = rows.iter().position(|r| *r == row) {
            self.reveal_row(ix, rows.len(), cx);
        }
        cx.notify();
    }

    fn select(&mut self, selected: (usize, usize), cx: &mut Context<Self>) {
        self.focus_row(Row::Reference(selected.0, selected.1), cx);
    }

    /// Selects the next (or previous) result, wrapping (Monaco
    /// `goToNextReference`, F4, and `goToPreviousReference`, Shift+F4).
    pub(crate) fn select_relative(&mut self, delta: isize, cx: &mut Context<Self>) {
        let references = self.references();
        if references.is_empty() {
            return;
        }
        let ix = references
            .iter()
            .position(|r| *r == self.selected)
            .unwrap_or(0) as isize;
        let next = (ix + delta).rem_euclid(references.len() as isize) as usize;
        self.select(references[next], cx);
    }

    /// The focused row's index among `rows`.
    fn focused_index(&self, rows: &[Row]) -> usize {
        rows.iter()
            .position(|row| *row == self.focused)
            .unwrap_or(0)
    }

    /// Up and Down (Monaco `list.focusUp`, `list.focusDown`): the previous
    /// or next row, files included.
    fn move_in_list(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let ix = self.focused_index(&rows);
        let next = if forward {
            (ix + 1).min(rows.len() - 1)
        } else {
            ix.saturating_sub(1)
        };
        self.focus_row(rows[next], cx);
    }

    /// Home and End (Monaco `list.focusFirst`, `list.focusLast`).
    fn focus_edge(&mut self, last: bool, cx: &mut Context<Self>) {
        let rows = self.rows();
        let row = if last { rows.last() } else { rows.first() };
        if let Some(row) = row.copied() {
            self.focus_row(row, cx);
        }
    }

    /// PageDown and PageUp (Monaco `list.focusPageDown`,
    /// `list.focusPageUp`).
    fn move_by_page(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let focused = self.focused_index(&rows);
        let height = self.list_height();
        let (ix, scroll) = if forward {
            page_down(focused, rows.len(), self.scroll_top, height, ROW_HEIGHT)
        } else {
            page_up(focused, rows.len(), self.scroll_top, height, ROW_HEIGHT)
        };
        self.set_scroll_top(scroll, rows.len(), cx);
        self.focus_row(rows[ix], cx);
    }

    /// Left (Monaco `list.collapse`): collapses a file, or from a reference
    /// focuses its file.
    fn collapse(&mut self, cx: &mut Context<Self>) {
        match self.focused {
            Row::File(g) if !self.groups[g].collapsed => self.toggle_group(g, cx),
            Row::Reference(g, _) if self.groups.len() > 1 => self.focus_row(Row::File(g), cx),
            _ => {}
        }
    }

    /// Right (Monaco `list.expand`): expands a file, or focuses its first
    /// reference when it's expanded.
    fn expand(&mut self, cx: &mut Context<Self>) {
        if let Row::File(g) = self.focused {
            if self.groups[g].collapsed {
                self.toggle_group(g, cx);
            } else if !self.groups[g].ranges.is_empty() {
                self.focus_row(Row::Reference(g, 0), cx);
            }
        }
    }

    /// Enter: goes to the focused result (Monaco `revealReference`), or
    /// toggles the focused file (`list.select` on a collapsible row).
    fn open_focused(&mut self, cx: &mut Context<Self>) {
        match self.focused {
            Row::File(g) => self.toggle_group(g, cx),
            Row::Reference(..) => self.open_selected(false, false, cx),
        }
    }

    /// A typed character (Monaco `TypeNavigationController`): focuses the
    /// next row whose label matches the word typed so far.
    fn type_to_navigate(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return;
        }
        let Some(text) = keystroke.key_char.as_deref() else {
            return;
        };
        if text.chars().any(|c| c.is_control()) || text.trim().is_empty() && self.typed.is_none() {
            return;
        }
        cx.stop_propagation();
        let now = Instant::now();
        let (mut word, new_word) = match self.typed.take() {
            Some((word, at)) if now.duration_since(at) < TYPE_NAVIGATION_DELAY => (word, false),
            _ => (String::new(), true),
        };
        word.push_str(text);
        let rows = self.rows();
        let labels: Vec<String> = rows.iter().map(|row| self.row_label(*row, cx)).collect();
        let focused = self.focused_index(&rows);
        if let Some(ix) = type_navigation_target(&labels, focused, &word, new_word) {
            self.focus_row(rows[ix], cx);
        }
        self.typed = Some((word, now));
    }

    /// Monaco `StringRepresentationProvider`: a reference's preview text, a
    /// file's name.
    fn row_label(&self, row: Row, cx: &gpui::App) -> String {
        match row {
            Row::File(g) => self.file_label(g).0,
            Row::Reference(g, r) => {
                let (before, inside, after) = self.reference_preview(g, r, cx);
                format!("{before}{inside}{after}")
            }
        }
    }

    fn file_label(&self, g: usize) -> (String, String) {
        self.groups[g]
            .uri
            .as_deref()
            .map(uri_label)
            .unwrap_or_else(|| ("(this document)".into(), String::new()))
    }

    fn reference_preview(&self, g: usize, r: usize, cx: &gpui::App) -> (String, String, String) {
        let group = &self.groups[g];
        let range = group.range(r, cx);
        match &group.model {
            Some(model) => {
                let line = model.read(cx).line_content(range.start.row);
                let end = (range.end.row == range.start.row).then_some(range.end.column);
                line_preview(&line, range.start.column, end)
            }
            None => (
                format!("Ln {}, Col {}", range.start.row + 1, range.start.column + 1),
                String::new(),
                String::new(),
            ),
        }
    }

    /// Enter or a double click: go to the selected result (Monaco
    /// `_gotoReference`). In this editor's model the peek view moves to it;
    /// elsewhere it closes and the host is asked to open it. A double click
    /// in the preview and a click on the title (`close`) always close it
    /// (Monaco `openReference`); with `side`, the host opens it to the side.
    fn open_selected(&mut self, close: bool, side: bool, cx: &mut Context<Self>) {
        let location = self.selected_location(cx);
        let references = self.references();
        let selected = references
            .iter()
            .position(|r| *r == self.selected)
            .unwrap_or(0);
        let locations = references
            .into_iter()
            .map(|(g, r)| Location {
                uri: self.groups[g].uri.clone(),
                range: self.groups[g].range(r, cx),
                ..Default::default()
            })
            .collect();
        let results = (self.kind, locations, selected);
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.peek_open_location(location, close, side, results, cx)
        });
    }

    /// Monaco `stablePeek`: Escape and double clicks in the preview leave
    /// the peek view open.
    fn stable_peek(&self, cx: &gpui::App) -> bool {
        self.editor
            .upgrade()
            .is_some_and(|editor| editor.read(cx).options.stable_peek)
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| {
                editor.close_peek(cx);
            });
        }
    }

    fn toggle_group(&mut self, group: usize, cx: &mut Context<Self>) {
        self.groups[group].collapsed = !self.groups[group].collapsed;
        let rows = self.rows();
        self.set_scroll_top(self.scroll_top, rows.len(), cx);
        cx.notify();
    }

    fn title(&self) -> (String, String) {
        peek_title(self.groups[self.selected.0].uri.as_deref())
    }

    /// The list's wheel: always consumed (Monaco's list has
    /// `alwaysConsumeMouseWheel`), so the editor behind doesn't scroll.
    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let delta = event.delta.pixel_delta(ROW_HEIGHT).y;
        let rows = self.rows().len();
        self.set_scroll_top(self.scroll_top - delta, rows, cx);
    }

    /// The list's slider: (top, height) in a track `height` tall, or `None`
    /// when every row fits.
    fn slider(&self, rows: usize) -> Option<(Pixels, Pixels)> {
        let height = self.list_height();
        let total = ROW_HEIGHT * rows as f32;
        if total <= height || height <= px(0.) {
            return None;
        }
        let size = (height * (height / total)).max(MIN_SLIDER).min(height);
        let top = (height - size) * (self.scroll_top / (total - height));
        Some((top, size))
    }

    /// Scrolls so the slider's top is at `slider_top` in the track.
    fn drag_slider(&mut self, slider_top: Pixels, cx: &mut Context<Self>) {
        let rows = self.rows().len();
        let Some((_, size)) = self.slider(rows) else {
            return;
        };
        let height = self.list_height();
        let room = (height - size).max(px(1.));
        let fraction = (slider_top / room).clamp(0., 1.);
        self.set_scroll_top(self.max_scroll(rows) * fraction, rows, cx);
    }

    fn render_row(
        &self,
        ix: usize,
        row: Row,
        list_focused: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = &self.colors;
        let focused = self.focused == row;
        // Monaco: the peek colors while the list has focus, else the list's
        // inactive selection.
        let (selection_background, selection_foreground) = if list_focused {
            (
                Some(colors.peek_view_result_selection_background),
                Some(colors.peek_view_result_selection_foreground),
            )
        } else {
            (
                colors.list_inactive_selection_background,
                colors.list_inactive_selection_foreground,
            )
        };
        let single_file = self.groups.len() == 1;
        let base = div()
            .h(ROW_HEIGHT)
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .when(focused, |row| {
                row.when_some(selection_background, |row, bg| row.bg(bg))
                    // `list.focusOutline`, inside the row.
                    .when_some(
                        colors.list_focus_outline.filter(|_| list_focused),
                        |row, c| row.border_1().border_color(c),
                    )
            });
        match row {
            Row::File(g) => {
                let group = &self.groups[g];
                let (name, dir) = self.file_label(g);
                base.id(("peek-file", ix))
                    .pl(px(0.))
                    .text_color(
                        selection_foreground
                            .filter(|_| focused)
                            .unwrap_or(colors.peek_view_result_file_foreground),
                    )
                    .when(self.high_contrast, |row| {
                        row.font_weight(gpui::FontWeight::BOLD)
                    })
                    // The tree's twistie: `tree-item-expanded`
                    // (`chevron-down`), turned to `chevron-right` when
                    // collapsed.
                    .child(
                        div()
                            .w(TWISTIE_WIDTH)
                            .flex_none()
                            .flex()
                            .justify_end()
                            .pr(px(6.))
                            .child(codicon(if group.collapsed {
                                Codicon::ChevronRight
                            } else {
                                Codicon::TreeItemExpanded
                            })),
                    )
                    // `IconLabel`: the name, then the folder as its
                    // description (`.9em`, `.7` opacity, `.5em` apart).
                    .child(div().flex_none().child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ml(px(6.5))
                            .text_size(px(11.7))
                            .opacity(0.7)
                            .text_ellipsis()
                            .child(dir),
                    )
                    // `CountBadge` (`.count { margin-right: 12px }`).
                    .child(
                        div()
                            .flex_none()
                            .mr(px(12.))
                            .ml(px(4.))
                            .min_w(px(18.))
                            .h(px(18.))
                            .px(px(6.))
                            .rounded(px(11.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.))
                            .bg(colors.badge_background)
                            .text_color(colors.badge_foreground)
                            .when_some(colors.contrast_border, |badge, border| {
                                badge.border_1().border_color(border)
                            })
                            .child(group.ranges.len().to_string()),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.focused = Row::File(g);
                        this.toggle_group(g, cx);
                    }))
                    .into_any_element()
            }
            Row::Reference(g, r) => {
                let (before, inside, after) = self.reference_preview(g, r, cx);
                let foreground = selection_foreground
                    .filter(|_| focused)
                    .unwrap_or(colors.peek_view_result_line_foreground);
                // Roots without a file row; otherwise one level in.
                let indent = if single_file {
                    TWISTIE_WIDTH
                } else {
                    TREE_INDENT + TWISTIE_WIDTH
                };
                base.id(("peek-reference", ix))
                    .pl(indent)
                    .pr(px(6.))
                    .text_color(foreground)
                    .child(before)
                    .child(
                        // `.referenceMatch .highlight`: the file color on the
                        // match background; dotted in high contrast.
                        div()
                            .text_color(colors.peek_view_result_file_foreground)
                            .bg(colors.peek_view_result_match_highlight_background)
                            .when_some(
                                colors.contrast_active_border.filter(|_| self.high_contrast),
                                |m, border| m.border_1().border_dashed().border_color(border),
                            )
                            .child(inside),
                    )
                    .child(div().min_w_0().text_ellipsis().child(after))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.select((g, r), cx);
                        if event.click_count() == 2 {
                            this.open_selected(false, false, cx);
                        }
                    }))
                    .into_any_element()
            }
        }
    }
}

impl Focusable for PeekView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PeekView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_preview(window, cx);
        if std::mem::take(&mut self.focus_pending) {
            match self.preview.as_ref().filter(|_| self.focus_preview) {
                Some(preview) => window.focus(&preview.read(cx).focus_handle.clone(), cx),
                None => window.focus(&self.focus, cx),
            }
        }
        let colors = self.colors.clone();
        let (arrow_height, head_height, body_height) = self.layout_heights();
        // Monaco `Arrow.show`: under the anchor, pushed to the second
        // column from the first.
        let arrow_x = self
            .editor
            .upgrade()
            .and_then(|editor| {
                let editor = editor.read(cx);
                let left = editor.layout.as_ref()?.bounds.left();
                let column = if self.anchor.column == 0 {
                    editor
                        .model()
                        .read(cx)
                        .line_len_utf16(self.anchor.row)
                        .min(1)
                } else {
                    self.anchor.column
                };
                let position =
                    editor.scrolled_visible_position(PointUtf16::new(self.anchor.row, column))?;
                Some(position.x - left)
            })
            .unwrap_or(px(30.));
        let border = colors.peek_view_border;
        let zone_top = self.zone_top.clone();
        let (title, description) = self.title();
        let meta = format!("{} ({})", kind_title(self.kind), self.reference_count());
        // The visible rows only (Monaco's list is virtualized).
        let rows = self.rows();
        let row_count = rows.len();
        self.scroll_top = self.scroll_top.clamp(px(0.), self.max_scroll(row_count));
        let first = ((self.scroll_top / ROW_HEIGHT).floor() as usize).min(row_count);
        let last = (((self.scroll_top + body_height) / ROW_HEIGHT).ceil() as usize).min(row_count);
        let list_focused = self.focus.is_focused(window);
        self.rendered = first..last;
        let visible_rows: Vec<_> = (first..last)
            .map(|ix| self.render_row(ix, rows[ix], list_focused, cx))
            .collect();
        let rows_top = ROW_HEIGHT * first as f32 - self.scroll_top;
        let slider = self.slider(row_count);
        let show_slider = self.list_hovered || cx.has_active_drag();
        let scrolled = self.scroll_top > px(0.);
        let preview = self.preview.clone();
        // Monaco `Sash`: `sash.hoverBorder` once the mouse has rested on
        // the sash for its hover delay.
        let sash_hover = colors.sash_hover_border;
        let shown = [
            self.sash_hover.shows("peek-sash-split"),
            self.sash_hover.shows("peek-sash-height"),
        ];
        let sash = |id: &'static str, drag: PeekDrag, cx: &mut Context<Self>| {
            let shown = shown[usize::from(id == "peek-sash-height")];
            div()
                .id(id)
                .debug_selector(move || id.into())
                .absolute()
                .when(shown, |sash| sash.bg(sash_hover))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.sash_hover.set_hovered(
                        id,
                        *hovered,
                        |this: &mut Self| &mut this.sash_hover,
                        cx,
                    );
                }))
                .on_drag(drag, |_, _, _, cx| cx.new(|_| gpui::Empty))
        };
        // Monaco `peekViewWidget.css`: the action bar's close action.
        let close = div()
            .id("peek-close")
            .debug_selector(|| "peek-close".into())
            .flex_none()
            .p(px(3.))
            .mr(px(2.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_color(colors.peek_view_title_foreground)
            .hover(|style| colors.toolbar_hover(style))
            .tooltip(crate::tooltip::tooltip("Close", &colors))
            .child(codicon(Codicon::Close))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)));
        // `.peekview-title`: 13px, 20px in, the folder `.9em` and `.5em`
        // after the name, and the meta after a `-`. Clickable: it opens the
        // focused result (Monaco `supportOnTitleClick`).
        let title = div()
            .id("peek-title")
            .debug_selector(|| "peek-title".into())
            .flex()
            .items_baseline()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .ml(px(20.))
            .text_size(px(13.))
            .cursor_pointer()
            .child(
                div()
                    .flex_none()
                    .text_color(colors.peek_view_title_foreground)
                    .child(title),
            )
            .when(!description.is_empty(), |title| {
                title.child(
                    div()
                        .ml(px(6.5))
                        .text_size(px(11.7))
                        .flex_shrink_1()
                        .min_w_0()
                        .text_ellipsis()
                        .text_color(colors.peek_view_title_description_foreground)
                        .child(description),
                )
            })
            .child(
                div()
                    .flex_none()
                    .flex()
                    .text_color(colors.peek_view_title_description_foreground)
                    .child(div().px(px(3.9)).child("-"))
                    .child(meta),
            )
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                let modifiers = event.modifiers();
                let side = modifiers.control || modifiers.platform || modifiers.alt;
                this.open_selected(true, side, cx);
            }));
        let list = div()
            .id("peek-results")
            .debug_selector(|| "peek-results".into())
            // With the editor's keys, for its preconditions.
            .key_context(crate::context_keys::widget_key_context(
                PEEK_KEY_CONTEXT,
                &crate::context_keys::editor_keys_for_widgets(&self.editor, cx),
            ))
            .track_focus(&self.focus)
            .relative()
            .h_full()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .bg(colors.peek_view_result_background)
            .text_color(colors.peek_view_result_line_foreground)
            .text_size(px(13.))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.list_hovered = *hovered;
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll_wheel(event, cx)))
            .on_key_down(cx.listener(|this, event, _, cx| this.type_to_navigate(event, cx)))
            .on_action(cx.listener(|this, _: &PeekSelectNext, _, cx| this.move_in_list(true, cx)))
            .on_action(
                cx.listener(|this, _: &PeekSelectPrevious, _, cx| this.move_in_list(false, cx)),
            )
            .on_action(cx.listener(|this, _: &PeekSelectFirst, _, cx| this.focus_edge(false, cx)))
            .on_action(cx.listener(|this, _: &PeekSelectLast, _, cx| this.focus_edge(true, cx)))
            .on_action(cx.listener(|this, _: &PeekPageDown, _, cx| this.move_by_page(true, cx)))
            .on_action(cx.listener(|this, _: &PeekPageUp, _, cx| this.move_by_page(false, cx)))
            .on_action(cx.listener(|this, _: &PeekOpen, _, cx| this.open_focused(cx)))
            .on_action(cx.listener(|this, _: &PeekOpenToSide, _, cx| {
                if matches!(this.focused, Row::Reference(..)) {
                    this.open_selected(true, true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PeekToggle, _, cx| {
                if let Row::File(g) = this.focused {
                    this.toggle_group(g, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PeekCollapse, _, cx| this.collapse(cx)))
            .on_action(cx.listener(|this, _: &PeekExpand, _, cx| this.expand(cx)))
            .on_action(cx.listener(|this, _: &ClosePeek, _, cx| {
                // Monaco's Escape keybinding is off with `stablePeek`.
                if this.stable_peek(cx) {
                    cx.propagate();
                } else {
                    this.close(cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &GoToNextReference, _, cx| this.select_relative(1, cx)),
            )
            .on_action(
                cx.listener(|this, _: &GoToPreviousReference, _, cx| this.select_relative(-1, cx)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(rows_top)
                    .children(visible_rows),
            )
            // Monaco's scrollbar shadow along the top once scrolled.
            .when(scrolled, |list| {
                list.child(
                    div()
                        .absolute()
                        .top(px(-6.))
                        .left_0()
                        .right_0()
                        .h(px(6.))
                        .shadow(vec![gpui::BoxShadow {
                            color: colors.scrollbar_shadow,
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(6.),
                            spread_radius: px(0.),
                            inset: false,
                        }]),
                )
            })
            .when_some(slider, |list, (top, size)| {
                list.child(
                    div()
                        .id("peek-scrollbar")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .w(LIST_SCROLLBAR_WIDTH)
                        // GPUI keeps a drag's cursor over the whole window
                        // (Monaco's pointer capture): the slider's arrow.
                        .cursor(gpui::CursorStyle::Arrow)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                // Monaco: a click on the track centers the
                                // slider there, then drags it.
                                this.slider_grab = size / 2.;
                                let y = this.list_y_of(event);
                                this.drag_slider(y - size / 2., cx);
                            }),
                        )
                        .on_drag(PeekDrag::Slider, |_, _, _, cx| cx.new(|_| gpui::Empty))
                        .when(show_slider, |track| {
                            track.child(
                                div()
                                    .id("peek-slider")
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .top(top)
                                    .h(size)
                                    .cursor(gpui::CursorStyle::Arrow)
                                    .bg(colors.scrollbar_slider)
                                    .hover(|style| style.bg(colors.scrollbar_slider_hover))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                            cx.stop_propagation();
                                            this.slider_grab = this.list_y_of(event) - top;
                                        }),
                                    )
                                    .on_drag(PeekDrag::Slider, |_, _, _, cx| {
                                        cx.new(|_| gpui::Empty)
                                    }),
                            )
                        }),
                )
            });
        div()
            .size_full()
            .relative()
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<PeekDrag>, _, cx| {
                let drag = *event.drag(cx);
                this.drag(drag, event, cx);
            }))
            .child(
                canvas(
                    move |bounds, _, _| zone_top.set(bounds.top()),
                    move |bounds, _, window, _| {
                        let x = bounds.left() + arrow_x;
                        let top = bounds.top();
                        let mut path = PathBuilder::fill();
                        path.move_to(point(x - arrow_height, top + arrow_height));
                        path.line_to(point(x, top));
                        path.line_to(point(x + arrow_height, top + arrow_height));
                        path.close();
                        if let Ok(path) = path.build() {
                            window.paint_path(path, border);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                // The zone widget's container: 1px frame top and bottom
                // (`.reference-zone-widget`), the shadow of
                // `.peekview-widget`, an arrow's height above and below.
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(arrow_height)
                    .h(head_height + body_height + px(3.))
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_b_1()
                    .border_color(border)
                    .shadow(crate::widgets::card::shadow_lg())
                    .child(
                        div()
                            .h(head_height)
                            // `_doLayoutHead`: the head's line height is its
                            // height.
                            .line_height(head_height)
                            .overflow_hidden()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .bg(colors.peek_view_title_background)
                            .child(title)
                            .child(close),
                    )
                    .child(
                        // `.peekview-widget > .body { border-top: 1px }`.
                        div()
                            .h(body_height + px(1.))
                            .flex_none()
                            .border_t_1()
                            .border_color(border)
                            .relative()
                            .flex()
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui::relative(self.ratio))
                                    .overflow_hidden()
                                    .children(preview),
                            )
                            .child(
                                sash("peek-sash-split", PeekDrag::Split, cx)
                                    .top_0()
                                    .bottom_0()
                                    .left(gpui::relative(self.ratio))
                                    .ml(px(-2.))
                                    .w(px(4.))
                                    .cursor_col_resize(),
                            )
                            .child(list),
                    ),
            )
            .child(
                // Monaco `getHorizontalSashTop`: the frame's bottom.
                sash("peek-sash-height", PeekDrag::Height, cx)
                    .left_0()
                    .right_0()
                    .top(arrow_height + head_height + body_height + px(3.) - px(2.))
                    .h(px(4.))
                    .cursor_row_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.sash_start = Some((event.position.y, this.height_in_lines));
                        }),
                    ),
            )
    }
}

impl PeekView {
    /// The y of `event` from the list's top (the zone's top plus the
    /// header).
    fn list_y_of(&self, event: &MouseDownEvent) -> Pixels {
        let (arrow, head, _) = self.layout_heights();
        let zone_top = self.zone_top.get();
        event.position.y - (zone_top + arrow + px(1.) + head + px(1.))
    }

    /// A drag in the peek view: Monaco's split view between preview and
    /// list, the zone widget's height sash (in whole lines from where the
    /// drag started, `floor`ed down and `ceil`ed up), or the list's slider.
    /// The layout is kept for the next peek view, like Monaco's
    /// `peekViewLayout`.
    fn drag(&mut self, drag: PeekDrag, event: &DragMoveEvent<PeekDrag>, cx: &mut Context<Self>) {
        let bounds = event.bounds;
        let position = event.event.position;
        match drag {
            PeekDrag::Split => {
                let width = bounds.size.width;
                if width <= px(MIN_PREVIEW_WIDTH + MIN_LIST_WIDTH) {
                    return;
                }
                let preview = (position.x - bounds.left())
                    .clamp(px(MIN_PREVIEW_WIDTH), width - px(MIN_LIST_WIDTH));
                self.ratio = preview / width;
                cx.default_global::<PeekLayout>().ratio = self.ratio;
                cx.notify();
            }
            PeekDrag::Height => {
                let Some((start_y, start_lines)) = self.sash_start else {
                    return;
                };
                let line_height = self.options.resolved_line_height();
                let delta = (position.y - start_y) / line_height;
                let delta = if delta < 0. {
                    delta.ceil()
                } else {
                    delta.floor()
                };
                let lines = start_lines + delta;
                if lines > MIN_LINES && lines < MAX_LINES && lines != self.height_in_lines {
                    cx.default_global::<PeekLayout>().height_in_lines = lines;
                    self.height_in_lines = lines;
                    cx.notify();
                    if let Some(editor) = self.editor.upgrade() {
                        editor.update(cx, |editor, cx| editor.set_peek_height(lines, cx));
                    }
                }
            }
            PeekDrag::Slider => {
                let (arrow, head, _) = self.layout_heights();
                let list_top = bounds.top() + arrow + px(1.) + head + px(1.);
                let y = position.y - list_top - self.slider_grab;
                self.drag_slider(y, cx);
            }
        }
    }

    /// Focuses the preview if the list has focus, else the list (Monaco
    /// `changeFocusBetweenPreviewAndReferences`).
    fn toggle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let preview_focused = self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.read(cx).focus_handle.contains_focused(window, cx));
        match self.preview.as_ref().filter(|_| !preview_focused) {
            Some(preview) => window.focus(&preview.read(cx).focus_handle.clone(), cx),
            None => window.focus(&self.focus, cx),
        }
    }
}

use gpui::prelude::FluentBuilder as _;

impl CodeEditor {
    /// Opens the peek view under the cursor's line with `locations` (Monaco
    /// `ReferencesController.toggleWidget`). `selected` defaults to the
    /// result nearest the cursor.
    pub(crate) fn open_peek(
        &mut self,
        kind: Option<LocationKind>,
        locations: Vec<Location>,
        selected: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.close_peek(cx);
        if locations.is_empty() {
            return;
        }
        let anchor = self.cursors.primary().position;
        let uri = self.model().read(cx).uri().cloned();
        let selected = selected.unwrap_or_else(|| nearest(&locations, uri.as_ref(), anchor));
        let editor = cx.entity();
        let model = self.model().clone();
        let theme = self.theme().clone();
        let options = self.options().clone();
        let focus_preview = self.options.peek_widget_default_focus
            == crate::options::PeekWidgetDefaultFocus::Editor;
        let view = cx.new(|cx| {
            let mut view = PeekView::new(
                &editor, model, theme, options, kind, anchor, locations, selected, cx,
            );
            view.focus_preview = focus_preview;
            view
        });
        // Monaco: the last height (18 lines at first), at most 80% of the
        // editor (at least 12 lines).
        let line_height = self.options.resolved_line_height();
        let height = cx
            .try_global::<PeekLayout>()
            .map_or(HEIGHT_IN_LINES, |layout| layout.height_in_lines);
        let max_lines = self.layout.as_ref().map_or(height, |layout| {
            (layout.text_bounds.size.height / line_height * 0.8).max(12.)
        });
        let height = height.min(max_lines);
        view.update(cx, |view, _| view.height_in_lines = height);
        // Monaco `ZoneWidget._showImpl`: after the anchor's line and
        // column (the view line showing it, on a wrapped line).
        let mut zone = crate::widgets::ViewZone::new(
            anchor.row + 1,
            ZoneHeight::Lines(height),
            view.clone().into(),
        );
        zone.after_column = Some(anchor.column);
        let id = self.add_zone(zone, true, cx);
        // Monaco's peek is a zone widget: an overlay widget, outside the
        // editor's scrollable element.
        self.widgets.set_zone_captures_wheel(id);
        self.peek = PeekState {
            zone: Some(id),
            view: Some(view),
        };
        // Monaco reveals the anchor line and the line after the zone.
        let start = PointUtf16::new(anchor.row, 0);
        let end = PointUtf16::new(anchor.row + 1, 0);
        self.reveal_range(start..end, RevealType::Simple, cx);
    }

    /// Whether a peek view is open.
    pub fn is_peek_visible(&self) -> bool {
        self.peek.view.is_some()
    }

    /// Sets the peek view's zone height in lines (its sash; the view keeps
    /// its own copy).
    fn set_peek_height(&mut self, lines: f32, cx: &mut Context<Self>) {
        if let Some(zone) = self.peek.zone {
            self.set_view_zone_height(zone, ZoneHeight::Lines(lines), cx);
        }
    }

    /// Moves focus between the peek view's preview and its results list
    /// (Monaco `togglePeekWidgetFocus`, Cmd/Ctrl+K F2). Returns whether a
    /// peek view was open; in a preview editor, it's the outer editor's.
    pub fn toggle_peek_widget_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.peek.view.clone() else {
            return false;
        };
        view.update(cx, |view, cx| view.toggle_focus(window, cx));
        true
    }

    /// Whether the peek view's preview editor has focus.
    pub fn is_peek_preview_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.peek_preview(cx)
            .is_some_and(|preview| preview.read(cx).focus_handle.contains_focused(window, cx))
    }

    /// The peek view's results, as (uri, range) with the selected index.
    pub fn peek_results(&self, cx: &gpui::App) -> Option<(Vec<Location>, usize)> {
        let view = self.peek.view.as_ref()?.read(cx);
        let references = view.references();
        let selected = references.iter().position(|r| *r == view.selected)?;
        let locations = references
            .into_iter()
            .map(|(g, r)| Location {
                uri: view.groups[g].uri.clone(),
                range: view.groups[g].range(r, cx),
                ..Default::default()
            })
            .collect();
        Some((locations, selected))
    }

    /// The peek view's results list, for tests: each row's label (a file's
    /// name, or two spaces and a result's preview), the focused row, the
    /// list's scroll offset and the rows rendered last.
    #[doc(hidden)]
    pub fn peek_list_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(Vec<String>, usize, Pixels, Range<usize>)> {
        let view = self.peek.view.as_ref()?.read(cx);
        let rows = view.rows();
        let labels = rows
            .iter()
            .map(|row| match row {
                Row::File(_) => view.row_label(*row, cx),
                Row::Reference(..) => format!("  {}", view.row_label(*row, cx)),
            })
            .collect();
        Some((
            labels,
            view.focused_index(&rows),
            view.scroll_top,
            view.rendered.clone(),
        ))
    }

    /// Whether the peek view's sash `id` (`peek-sash-split` or
    /// `peek-sash-height`) shows `sash.hoverBorder` for the mouse resting
    /// on it, for tests.
    #[doc(hidden)]
    pub fn peek_sash_hover_for_tests(&self, id: &'static str, cx: &gpui::App) -> bool {
        self.peek
            .view
            .as_ref()
            .is_some_and(|view| view.read(cx).sash_hover.shows(id))
    }

    /// The peek view's preview editor, once it has rendered.
    pub fn peek_preview(&self, cx: &gpui::App) -> Option<Entity<CodeEditor>> {
        self.peek.view.as_ref()?.read(cx).preview.clone()
    }

    /// Closes the peek view (Monaco `closeReferenceSearch`, Escape).
    /// Returns whether one was open.
    pub fn close_peek(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(zone) = self.peek.zone.take() else {
            return false;
        };
        // The view clears its highlights when it's released.
        self.peek.view = None;
        self.remove_widget(zone, cx);
        // Monaco focuses the editor again.
        if !self.in_peek {
            self.refocus = true;
        }
        true
    }

    /// F4 / Shift+F4 (F12 / Shift+F12 in the list) with the peek view open: the next or previous result.
    /// Returns whether a peek view was open.
    pub fn peek_select_relative(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.peek.view.clone() else {
            return false;
        };
        view.update(cx, |view, cx| view.select_relative(delta, cx));
        true
    }

    /// Monaco `ReferencesController._gotoReference` / `openReference`.
    fn peek_open_location(
        &mut self,
        location: Location,
        close: bool,
        side: bool,
        (kind, locations, selected): (Option<LocationKind>, Vec<Location>, usize),
        cx: &mut Context<Self>,
    ) {
        // Monaco `openReference` to the side: the peek view stays open and
        // the host opens the result beside this editor.
        if side {
            self.open_location_with_openers(location, true, cx);
            return;
        }
        let here = self.model().read(cx).is_at(location.uri.as_deref());
        // Monaco `OneReference.range`: a link's target selection range.
        let range = location.selection_range();
        if here && !close {
            // Move the peek view to the result, keeping its list (Monaco
            // focuses the list then).
            let start = range.start;
            self.set_selections(&[crate::cursor::Selection::collapsed(start)], cx);
            self.open_peek(kind, locations, Some(selected), cx);
            if let Some(view) = &self.peek.view {
                view.update(cx, |view, _| view.focus_preview = false);
            }
            return;
        }
        self.close_peek(cx);
        if here {
            let start = range.start;
            self.set_selections(&[crate::cursor::Selection::collapsed(start)], cx);
            // Standalone Monaco's `openCodeEditor` centers the selection
            // at once (`StandaloneCodeEditorService.doOpenEditor`).
            self.reveal_range_with_scroll_type(
                range,
                RevealType::Center,
                ScrollType::Immediate,
                cx,
            );
        } else {
            self.open_location_with_openers(location, false, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews() {
        assert_eq!(
            line_preview("    let total = add(1, 2);", 16, Some(19)),
            (
                "total = ".to_string(),
                "add".to_string(),
                "(1, 2);".to_string()
            )
        );
        // A long word before the match is kept whole.
        assert_eq!(
            line_preview("x = some_long_identifier.call()", 25, Some(29)),
            (
                "some_long_identifier.".to_string(),
                "call".to_string(),
                "()".to_string()
            )
        );
        // Multi-line matches run to the end of the first line.
        assert_eq!(line_preview("fn f() {", 3, None).1, "f() {");
    }

    #[test]
    fn in_memory_models_are_titled_references() {
        // Monaco `_revealReference`: `inmemory:` models (and so models
        // without a URI) are "References", whatever the kind.
        let references = ("References".to_string(), String::new());
        assert_eq!(peek_title(None), references);
        assert_eq!(peek_title(Some("inmemory://model/1")), references);
        assert_eq!(
            peek_title(Some("file:///src/lib.rs")),
            ("lib.rs".to_string(), "/src".to_string())
        );
    }

    #[test]
    fn labels_and_nearest() {
        assert_eq!(
            uri_label("file:///home/me/my%20project/src/main.rs"),
            ("main.rs".to_string(), "/home/me/my project/src".to_string())
        );
        let here = SharedString::from("file:///b.rs");
        let at = |uri: &str, row: u32| Location {
            uri: Some(uri.to_string().into()),
            range: PointUtf16::new(row, 0)..PointUtf16::new(row, 1),
            ..Default::default()
        };
        let locations = [
            at("file:///a.rs", 10),
            at("file:///b.rs", 1),
            at("file:///b.rs", 9),
        ];
        assert_eq!(nearest(&locations, Some(&here), PointUtf16::new(8, 0)), 2);
    }

    /// Monaco `referencesModel.test.ts` (`nearestReference`).
    #[test]
    fn nearest_reference() {
        let at = |uri: &str| Location {
            uri: Some(format!("file://{uri}").into()),
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 0),
            ..Default::default()
        };
        let model = [at("/out/obj/can"), at("/out/obj/can2"), at("/src/can")];
        let nearest_to = |uri: &str| {
            let uri = SharedString::from(format!("file://{uri}"));
            nearest(&model, Some(&uri), PointUtf16::new(0, 0))
        };
        assert_eq!(nearest_to("/src/can"), 2);
        assert_eq!(nearest_to("/src/someOtherFileInSrc"), 2);
        assert_eq!(nearest_to("/out/someOtherFile"), 0);
        assert_eq!(nearest_to("/out/obj/can2222"), 1);
    }

    #[test]
    fn rows_skip_the_file_with_one_file() {
        // Monaco `setInput(groups.length === 1 ? groups[0] : model)`.
        assert_eq!(
            list_rows(&[(3, false)]),
            vec![
                Row::Reference(0, 0),
                Row::Reference(0, 1),
                Row::Reference(0, 2)
            ]
        );
        assert_eq!(
            list_rows(&[(1, false), (2, true), (1, false)]),
            vec![
                Row::File(0),
                Row::Reference(0, 0),
                Row::File(1),
                Row::File(2),
                Row::Reference(2, 0)
            ]
        );
    }

    #[test]
    fn type_navigation() {
        let labels: Vec<String> = ["fn add", "add(1)", "add(2)", "main.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // A new word starts after the focused row; a longer one stays.
        assert_eq!(type_navigation_target(&labels, 1, "a", true), Some(2));
        assert_eq!(type_navigation_target(&labels, 1, "ad", false), Some(1));
        // Wraps, ignores case.
        assert_eq!(type_navigation_target(&labels, 3, "F", true), Some(0));
        // Two or more characters also match inside a label.
        assert_eq!(type_navigation_target(&labels, 0, "rs", true), Some(3));
        assert_eq!(type_navigation_target(&labels, 0, "s", true), None);
    }

    #[test]
    fn pages() {
        // Monaco `focusNextPage`/`focusPreviousPage`: ten 23px rows in a
        // 100px list.
        let h = px(23.);
        let down = |focused, scroll: f32| page_down(focused, 10, px(scroll), px(100.), h);
        let up = |focused, scroll: f32| page_up(focused, 10, px(scroll), px(100.), h);
        // To the last row in view, then a page on.
        assert_eq!(down(0, 0.), (3, px(0.)));
        assert_eq!(down(3, 0.), (7, px(100.)));
        // At the end it stays on the last row.
        assert_eq!(down(9, 130.), (9, px(130.)));
        // Up: the first fully visible row, then a page back.
        assert_eq!(up(7, 100.), (5, px(100.)));
        assert_eq!(up(5, 100.), (0, px(0.)));
        assert_eq!(up(0, 0.), (0, px(0.)));
    }
}
