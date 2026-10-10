//! The peek view: navigation results in a view zone, with a preview editor
//! and a results list grouped by file.
//!
//! Ports Monaco's `src/vs/editor/contrib/gotoSymbol/browser/peek/`
//! (`referencesController.ts`, `referencesWidget.ts`), the peek view and
//! zone widget frames (`contrib/peekView/`, `contrib/zoneWidget/`) (MIT).

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, ParentElement, PathBuilder, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, canvas, div, point, px,
};
use text::PointUtf16;

use crate::actions::*;
use crate::api::DecorationsCollection;
use crate::decorations::DecorationOptions;
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::model::{TextModel, model_for_uri};
use crate::mouse_target::MouseTargetKind;
use crate::options::LocationKind;
use crate::providers::Location;
use crate::theme::{EditorColors, Theme};
use crate::view::reveal::RevealType;
use crate::widgets::{WidgetId, ZoneHeight};

/// The key context of the results list.
pub const PEEK_KEY_CONTEXT: &str = "PeekView";

/// Monaco `LayoutData`: the preview takes this share of the width.
const PREVIEW_RATIO: f32 = 0.7;
/// Monaco `LayoutData.heightInLines`.
const HEIGHT_IN_LINES: f32 = 18.;

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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    File(usize),
    Reference(usize, usize),
}

/// The peek view's content (Monaco `ReferenceWidget`).
pub(crate) struct PeekView {
    editor: WeakEntity<CodeEditor>,
    kind: LocationKind,
    anchor: PointUtf16,
    groups: Vec<Group>,
    /// (group, reference).
    selected: (usize, usize),
    preview: Option<Entity<CodeEditor>>,
    no_preview: Entity<TextModel>,
    preview_theme: Arc<Theme>,
    options: crate::options::EditorOptions,
    colors: EditorColors,
    focus: FocusHandle,
    focus_pending: bool,
    scroll: ScrollHandle,
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
            let start = location.range.start;
            let distance = start.row.abs_diff(position.row) as u64 * 100
                + start.column.abs_diff(position.column) as u64;
            (std::cmp::Reverse(prefix(location.uri.as_ref())), distance)
        })
        .unwrap_or(0)
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

fn kind_title(kind: LocationKind) -> &'static str {
    match kind {
        LocationKind::Definition => "Definitions",
        LocationKind::Declaration => "Declarations",
        LocationKind::TypeDefinition => "Type Definitions",
        LocationKind::Implementation => "Implementations",
        LocationKind::References => "References",
    }
}

impl PeekView {
    #[allow(clippy::too_many_arguments)]
    fn new(
        editor: &Entity<CodeEditor>,
        main_model: Entity<TextModel>,
        theme: Arc<Theme>,
        options: crate::options::EditorOptions,
        kind: LocationKind,
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
            groups[group].ranges.push(location.range);
        }
        let mut preview_theme = (*theme).clone();
        preview_theme.colors.background = theme.colors.peek_view_editor_background;
        preview_theme.colors.gutter_background = theme.colors.peek_view_editor_background;
        let mut preview_options = options.clone();
        preview_options.minimap.enabled = false;
        preview_options.sticky_scroll.enabled = false;
        preview_options.glyph_margin = false;
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
            preview: None,
            no_preview: cx.new(|_| TextModel::new("no preview available")),
            colors: theme.colors.clone(),
            preview_theme: Arc::new(preview_theme),
            options: preview_options,
            focus: cx.focus_handle(),
            focus_pending: true,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (g, group) in self.groups.iter().enumerate() {
            rows.push(Row::File(g));
            if !group.collapsed {
                rows.extend((0..group.ranges.len()).map(|r| Row::Reference(g, r)));
            }
        }
        rows
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
        }
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
        // A double click in the preview opens the result (Monaco).
        self._subscriptions
            .push(cx.subscribe(&preview, |this, _, event, cx| {
                if let CodeEditorEvent::MouseDown(mouse) = event
                    && mouse.click_count == 2
                    && mouse.target.kind == MouseTargetKind::ContentText
                {
                    this.open_selected(true, cx);
                }
            }));
        self.preview = Some(preview);
        self.reveal_selected(cx);
    }

    /// Shows the selected result in the preview (Monaco `_revealReference`).
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
            if preview.model() != &model {
                preview.set_model(model, cx);
            }
            if let Some(range) = range {
                let start = range.start;
                preview.set_selections(&[crate::cursor::Selection::collapsed(start)], cx);
                preview.reveal_range(range, RevealType::Center, cx);
            }
        });
        cx.notify();
    }

    fn select(&mut self, selected: (usize, usize), cx: &mut Context<Self>) {
        self.selected = selected;
        self.groups[selected.0].collapsed = false;
        if let Some(ix) = self
            .rows()
            .iter()
            .position(|row| *row == Row::Reference(selected.0, selected.1))
        {
            self.scroll.scroll_to_item(ix);
        }
        self.reveal_selected(cx);
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

    /// Selects the next (`forward`) or previous result among the visible
    /// rows, skipping file rows and collapsed files.
    fn move_in_list(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.rows();
        let current = rows
            .iter()
            .position(|row| *row == Row::Reference(self.selected.0, self.selected.1));
        let next = match (current, forward) {
            (Some(ix), true) => rows[ix + 1..].iter().find_map(|row| match row {
                Row::Reference(g, r) => Some((*g, *r)),
                Row::File(_) => None,
            }),
            (Some(ix), false) => rows[..ix].iter().rev().find_map(|row| match row {
                Row::Reference(g, r) => Some((*g, *r)),
                Row::File(_) => None,
            }),
            (None, _) => rows.iter().find_map(|row| match row {
                Row::Reference(g, r) => Some((*g, *r)),
                Row::File(_) => None,
            }),
        };
        if let Some(next) = next {
            self.select(next, cx);
        }
    }

    /// Enter or a double click: go to the selected result (Monaco
    /// `_gotoReference`). In this editor's model the peek view moves to it;
    /// elsewhere it closes and the host is asked to open it. A double click
    /// in the preview (`close`) always closes it (Monaco `openReference`).
    fn open_selected(&mut self, close: bool, cx: &mut Context<Self>) {
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
            })
            .collect();
        let results = (self.kind, locations, selected);
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.peek_open_location(location, close, results, cx)
        });
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
        cx.notify();
    }

    fn title(&self) -> (String, String) {
        match &self.groups[self.selected.0].uri {
            Some(uri) => uri_label(uri),
            None => (kind_title(self.kind).to_string(), String::new()),
        }
    }

    fn render_row(&self, ix: usize, row: Row, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = &self.colors;
        let height = px(22.);
        match row {
            Row::File(g) => {
                let group = &self.groups[g];
                let (name, dir) = group
                    .uri
                    .as_deref()
                    .map(uri_label)
                    .unwrap_or_else(|| ("(this document)".into(), String::new()));
                div()
                    .id(("peek-file", ix))
                    .h(height)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .text_color(colors.peek_view_result_file_foreground)
                    .child(if group.collapsed { "▸" } else { "▾" })
                    .child(name)
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(colors.peek_view_title_description_foreground)
                            .child(dir),
                    )
                    .child(
                        div()
                            .px(px(6.))
                            .rounded(px(9.))
                            .bg(colors.peek_view_result_selection_background)
                            .child(group.ranges.len().to_string()),
                    )
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_group(g, cx)),
                    )
                    .into_any_element()
            }
            Row::Reference(g, r) => {
                let group = &self.groups[g];
                let range = group.range(r, cx);
                let (before, inside, after) = match &group.model {
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
                };
                let selected = self.selected == (g, r);
                let foreground = if selected {
                    colors.peek_view_result_selection_foreground
                } else {
                    colors.peek_view_result_line_foreground
                };
                div()
                    .id(("peek-reference", ix))
                    .h(height)
                    .flex()
                    .items_center()
                    .pl(px(28.))
                    .pr(px(6.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(foreground)
                    .when(selected, |row| {
                        row.bg(colors.peek_view_result_selection_background)
                    })
                    .child(before)
                    .child(
                        div()
                            .bg(colors.peek_view_result_match_highlight_background)
                            .child(inside),
                    )
                    .child(after)
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.select((g, r), cx);
                        if event.click_count() == 2 {
                            this.open_selected(false, cx);
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
            window.focus(&self.focus, cx);
        }
        let colors = self.colors.clone();
        let line_height = self.options.resolved_line_height();
        // Monaco: the arrow is a third of a line tall, under the anchor.
        let arrow_height = (line_height / 3.).round();
        let arrow_x = self
            .editor
            .upgrade()
            .and_then(|editor| {
                let editor = editor.read(cx);
                let left = editor.layout.as_ref()?.bounds.left();
                let position = editor.scrolled_visible_position(self.anchor)?;
                Some(position.x - left)
            })
            .unwrap_or(px(30.));
        let border = colors.peek_view_border;
        let (title, description) = self.title();
        let meta = format!("{} ({})", kind_title(self.kind), self.reference_count());
        let rows = self.rows();
        let rows: Vec<_> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, row)| self.render_row(ix, row, cx))
            .collect();
        let preview = self.preview.clone();
        div()
            .size_full()
            .relative()
            .child(
                canvas(
                    |_, _, _| {},
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
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .top(arrow_height)
                    .flex()
                    .flex_col()
                    .border_t_2()
                    .border_b_2()
                    .border_color(border)
                    .child(
                        div()
                            .h((line_height * 1.2).ceil())
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(10.))
                            .bg(colors.peek_view_title_background)
                            .child(
                                div()
                                    .text_color(colors.peek_view_title_foreground)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_color(colors.peek_view_title_description_foreground)
                                    .child(description),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(colors.peek_view_title_description_foreground)
                                    .child(meta),
                            )
                            .child(
                                div()
                                    .id("peek-close")
                                    .px(px(4.))
                                    .cursor_pointer()
                                    .text_color(colors.peek_view_title_foreground)
                                    .child("✕")
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(
                                        cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui::relative(PREVIEW_RATIO))
                                    .overflow_hidden()
                                    .children(preview),
                            )
                            .child(
                                div()
                                    .id("peek-results")
                                    .key_context(PEEK_KEY_CONTEXT)
                                    .track_focus(&self.focus)
                                    .h_full()
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .bg(colors.peek_view_result_background)
                                    .text_size(px(13.))
                                    .on_action(cx.listener(|this, _: &PeekSelectNext, _, cx| {
                                        this.move_in_list(true, cx)
                                    }))
                                    .on_action(cx.listener(
                                        |this, _: &PeekSelectPrevious, _, cx| {
                                            this.move_in_list(false, cx)
                                        },
                                    ))
                                    .on_action(cx.listener(|this, _: &PeekSelectFirst, _, cx| {
                                        if let Some(first) = this.references().first().copied() {
                                            this.select(first, cx);
                                        }
                                    }))
                                    .on_action(cx.listener(|this, _: &PeekSelectLast, _, cx| {
                                        if let Some(last) = this.references().last().copied() {
                                            this.select(last, cx);
                                        }
                                    }))
                                    .on_action(cx.listener(|this, _: &PeekOpen, _, cx| {
                                        this.open_selected(false, cx)
                                    }))
                                    .on_action(cx.listener(|this, _: &PeekCollapse, _, cx| {
                                        let group = this.selected.0;
                                        this.groups[group].collapsed = true;
                                        cx.notify();
                                    }))
                                    .on_action(cx.listener(|this, _: &PeekExpand, _, cx| {
                                        let group = this.selected.0;
                                        this.groups[group].collapsed = false;
                                        cx.notify();
                                    }))
                                    .on_action(
                                        cx.listener(|this, _: &ClosePeek, _, cx| this.close(cx)),
                                    )
                                    .on_action(cx.listener(|this, _: &GoToNextReference, _, cx| {
                                        this.select_relative(1, cx)
                                    }))
                                    .on_action(cx.listener(
                                        |this, _: &GoToPreviousReference, _, cx| {
                                            this.select_relative(-1, cx)
                                        },
                                    ))
                                    .children(rows),
                            ),
                    ),
            )
    }
}

use gpui::prelude::FluentBuilder as _;

impl CodeEditor {
    /// Opens the peek view under the cursor's line with `locations` (Monaco
    /// `ReferencesController.toggleWidget`). `selected` defaults to the
    /// result nearest the cursor.
    pub(crate) fn open_peek(
        &mut self,
        kind: LocationKind,
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
        let view = cx.new(|cx| {
            PeekView::new(
                &editor, model, theme, options, kind, anchor, locations, selected, cx,
            )
        });
        // Monaco: 18 lines, at most 80% of the editor (at least 12 lines).
        let line_height = self.options.resolved_line_height();
        let max_lines = self.layout.as_ref().map_or(HEIGHT_IN_LINES, |layout| {
            (layout.text_bounds.size.height / line_height * 0.8).max(12.)
        });
        let id = self.add_zone(
            anchor.row + 1,
            ZoneHeight::Lines(HEIGHT_IN_LINES.min(max_lines)),
            view.clone().into(),
            true,
            cx,
        );
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
            })
            .collect();
        Some((locations, selected))
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

    /// F4 / Shift+F4 with the peek view open: the next or previous result.
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
        (kind, locations, selected): (LocationKind, Vec<Location>, usize),
        cx: &mut Context<Self>,
    ) {
        let here = location.uri.is_none() || location.uri.as_ref() == self.model().read(cx).uri();
        if here && !close {
            // Move the peek view to the result, keeping its list.
            let start = location.range.start;
            self.set_selections(&[crate::cursor::Selection::collapsed(start)], cx);
            self.open_peek(kind, locations, Some(selected), cx);
            return;
        }
        self.close_peek(cx);
        if here {
            let start = location.range.start;
            self.set_selections(&[crate::cursor::Selection::collapsed(start)], cx);
            self.reveal_range(location.range, RevealType::Center, cx);
        } else {
            cx.emit(CodeEditorEvent::OpenLocation(location));
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
    fn labels_and_nearest() {
        assert_eq!(
            uri_label("file:///home/me/my%20project/src/main.rs"),
            ("main.rs".to_string(), "/home/me/my project/src".to_string())
        );
        let here = SharedString::from("file:///b.rs");
        let at = |uri: &str, row: u32| Location {
            uri: Some(uri.to_string().into()),
            range: PointUtf16::new(row, 0)..PointUtf16::new(row, 1),
        };
        let locations = [
            at("file:///a.rs", 10),
            at("file:///b.rs", 1),
            at("file:///b.rs", 9),
        ];
        assert_eq!(nearest(&locations, Some(&here), PointUtf16::new(8, 0)), 2);
    }
}
