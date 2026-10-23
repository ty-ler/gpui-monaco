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
    PathBuilder, Pixels, Render, Role, ScrollWheelEvent, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, WeakEntity, Window, canvas, div, point, px,
};
use text::PointUtf16;

use crate::accessibility::AriaNode;
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
/// 16px wide with 6px right padding), and the left padding of a top-level
/// twistie (`TreeRenderer.DefaultIndent`).
pub(crate) const TREE_INDENT: Pixels = px(8.);
pub(crate) const TWISTIE_WIDTH: Pixels = px(22.);
pub(crate) const TREE_DEFAULT_INDENT: Pixels = px(8.);
/// `.monaco-tl-twistie { transform: translateX(3px) }`: the chevron sits
/// 3px right of the twistie's 16px box.
pub(crate) const TWISTIE_SHIFT: Pixels = px(3.);
/// `margin-left: 0.5em` on text in `font-size: 0.9em` of 13px: an
/// `IconLabel`'s description and the peek title's folder.
pub(crate) const DESCRIPTION_GAP: Pixels = px(5.85);

/// The opacity of an `IconLabel`'s description: `.7`, `.95` in the light
/// theme (`.vs`, not high contrast light), and `1` on a selected row.
pub(crate) fn label_description_opacity(light: bool, selected: bool) -> f32 {
    if selected {
        1.
    } else if light {
        0.95
    } else {
        0.7
    }
}
/// Monaco's list scrollbar (`ScrollableElement`'s default
/// `verticalScrollbarSize`).
const LIST_SCROLLBAR_WIDTH: Pixels = px(10.);
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
}

#[derive(Default)]
pub(crate) struct PeekState {
    zone: Option<WidgetId>,
    view: Option<Entity<PeekView>>,
    /// The references request of `editor.action.findReferences`, whose
    /// peek view shows "Loading..." until it's done (Monaco
    /// `toggleWidget`'s `modelPromise`, cancelled when the view closes).
    task: Option<Task<()>>,
}

/// Where the focus goes after F4 moves the peek view (Monaco
/// `goToNextOrPreviousReference`): back to the editor or the preview if
/// one of them had it, else the results list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PeekFocus {
    Editor,
    Preview,
    List,
}

/// The peek view's results, handed to its editor (which can't read the
/// view while the view is calling it).
pub(crate) struct PeekResults {
    kind: Option<LocationKind>,
    /// Where the view is (Monaco `ZoneWidget.position`).
    anchor: PointUtf16,
    locations: Vec<Location>,
    /// The selected result's index in `locations`.
    selected: usize,
    peek_mode: bool,
}

/// How the peek view goes to a result (Monaco `ReferenceWidget`'s
/// `SelectionEvent.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenKind {
    /// Enter or a double click in the list: in peek mode the view moves to
    /// it, else it closes and the result is selected.
    Goto,
    /// A click on the title or a double click in the preview: the view
    /// closes and the result is selected.
    Open,
    /// The same with Ctrl, Cmd or Alt: the host opens it beside this
    /// editor and the view stays.
    Side,
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
    /// Monaco `ReferencesController._peekMode`: opened by a Peek command,
    /// so going to a result in this model moves the view there instead of
    /// closing it.
    peek_mode: bool,
    /// The results are still being found: no list or preview yet, and the
    /// title says "Loading..." (Monaco `toggleWidget`).
    loading: bool,
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
    /// The editor's line height with the font zoom (Monaco
    /// `EditorOption.lineHeight`), refreshed every frame.
    line_height: Pixels,
    colors: EditorColors,
    high_contrast: bool,
    /// Monaco's `vs` base: a light theme that isn't high contrast.
    light: bool,
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
    /// The mouse is over the list, which shows its indent guides.
    list_hovered: bool,
    /// The list's scrollbar.
    scrollbar: crate::scrollbars::ListScrollbar,
    /// A height sash drag's start: the pointer's y and the height in lines
    /// (Monaco `ZoneWidget._initSash`'s `data`).
    sash_start: Option<(Pixels, f32)>,
    /// The sash showing `sash.hoverBorder` for the mouse resting on it.
    sash_hover: crate::sash::SashHover,
    /// The sash under a pressed mouse, which shows `sash.hoverBorder` at
    /// once (Monaco's `active` class).
    sash_active: Option<&'static str>,
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

/// The last path segment of `uri` (Monaco `basenameOrAuthority`) and the
/// label of the folder before it (`getUriLabel(dirname(uri))`, which the
/// standalone label service makes a `file` uri's `fsPath` and any other
/// uri's `path`: `/` for `inmemory://model/1`).
fn uri_label(uri: &str) -> (String, String) {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return (percent_decode(uri), String::new());
    };
    let (authority, path) = match rest.find('/') {
        Some(ix) => rest.split_at(ix),
        None => (rest, ""),
    };
    // Monaco `posix.basename` and `posix.dirname` ignore a trailing slash.
    let trimmed = path.trim_end_matches('/');
    let (dir, name) = match trimmed.rsplit_once('/') {
        Some(("", name)) => ("/", name),
        Some((dir, name)) => (dir, name),
        // `dirname` keeps a uri without a path as it is.
        None => (path, trimmed),
    };
    let name = if name.is_empty() { authority } else { name };
    let description = if scheme == "file" {
        fs_path(&format!("file://{authority}{dir}"))
    } else {
        percent_decode(dir)
    };
    (percent_decode(name), description)
}

/// Monaco `URI.fsPath`: the path, after the authority (kept for a `file`
/// URI's UNC path).
fn fs_path(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return percent_decode(uri);
    };
    let (authority, path) = match rest.find('/') {
        Some(ix) => rest.split_at(ix),
        None => (rest, ""),
    };
    if scheme == "file" && !authority.is_empty() && path.len() > 1 {
        percent_decode(&format!("//{authority}{path}"))
    } else {
        percent_decode(path)
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
        let (groups, selected_pos) = Self::groups(&main_model, locations, selected, cx);
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
            peek_mode: false,
            loading: false,
            anchor,
            groups,
            selected: selected_pos,
            focused: Row::Reference(selected_pos.0, selected_pos.1),
            preview: None,
            no_preview: cx.new(|_| TextModel::new_internal("no preview available")),
            colors: theme.colors.clone(),
            high_contrast: theme.high_contrast,
            light: theme.kind == crate::theme::ThemeKind::Light && !theme.high_contrast,
            preview_theme: Arc::new(preview_theme),
            // Until the opening editor sets its zoomed height.
            line_height: preview_options.resolved_line_height(),
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
            scrollbar: Default::default(),
            sash_start: None,
            sash_hover: Default::default(),
            sash_active: None,
            typed: None,
            zone_top: Default::default(),
            rendered: 0..0,
            _subscriptions: subscriptions,
        }
    }

    /// `locations` grouped by model, with the position of `selected`.
    fn groups(
        main_model: &Entity<TextModel>,
        locations: Vec<Location>,
        selected: usize,
        cx: &mut gpui::App,
    ) -> (Vec<Group>, (usize, usize)) {
        let mut groups: Vec<Group> = Vec::new();
        let mut selected_pos = (0, 0);
        for (ix, location) in locations.into_iter().enumerate() {
            if groups.last().is_none_or(|g| g.uri != location.uri) {
                let model = if main_model.read(cx).is_at(location.uri.as_deref()) {
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
        (groups, selected_pos)
    }

    /// The results of a view opened while loading (Monaco `toggleWidget`
    /// once its `modelPromise` resolves): the list and the preview with
    /// the result nearest the anchor selected and focused, or "No results".
    fn set_results(
        &mut self,
        main_model: &Entity<TextModel>,
        locations: Vec<Location>,
        cx: &mut Context<Self>,
    ) {
        let uri = main_model.read(cx).resource().clone();
        let selected = nearest(&locations, Some(&uri), self.anchor);
        let (groups, selected) = Self::groups(main_model, locations, selected, cx);
        self.focus_pending = !groups.is_empty();
        self.groups = groups;
        self.selected = selected;
        self.focused = Row::Reference(selected.0, selected.1);
        self.loading = false;
        cx.notify();
    }

    /// The results, for the editor (which can't read this view while it's
    /// calling the editor).
    fn results(&self, cx: &gpui::App) -> PeekResults {
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
        PeekResults {
            kind: self.kind,
            anchor: self.anchor,
            locations,
            selected,
            peek_mode: self.peek_mode,
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
        let line_height = self.line_height;
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
            self.scrollbar.reveal(cx);
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
        // Monaco shows the preview once there are results.
        if self.preview.is_some() || self.groups.is_empty() {
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
        // A double click in the preview opens the clicked place, selecting
        // the mouse target's range (Monaco `_onNewModel`'s `onMouseDown`),
        // unless `stablePeek`; with Ctrl, Cmd or Alt, to the side.
        self._subscriptions
            .push(cx.subscribe(&preview, |this, _, event, cx| {
                if let CodeEditorEvent::MouseDown(mouse) = event
                    && mouse.click_count == 2
                    && mouse.target.kind == MouseTargetKind::ContentText
                    && let Some(range) = mouse.target.range.clone()
                    && let Some(group) = this.groups.get(this.selected.0)
                {
                    let side =
                        mouse.modifiers.control || mouse.modifiers.platform || mouse.modifiers.alt;
                    if !side && this.stable_peek(cx) {
                        return;
                    }
                    let location = Location {
                        uri: group.uri.clone(),
                        range,
                        ..Default::default()
                    };
                    let kind = if side { OpenKind::Side } else { OpenKind::Open };
                    this.open(location, kind, cx);
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

    /// F4 / Shift+F4 (F12 / Shift+F12) in the list: goes to the next (or
    /// previous) result, keeping the focus in the list (Monaco
    /// `goToNextOrPreviousReference`).
    fn go_to_relative(&mut self, delta: isize, cx: &mut Context<Self>) {
        let results = self.results(cx);
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| {
                editor.peek_go_to_relative(results, delta, PeekFocus::List, cx)
            });
        }
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
            Row::Reference(..) => self.open_selected(OpenKind::Goto, cx),
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

    /// Monaco's `ariaMessage`s (`FileReferences` and `OneReference`): what
    /// a screen reader reads for `row`.
    fn row_aria_label(&self, row: Row, cx: &gpui::App) -> String {
        match row {
            Row::File(g) => {
                let name = self.file_label(g).0;
                let path = self.groups[g]
                    .uri
                    .as_deref()
                    .map(fs_path)
                    .unwrap_or_default();
                match self.groups[g].ranges.len() {
                    1 => format!("1 symbol in {name}, full path {path}"),
                    n => format!("{n} symbols in {name}, full path {path}"),
                }
            }
            Row::Reference(g, r) => {
                let name = self.file_label(g).0;
                let range = self.groups[g].range(r, cx);
                let (line, column) = (range.start.row + 1, range.start.column + 1);
                if self.groups[g].model.is_some() {
                    let preview = self.row_label(row, cx);
                    format!("{preview} in {name} on line {line} at column {column}")
                } else {
                    format!("in {name} on line {line} at column {column}")
                }
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

    /// Goes to the selected result as `kind` says (Monaco
    /// `onDidSelectReference`).
    fn open_selected(&mut self, kind: OpenKind, cx: &mut Context<Self>) {
        // Monaco's title click needs the preview's model: nothing found,
        // nothing to open.
        if self.groups.is_empty() {
            return;
        }
        let location = self.selected_location(cx);
        self.open(location, kind, cx);
    }

    fn open(&mut self, location: Location, kind: OpenKind, cx: &mut Context<Self>) {
        let results = self.results(cx);
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.peek_open_location(location, kind, results, cx)
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

    /// The meta title, after the title: the kind and the count, none while
    /// loading or without results (Monaco `setMetaTitle`).
    fn meta(&self) -> Option<String> {
        (!self.groups.is_empty())
            .then(|| format!("{} ({})", kind_title(self.kind), self.reference_count()))
    }

    /// The body's message, which Monaco shows instead of the preview and
    /// the list once nothing was found.
    fn message(&self) -> Option<&'static str> {
        (self.groups.is_empty() && !self.loading).then_some("No results")
    }

    /// The title and its description: "Loading..." until the results come,
    /// nothing without results (Monaco `toggleWidget`, `_onNewModel`).
    fn title(&self) -> (String, String) {
        match self.groups.get(self.selected.0) {
            Some(group) => peek_title(group.uri.as_deref()),
            None if self.loading => ("Loading...".into(), String::new()),
            None => Default::default(),
        }
    }

    /// The list's wheel: always consumed (Monaco's list has
    /// `alwaysConsumeMouseWheel`), so the editor behind doesn't scroll.
    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let delta = event.delta.pixel_delta(ROW_HEIGHT).y;
        let rows = self.rows().len();
        self.set_scroll_top(self.scroll_top - delta, rows, cx);
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
        // Monaco's tree items, read by their `ariaMessage`; the focused row
        // is the focused list's active descendant. Without a file row the
        // references are the tree's roots.
        let level = match row {
            Row::Reference(..) if !single_file => 2,
            _ => 1,
        };
        let aria = AriaNode::new(Role::TreeItem, self.row_aria_label(row, cx))
            .level(level)
            .selected(focused)
            .active_descendant(focused);
        let aria = match row {
            Row::File(g) => aria.expanded(!self.groups[g].collapsed),
            Row::Reference(..) => aria,
        };
        let id: gpui::ElementId = match row {
            Row::File(_) => ("peek-file", ix).into(),
            Row::Reference(..) => ("peek-reference", ix).into(),
        };
        let base = aria
            .apply(div().id(id))
            .h(ROW_HEIGHT)
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .relative()
            // `.monaco-list.mouse-support .monaco-list-row { cursor: pointer }`.
            .cursor_pointer()
            .when(focused, |row| {
                row.when_some(selection_background, |row, bg| row.bg(bg))
                    // `list.focusOutline`, inside the row.
                    .when_some(
                        colors.list_focus_outline.filter(|_| list_focused),
                        |row, c| row.border_1().border_color(c),
                    )
            });
        let base = list_row_states(base, colors, focused, list_focused);
        match row {
            Row::File(g) => {
                let group = &self.groups[g];
                let (name, dir) = self.file_label(g);
                base.pl(px(0.))
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
                            .w(TREE_DEFAULT_INDENT + TWISTIE_WIDTH)
                            .flex_none()
                            .flex()
                            .justify_end()
                            .pr(TWISTIE_SHIFT)
                            .child(codicon(if group.collapsed {
                                Codicon::ChevronRight
                            } else {
                                Codicon::TreeItemExpanded
                            })),
                    )
                    // `IconLabel`: the name, then the folder as its
                    // description (`.9em`, `.5em` apart).
                    .child(div().flex_none().child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ml(DESCRIPTION_GAP)
                            .text_size(px(11.7))
                            .opacity(label_description_opacity(self.light, focused))
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
                    TREE_DEFAULT_INDENT + TWISTIE_WIDTH
                } else {
                    TREE_DEFAULT_INDENT + TREE_INDENT + TWISTIE_WIDTH
                };
                // The file's guide; it's active while one of its
                // references or the expanded file has the focus.
                let guide_active = match self.focused {
                    Row::File(f) => f == g && !self.groups[g].collapsed,
                    Row::Reference(f, _) => f == g,
                };
                let guides = (!single_file)
                    .then(|| tree_indent_guides(&[guide_active], self.list_hovered, colors))
                    .flatten();
                base.pl(indent)
                    .children(guides)
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
                            this.open_selected(OpenKind::Goto, cx);
                        }
                    }))
                    .into_any_element()
            }
        }
    }
}

/// Monaco's list row styles for the mouse and high contrast
/// (`DefaultStyleController`): a row other than the selected one shows
/// `list.hoverBackground` and `list.hoverForeground` under the mouse; in high
/// contrast any hovered row gets a dashed `contrastActiveBorder` outline and
/// the selected row a dotted one while the list isn't focused (dashed here,
/// as GPUI has no dotted borders).
pub(crate) fn list_row_states(
    row: gpui::Stateful<gpui::Div>,
    colors: &EditorColors,
    selected: bool,
    list_focused: bool,
) -> gpui::Stateful<gpui::Div> {
    let hover_background = colors.list_hover_background.filter(|_| !selected);
    let hover_foreground = colors.list_hover_foreground.filter(|_| !selected);
    let contrast = colors.contrast_active_border;
    row.when_some(contrast.filter(|_| selected && !list_focused), |row, c| {
        row.border_1().border_dashed().border_color(c)
    })
    .when(
        hover_background.is_some() || hover_foreground.is_some() || contrast.is_some(),
        |row| {
            row.hover(move |style| {
                let style = match hover_background {
                    Some(bg) => style.bg(bg),
                    None => style,
                };
                let style = match hover_foreground {
                    Some(fg) => style.text_color(fg),
                    None => style,
                };
                match contrast {
                    Some(c) => style.border_1().border_dashed().border_color(c),
                    None => style,
                }
            })
        },
    )
}

/// The workbench tree's indent guides (`renderIndentGuides: onHover`): a 1px
/// line per ancestor, 8px apart from 16px in (`.monaco-tl-indent`). The
/// guides of the focused item's parent, or of the item itself while it's
/// expanded, are drawn in `tree.indentGuidesStroke`; the rest only while
/// the tree is hovered, in `tree.inactiveIndentGuidesStroke`. `active` has
/// one entry per ancestor, the top-level one first.
pub(crate) fn tree_indent_guides(
    active: &[bool],
    tree_hovered: bool,
    colors: &EditorColors,
) -> Option<gpui::Div> {
    let guides: Vec<_> = active
        .iter()
        .map(|&active| {
            let color = if active {
                colors.tree_indent_guides_stroke
            } else {
                colors
                    .tree_inactive_indent_guides_stroke
                    .filter(|_| tree_hovered)
            };
            div()
                .debug_selector(move || {
                    if color.is_some() {
                        "tree-indent-guide"
                    } else {
                        "tree-indent-guide-hidden"
                    }
                    .into()
                })
                .w(TREE_INDENT)
                .h_full()
                .flex_none()
                .when_some(color, |guide, c| guide.border_l_1().border_color(c))
        })
        .collect();
    (!guides.is_empty()).then(|| {
        div()
            .debug_selector(|| "tree-indent-guides".into())
            .absolute()
            .top_0()
            .left(px(16.))
            .h_full()
            .flex()
            .children(guides)
    })
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
        if let Some(editor) = self.editor.upgrade() {
            self.line_height = editor.read(cx).line_height();
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
        let meta = self.meta();
        let message = self.message();
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
        let scrollbar_opacity = self.scrollbar.opacity(window, cx);
        let scrollbar = self.scrollbar.render(
            "peek",
            LIST_SCROLLBAR_WIDTH,
            body_height,
            ROW_HEIGHT * row_count as f32,
            self.scroll_top,
            scrollbar_opacity,
            &colors,
            |this: &mut Self| &mut this.scrollbar,
            |this, top, cx| {
                let rows = this.rows().len();
                this.set_scroll_top(top, rows, cx);
            },
            cx,
        );
        let scrolled = self.scroll_top > px(0.);
        let preview = self.preview.clone();
        // Monaco `Sash`: `sash.hoverBorder` once the mouse has rested on
        // the sash for its hover delay.
        let sash_hover = colors.sash_hover_border;
        let shown = ["peek-sash-split", "peek-sash-height"]
            .map(|id| self.sash_hover.shows(id) || self.sash_active == Some(id));
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
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        this.sash_active = Some(id);
                        cx.notify();
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| {
                        this.sash_active = None;
                        cx.notify();
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| {
                        this.sash_active = None;
                        cx.notify();
                    }),
                )
                .on_drag(drag, |_, _, _, cx| cx.new(|_| gpui::Empty))
        };
        // Monaco `peekViewWidget.css`: the action bar's close action.
        let close = AriaNode::new(Role::Button, "Close")
            .apply(div().id("peek-close"))
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
                        .ml(DESCRIPTION_GAP)
                        .text_size(px(11.7))
                        .flex_shrink_1()
                        .min_w_0()
                        .text_ellipsis()
                        .text_color(colors.peek_view_title_description_foreground)
                        .child(description),
                )
            })
            // Monaco sets no meta title while loading or without results.
            .when_some(meta, |title, meta| {
                title.child(
                    div()
                        .flex_none()
                        .flex()
                        .text_color(colors.peek_view_title_description_foreground)
                        .child(div().px(px(3.9)).child("-"))
                        .child(meta),
                )
            })
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                let modifiers = event.modifiers();
                let side = modifiers.control || modifiers.platform || modifiers.alt;
                let kind = if side { OpenKind::Side } else { OpenKind::Open };
                this.open_selected(kind, cx);
            }));
        // Monaco's references tree, labelled "References".
        let list = AriaNode::new(Role::Tree, "References")
            .apply(div().id("peek-results"))
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
                this.scrollbar.set_hovered(*hovered, cx);
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
                    this.open_selected(OpenKind::Side, cx);
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
            .on_action(cx.listener(|this, _: &GoToNextReference, _, cx| this.go_to_relative(1, cx)))
            .on_action(
                cx.listener(|this, _: &GoToPreviousReference, _, cx| this.go_to_relative(-1, cx)),
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
                list.child(crate::scrollbars::list_top_shadow(&colors))
            })
            .children(scrollbar);
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
                            .map(|body| {
                                if self.groups.is_empty() {
                                    // Monaco hides the preview and the list
                                    // until there are results, and shows
                                    // "No results" without (`.messages`:
                                    // centered, `3em` down).
                                    return body.child(
                                        div()
                                            .debug_selector(|| "peek-messages".into())
                                            .size_full()
                                            .pt(px(39.))
                                            .flex()
                                            .justify_center()
                                            .text_size(px(13.))
                                            .text_color(colors.foreground)
                                            .children(message),
                                    );
                                }
                                body.child(
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
                                .child(list)
                            }),
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
    /// A drag in the peek view: Monaco's split view between preview and
    /// list, the zone widget's height sash (in whole lines from where the
    /// drag started, `floor`ed down and `ceil`ed up).
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
                let line_height = self.line_height;
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
    /// Opens the peek view with `locations` under the selection's start
    /// (Monaco `ReferencesController.toggleWidget`), or only closes the
    /// open one when the selection contains its position. `selected`
    /// defaults to the result nearest the selection's start. In
    /// `peek_mode` (a Peek command), going to a result in this model moves
    /// the view there instead of closing it.
    pub(crate) fn open_peek(
        &mut self,
        kind: Option<LocationKind>,
        locations: Vec<Location>,
        selected: Option<usize>,
        peek_mode: bool,
        cx: &mut Context<Self>,
    ) {
        if self.close_peek_at_selection(cx) {
            return;
        }
        let anchor = self.cursors.primary().selection().start();
        let uri = self.model().read(cx).resource().clone();
        let selected = selected.unwrap_or_else(|| nearest(&locations, Some(&uri), anchor));
        self.show_peek(kind, anchor, locations, selected, peek_mode, false, cx);
    }

    /// Monaco `toggleWidget`'s start: closes the peek view, returning
    /// whether the selection contains its position.
    fn close_peek_at_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let position = self.peek.view.as_ref().map(|view| view.read(cx).anchor);
        self.close_peek(cx);
        let selection = self.cursors.primary().selection();
        position
            .is_some_and(|position| selection.start() <= position && position <= selection.end())
    }

    /// Shows a peek view under `anchor` (Monaco `ReferenceWidget.show`),
    /// replacing the open one; `loading` until its results are set.
    #[allow(clippy::too_many_arguments)]
    fn show_peek(
        &mut self,
        kind: Option<LocationKind>,
        anchor: PointUtf16,
        locations: Vec<Location>,
        selected: usize,
        peek_mode: bool,
        loading: bool,
        cx: &mut Context<Self>,
    ) -> Entity<PeekView> {
        self.close_peek(cx);
        let editor = cx.entity();
        let model = self.model().clone();
        let theme = self.theme().clone();
        let options = self.options().clone();
        let line_height = self.line_height();
        let focus_preview = self.options.peek_widget_default_focus
            == crate::options::PeekWidgetDefaultFocus::Editor;
        let view = cx.new(|cx| {
            let mut view = PeekView::new(
                &editor, model, theme, options, kind, anchor, locations, selected, cx,
            );
            view.line_height = line_height;
            view.focus_preview = focus_preview;
            view.peek_mode = peek_mode;
            view.loading = loading;
            // Monaco focuses the list once there are results.
            view.focus_pending = !view.groups.is_empty();
            view
        });
        // Monaco: the last height (18 lines at first), at most 80% of the
        // editor (at least 12 lines).
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
            view: Some(view.clone()),
            task: None,
        };
        // Monaco `ZoneWidget._showImpl` reveals the anchor line and the
        // line after the zone, or puts the last line near the top.
        if anchor.row + 1 >= self.model().read(cx).line_count() {
            self.reveal_line_near_top(anchor.row, cx);
        } else {
            let start = PointUtf16::new(anchor.row, 0);
            let end = PointUtf16::new(anchor.row + 1, 0);
            self.reveal_range(start..end, RevealType::Simple, cx);
        }
        view
    }

    /// Peeks the references at `position`, else the cursor's (Monaco
    /// `editor.action.findReferences`): the peek view opens at once,
    /// "Loading..." until the references come and "No results" without
    /// any, not in peek mode. Run again at the open view's position, it
    /// closes it.
    pub(crate) fn find_references(&mut self, position: Option<PointUtf16>, cx: &mut Context<Self>) {
        if self.in_peek {
            return;
        }
        if let Some(position) = position {
            let position = self.model().read(cx).validate_position(position);
            self.set_selections(&[crate::cursor::Selection::collapsed(position)], cx);
        }
        if self.close_peek_at_selection(cx) {
            return;
        }
        let position = self.cursors.primary().position;
        let anchor = self.cursors.primary().selection().start();
        let model = self.model().clone();
        // Not compact, as Monaco's `getReferencesAtPosition(.., false, ..)`.
        let tasks = Self::request_locations(&model, LocationKind::References, position, false, cx);
        let view = self.show_peek(
            Some(LocationKind::References),
            anchor,
            Vec::new(),
            0,
            false,
            true,
            cx,
        );
        let uri = model.read(cx).resource().clone();
        self.peek.task = Some(cx.spawn(async move |this, cx| {
            let mut locations = Vec::new();
            for task in tasks {
                match task.await {
                    Ok(found) => locations.extend(found),
                    Err(err) => log::warn!("finding references failed: {err:#}"),
                }
            }
            let locations = crate::goto::sorted_locations(locations, &uri);
            this.update(cx, |this, cx| {
                // Monaco drops the results of a request whose view closed.
                if this.peek.view.as_ref() != Some(&view) {
                    return;
                }
                let model = this.model().clone();
                view.update(cx, |view, cx| view.set_results(&model, locations, cx));
            })
            .ok();
        }));
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

    /// The peek view's title, meta title and body message, for tests.
    #[doc(hidden)]
    pub fn peek_title_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(String, Option<String>, Option<&'static str>)> {
        let view = self.peek.view.as_ref()?.read(cx);
        Some((view.title().0, view.meta(), view.message()))
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
        self.peek.task = None;
        self.remove_widget(zone, cx);
        // Monaco focuses the editor again.
        if !self.in_peek {
            self.refocus = true;
        }
        true
    }

    /// F4 / Shift+F4 with the peek view open: goes to the result after (or
    /// before) the one nearest the view, wrapping, like Enter in peek mode
    /// (Monaco `goToNextReference`, `goToPreviousReference`): the cursor
    /// and the view move there, and the results list gets the focus. In
    /// another model, the view closes and the host's
    /// [editor openers](crate::register_editor_opener) get it. Returns
    /// whether a peek view was open.
    pub fn peek_select_relative(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        self.peek_step(delta, PeekFocus::List, cx)
    }

    /// [`CodeEditor::peek_select_relative`], giving the focus back to
    /// `focus` (Monaco gives it back to the editor or the preview if one of
    /// them had it).
    pub(crate) fn peek_step(
        &mut self,
        delta: isize,
        focus: PeekFocus,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.peek.view.clone() else {
            return false;
        };
        let results = view.read(cx).results(cx);
        self.peek_go_to_relative(results, delta, focus, cx);
        true
    }

    /// Where F4 from the editor gives the focus back: the preview if it has
    /// it, else this editor (the list handles its own F4).
    pub(crate) fn peek_focus(&self, window: &Window, cx: &gpui::App) -> PeekFocus {
        if self.is_peek_preview_focused(window, cx) {
            PeekFocus::Preview
        } else {
            PeekFocus::Editor
        }
    }

    /// Monaco `goToNextOrPreviousReference`: from the result nearest the
    /// view's position.
    fn peek_go_to_relative(
        &mut self,
        results: PeekResults,
        delta: isize,
        focus: PeekFocus,
        cx: &mut Context<Self>,
    ) {
        let count = results.locations.len();
        if count == 0 {
            return;
        }
        let uri = self.model().read(cx).resource().clone();
        let source = nearest(&results.locations, Some(&uri), results.anchor);
        let target = (source as isize + delta).rem_euclid(count as isize) as usize;
        self.peek_go_to_reference(results, target, focus, cx);
    }

    /// Monaco `ReferencesController._gotoReference`: the cursor goes to the
    /// start of result `target` and the peek view moves under it. Standalone
    /// Monaco opens no other model; the view closes and the host's openers
    /// get it.
    fn peek_go_to_reference(
        &mut self,
        results: PeekResults,
        target: usize,
        focus: PeekFocus,
        cx: &mut Context<Self>,
    ) {
        let location = results.locations[target].clone();
        if !self.model().read(cx).is_at(location.uri.as_deref()) {
            self.close_peek(cx);
            self.open_location_with_openers(location, false, cx);
            return;
        }
        // Monaco `OneReference.range`: a link's target selection range.
        let start = location.selection_range().start;
        self.set_selections(&[crate::cursor::Selection::collapsed(start)], cx);
        let view = self.show_peek(
            results.kind,
            start,
            results.locations,
            target,
            results.peek_mode,
            false,
            cx,
        );
        view.update(cx, |view, _| match focus {
            // `close_peek` gives it back to the editor.
            PeekFocus::Editor => view.focus_pending = false,
            PeekFocus::Preview => view.focus_preview = true,
            PeekFocus::List => view.focus_preview = false,
        });
    }

    /// Goes to a result from the peek view (Monaco
    /// `onDidSelectReference`): `Goto` in peek mode moves the view
    /// (`_gotoReference`); otherwise it closes and the result's whole range
    /// is selected (`openReference`), or the host opens it.
    fn peek_open_location(
        &mut self,
        location: Location,
        kind: OpenKind,
        results: PeekResults,
        cx: &mut Context<Self>,
    ) {
        match kind {
            // The peek view stays open and the host opens the result
            // beside this editor.
            OpenKind::Side => self.open_location_with_openers(location, true, cx),
            OpenKind::Goto if results.peek_mode => {
                let target = results.selected;
                self.peek_go_to_reference(results, target, PeekFocus::List, cx);
            }
            OpenKind::Goto | OpenKind::Open => {
                self.close_peek(cx);
                if self.model().read(cx).is_at(location.uri.as_deref()) {
                    // Monaco `OneReference.range`: a link's target
                    // selection range, selected whole.
                    let range = location.selection_range();
                    self.set_selections(
                        &[crate::cursor::Selection::new(range.start, range.end)],
                        cx,
                    );
                    // Standalone Monaco's `openCodeEditor` centers the
                    // selection at once (`StandaloneCodeEditorService.doOpenEditor`).
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
        // Monaco `getUriLabel(dirname(uri))`: a `file` uri's `fsPath` (a
        // UNC path keeps its server), any other uri's `path`.
        assert_eq!(
            uri_label("inmemory://model/1"),
            ("1".to_string(), "/".to_string())
        );
        assert_eq!(
            uri_label("file:///main.rs"),
            ("main.rs".to_string(), "/".to_string())
        );
        assert_eq!(
            uri_label("file://server/share/a.rs"),
            ("a.rs".to_string(), "//server/share".to_string())
        );
        assert_eq!(
            uri_label("untitled://host/notes/a%20b.txt"),
            ("a b.txt".to_string(), "/notes".to_string())
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
