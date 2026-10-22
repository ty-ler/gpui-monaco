//! Peek Call Hierarchy and Peek Type Hierarchy: a peek view with a preview
//! editor and a tree that loads each item's calls or types as it's
//! expanded.
//!
//! Ports VS Code's `callHierarchyPeek.ts` and `typeHierarchyPeek.ts`
//! (`contrib/callHierarchy`, `contrib/typeHierarchy`, MIT), which Monaco
//! doesn't have. Like the references peek view, it's a view zone under the
//! position it was opened at, and its list uses the peek view's keys.

use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Render, Role, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement, Styled, Task, WeakEntity, Window, div, px,
};
use text::PointUtf16;

use crate::accessibility::AriaNode;
use crate::actions::*;
use crate::api::DecorationsCollection;
use crate::codicons::{Codicon, icon as codicon};
use crate::decorations::DecorationOptions;
use crate::editor::CodeEditor;
use crate::model::{TextModel, model_for_uri};
use crate::peek::PEEK_KEY_CONTEXT;
use crate::providers::{
    CallHierarchyProvider, HierarchyItem, Location, TypeHierarchyProvider,
    call_hierarchy_providers, type_hierarchy_providers,
};
use crate::theme::{EditorColors, Theme};
use crate::view::reveal::{RevealType, ScrollType};
use crate::widgets::{WidgetId, ZoneHeight};

/// The zone's height in lines (the references peek view's default).
const HEIGHT_IN_LINES: f32 = 18.;
/// The preview's share of the width.
const PREVIEW_RATIO: f32 = 0.6;
/// The tree's rows, indent per level and twistie.
const ROW_HEIGHT: Pixels = px(22.);
const INDENT: Pixels = px(8.);
const TWISTIE_WIDTH: Pixels = px(22.);
/// The head's height.
const HEAD_HEIGHT: Pixels = px(30.);

/// Which hierarchy the view shows, and which way (VS Code
/// `CallHierarchyDirection`, `TypeHierarchyDirection`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HierarchyDirection {
    /// The calls to each item (VS Code `CallsTo`, "Incoming Calls").
    IncomingCalls,
    /// The calls each item makes (VS Code `CallsFrom`, "Outgoing Calls").
    OutgoingCalls,
    /// The types each item extends or implements.
    Supertypes,
    /// The types that extend or implement each item.
    Subtypes,
}

impl HierarchyDirection {
    fn is_call(self) -> bool {
        matches!(self, Self::IncomingCalls | Self::OutgoingCalls)
    }

    /// The other direction of the same hierarchy.
    pub fn toggled(self) -> Self {
        match self {
            Self::IncomingCalls => Self::OutgoingCalls,
            Self::OutgoingCalls => Self::IncomingCalls,
            Self::Supertypes => Self::Subtypes,
            Self::Subtypes => Self::Supertypes,
        }
    }

    /// The view's meta title (VS Code's `title.incoming`, ...).
    fn label(self) -> &'static str {
        match self {
            Self::IncomingCalls => "Callers Of",
            Self::OutgoingCalls => "Calls From",
            Self::Supertypes => "Supertypes Of",
            Self::Subtypes => "Subtypes Of",
        }
    }

    /// The message when the root has nothing (VS Code `empt.callsFrom`,
    /// ...).
    fn empty_message(self, name: &str) -> String {
        match self {
            Self::IncomingCalls => format!("No calls to '{name}'"),
            Self::OutgoingCalls => format!("No calls from '{name}'"),
            Self::Supertypes => format!("No supertypes of '{name}'"),
            Self::Subtypes => format!("No subtypes of '{name}'"),
        }
    }
}

/// The provider a view asks, the one that prepared its root.
#[derive(Clone)]
enum Provider {
    Call(Arc<dyn CallHierarchyProvider>),
    Type(Arc<dyn TypeHierarchyProvider>),
}

/// One item of the tree.
struct Node {
    item: HierarchyItem,
    depth: usize,
    /// Its children: `None` until asked for.
    children: Option<Vec<usize>>,
    expanded: bool,
    /// The children's request.
    loading: Option<Task<()>>,
    /// The model the preview shows for it, and the ranges it highlights
    /// there: the call sites (VS Code's `Call.locations`), else the item's
    /// name.
    preview_uri: Option<SharedString>,
    preview_ranges: Vec<Range<PointUtf16>>,
}

/// A loaded child: its item, and its call sites and their model.
type Child = (HierarchyItem, Option<SharedString>, Vec<Range<PointUtf16>>);

/// The hierarchy peek view's state on its editor.
#[derive(Default)]
pub(crate) struct HierarchyState {
    zone: Option<WidgetId>,
    view: Option<Entity<HierarchyPeek>>,
    /// The prepare request, until the root comes.
    task: Option<Task<()>>,
}

/// The view (VS Code `CallHierarchyTreePeekWidget`,
/// `TypeHierarchyTreePeekWidget`).
pub(crate) struct HierarchyPeek {
    editor: WeakEntity<CodeEditor>,
    main_model: Entity<TextModel>,
    direction: HierarchyDirection,
    provider: Option<Provider>,
    nodes: Vec<Node>,
    roots: Vec<usize>,
    focused: Option<usize>,
    /// Still preparing the root.
    loading: bool,
    preview: Option<Entity<CodeEditor>>,
    highlights: Option<DecorationsCollection>,
    no_preview: Entity<TextModel>,
    preview_theme: Arc<Theme>,
    options: crate::options::EditorOptions,
    colors: EditorColors,
    focus: FocusHandle,
    focus_pending: bool,
    height_in_lines: f32,
    line_height: Pixels,
    scroll_top: Pixels,
}

impl HierarchyPeek {
    fn new(
        editor: &Entity<CodeEditor>,
        main_model: Entity<TextModel>,
        theme: Arc<Theme>,
        options: crate::options::EditorOptions,
        direction: HierarchyDirection,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut preview_theme = (*theme).clone();
        preview_theme.colors.background = theme.colors.peek_view_editor_background;
        preview_theme.colors.gutter_background = theme.colors.peek_view_editor_gutter_background;
        let mut preview_options = options.clone();
        preview_options.scroll_beyond_last_line = false;
        preview_options.minimap.enabled = false;
        preview_options.read_only = true;
        cx.on_release(|this: &mut Self, cx| {
            if let Some(mut highlights) = this.highlights.take() {
                highlights.clear(cx);
            }
        })
        .detach();
        Self {
            editor: editor.downgrade(),
            main_model,
            direction,
            provider: None,
            nodes: Vec::new(),
            roots: Vec::new(),
            focused: None,
            loading: true,
            preview: None,
            highlights: None,
            no_preview: cx.new(|_| TextModel::new_internal("no preview available")),
            preview_theme: Arc::new(preview_theme),
            line_height: preview_options.resolved_line_height(),
            options: preview_options,
            colors: theme.colors.clone(),
            focus: cx.focus_handle(),
            focus_pending: true,
            height_in_lines: HEIGHT_IN_LINES,
            scroll_top: px(0.),
        }
    }

    /// Sets the root the provider prepared (VS Code shows the first item)
    /// and expands it.
    fn set_root(&mut self, root: Option<(Provider, HierarchyItem)>, cx: &mut Context<Self>) {
        self.loading = false;
        self.nodes.clear();
        self.roots.clear();
        self.focused = None;
        let Some((provider, item)) = root else {
            cx.notify();
            return;
        };
        self.provider = Some(provider);
        self.nodes.push(Node {
            preview_uri: item.uri.clone(),
            preview_ranges: vec![item.selection_range.clone()],
            item,
            depth: 0,
            children: None,
            expanded: false,
            loading: None,
        });
        self.roots.push(0);
        self.focused = Some(0);
        self.set_expanded(0, true, cx);
        self.show_focused(cx);
    }

    /// Shows the other direction, from the same root (VS Code's
    /// "Show Incoming/Outgoing Calls" in the view).
    pub(crate) fn toggle_direction(&mut self, cx: &mut Context<Self>) {
        self.set_direction(self.direction.toggled(), cx);
    }

    fn set_direction(&mut self, direction: HierarchyDirection, cx: &mut Context<Self>) {
        if direction == self.direction {
            return;
        }
        self.direction = direction;
        let Some(&root) = self.roots.first() else {
            cx.notify();
            return;
        };
        let item = self.nodes[root].item.clone();
        let provider = self.provider.clone();
        self.set_root(provider.map(|provider| (provider, item)), cx);
    }

    /// The tree's visible rows, in order.
    fn rows(&self) -> Vec<usize> {
        let mut rows = Vec::new();
        let mut stack: Vec<usize> = self.roots.iter().rev().copied().collect();
        while let Some(ix) = stack.pop() {
            rows.push(ix);
            let node = &self.nodes[ix];
            if node.expanded
                && let Some(children) = &node.children
            {
                stack.extend(children.iter().rev());
            }
        }
        rows
    }

    /// Expands or collapses `ix`, asking for its children the first time.
    fn set_expanded(&mut self, ix: usize, expanded: bool, cx: &mut Context<Self>) {
        self.nodes[ix].expanded = expanded;
        if expanded && self.nodes[ix].children.is_none() && self.nodes[ix].loading.is_none() {
            self.load_children(ix, cx);
        }
        cx.notify();
    }

    fn load_children(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(provider) = self.provider.clone() else {
            return;
        };
        let item = self.nodes[ix].item.clone();
        let parent_uri = item.uri.clone();
        let direction = self.direction;
        let task: Task<anyhow::Result<Vec<Child>>> = match (&provider, direction) {
            (Provider::Call(provider), HierarchyDirection::IncomingCalls) => {
                let calls = provider.provide_incoming_calls(&item, cx);
                cx.background_executor().spawn(async move {
                    Ok(calls
                        .await?
                        .into_iter()
                        .map(|call| {
                            let uri = call.from.uri.clone();
                            (call.from, uri, call.from_ranges)
                        })
                        .collect())
                })
            }
            (Provider::Call(provider), HierarchyDirection::OutgoingCalls) => {
                let calls = provider.provide_outgoing_calls(&item, cx);
                cx.background_executor().spawn(async move {
                    Ok(calls
                        .await?
                        .into_iter()
                        .map(|call| (call.to, parent_uri.clone(), call.from_ranges))
                        .collect())
                })
            }
            (Provider::Type(provider), HierarchyDirection::Supertypes) => {
                let types = provider.provide_supertypes(&item, cx);
                Self::type_children(types, cx)
            }
            (Provider::Type(provider), HierarchyDirection::Subtypes) => {
                let types = provider.provide_subtypes(&item, cx);
                Self::type_children(types, cx)
            }
            _ => return,
        };
        let depth = self.nodes[ix].depth + 1;
        self.nodes[ix].loading = Some(cx.spawn(async move |this, cx| {
            let children = task.await.unwrap_or_else(|err| {
                log::warn!("hierarchy request failed: {err:#}");
                Vec::new()
            });
            this.update(cx, |this, cx| {
                // A direction change replaced the tree.
                if this.direction != direction || ix >= this.nodes.len() {
                    return;
                }
                let mut ids = Vec::new();
                for (item, preview_uri, ranges) in children {
                    let preview_ranges = if ranges.is_empty() {
                        vec![item.selection_range.clone()]
                    } else {
                        ranges
                    };
                    ids.push(this.nodes.len());
                    this.nodes.push(Node {
                        item,
                        depth,
                        children: None,
                        expanded: false,
                        loading: None,
                        preview_uri,
                        preview_ranges,
                    });
                }
                this.nodes[ix].children = Some(ids);
                this.nodes[ix].loading = None;
                cx.notify();
            })
            .ok();
        }));
    }

    fn type_children(
        types: Task<anyhow::Result<Vec<HierarchyItem>>>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Vec<Child>>> {
        cx.background_executor().spawn(async move {
            Ok(types
                .await?
                .into_iter()
                .map(|item| {
                    let uri = item.uri.clone();
                    let range = item.selection_range.clone();
                    (item, uri, vec![range])
                })
                .collect())
        })
    }

    /// The model at `uri` (`None`: the editor's).
    fn model_at(&self, uri: Option<&str>, cx: &mut gpui::App) -> Option<Entity<TextModel>> {
        if self.main_model.read(cx).is_at(uri) {
            return Some(self.main_model.clone());
        }
        model_for_uri(cx, uri?)
    }

    /// Shows the focused item in the preview, with its ranges highlighted.
    fn show_focused(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.preview.clone() else {
            return;
        };
        let Some(focused) = self.focused else {
            return;
        };
        let uri = self.nodes[focused].preview_uri.clone();
        let ranges = self.nodes[focused].preview_ranges.clone();
        let model = self.model_at(uri.as_deref(), cx);
        if let Some(mut highlights) = self.highlights.take() {
            highlights.clear(cx);
        }
        let shown = model.clone().unwrap_or_else(|| self.no_preview.clone());
        preview.update(cx, |preview, cx| {
            let scroll_type = if preview.model() == &shown {
                ScrollType::Smooth
            } else {
                preview.set_model(shown, cx);
                ScrollType::Immediate
            };
            if model.is_some()
                && let Some(first) = ranges.first()
            {
                let start = first.start;
                preview.set_selections(&[crate::cursor::Selection::collapsed(start)], cx);
                preview.reveal_range_with_scroll_type(
                    start..start,
                    RevealType::Center,
                    scroll_type,
                    cx,
                );
            }
        });
        if let Some(model) = model {
            let highlight = DecorationOptions {
                background: Some(self.colors.peek_view_editor_match_highlight_background),
                border: self.colors.peek_view_editor_match_highlight_border,
                ..Default::default()
            };
            let mut highlights = DecorationsCollection::for_owner(model, preview.entity_id());
            highlights.set(
                ranges
                    .into_iter()
                    .map(|range| (range, highlight.clone()))
                    .collect(),
                cx,
            );
            self.highlights = Some(highlights);
        }
        cx.notify();
    }

    fn ensure_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview.is_some() || self.nodes.is_empty() {
            return;
        }
        let theme = self.preview_theme.clone();
        let options = self.options.clone();
        let model = self.no_preview.clone();
        let preview = cx.new(|cx| {
            let mut editor = CodeEditor::new(model, window, cx);
            editor.in_peek = true;
            editor.update_options(options, cx);
            editor.set_theme(theme, cx);
            editor
        });
        self.preview = Some(preview);
        self.show_focused(cx);
    }

    fn focus_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.focused = Some(ix);
        let rows = self.rows();
        if let Some(row) = rows.iter().position(|r| *r == ix) {
            let top = ROW_HEIGHT * row as f32;
            let height = self.list_height();
            if top < self.scroll_top {
                self.scroll_top = top;
            } else if top + ROW_HEIGHT > self.scroll_top + height {
                self.scroll_top = top + ROW_HEIGHT - height;
            }
        }
        self.show_focused(cx);
    }

    fn move_in_list(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let current = self
            .focused
            .and_then(|f| rows.iter().position(|r| *r == f))
            .unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.focus_row(rows[next], cx);
    }

    fn focus_edge(&mut self, last: bool, cx: &mut Context<Self>) {
        let rows = self.rows();
        let row = if last { rows.last() } else { rows.first() };
        if let Some(&row) = row {
            self.focus_row(row, cx);
        }
    }

    /// Left: collapses the focused item, or focuses its parent.
    fn collapse(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.focused else {
            return;
        };
        let node = &self.nodes[ix];
        let leaf = node.children.as_ref().is_some_and(|c| c.is_empty());
        if node.expanded && !leaf {
            self.set_expanded(ix, false, cx);
        } else if let Some(parent) = self.parent(ix) {
            self.focus_row(parent, cx);
        }
    }

    /// Right: expands the focused item, or focuses its first child.
    fn expand(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.focused else {
            return;
        };
        if !self.nodes[ix].expanded {
            self.set_expanded(ix, true, cx);
        } else if let Some(&first) = self.nodes[ix].children.as_ref().and_then(|c| c.first()) {
            self.focus_row(first, cx);
        }
    }

    fn toggle_focused(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.focused {
            let expanded = self.nodes[ix].expanded;
            self.set_expanded(ix, !expanded, cx);
        }
    }

    fn parent(&self, ix: usize) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.children.as_ref().is_some_and(|c| c.contains(&ix)))
    }

    /// Enter or a double click: closes the view and goes to the focused
    /// item (VS Code's tree `onDidOpen`).
    fn open_focused(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.focused else {
            return;
        };
        let item = &self.nodes[ix].item;
        let location = Location {
            uri: item.uri.clone(),
            range: item.range.clone(),
            target_selection_range: Some(item.selection_range.clone()),
            ..Default::default()
        };
        if let Some(editor) = self.editor.upgrade() {
            cx.defer(move |cx| {
                editor.update(cx, |editor, cx| {
                    editor.close_hierarchy(cx);
                    editor.open_location(&location, true, cx);
                });
            });
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            cx.defer(move |cx| {
                editor.update(cx, |editor, cx| {
                    editor.close_hierarchy(cx);
                });
            });
        }
    }

    fn list_height(&self) -> Pixels {
        (self.line_height * self.height_in_lines - HEAD_HEIGHT).max(ROW_HEIGHT)
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(ROW_HEIGHT).y;
        let max = (ROW_HEIGHT * self.rows().len() as f32 - self.list_height()).max(px(0.));
        self.scroll_top = (self.scroll_top - delta).clamp(px(0.), max);
        cx.stop_propagation();
        cx.notify();
    }

    /// The title: the root's name, and its detail.
    fn title(&self) -> (String, String) {
        match self.roots.first() {
            Some(&root) => {
                let item = &self.nodes[root].item;
                (
                    item.name.to_string(),
                    item.detail
                        .as_ref()
                        .map(|d| d.to_string())
                        .unwrap_or_default(),
                )
            }
            None => (String::new(), String::new()),
        }
    }

    /// The message instead of the tree: "Loading...", nothing to show, or
    /// the root has nothing.
    fn message(&self) -> Option<String> {
        if self.loading {
            return Some("Loading...".into());
        }
        let Some(&root) = self.roots.first() else {
            return Some("No results".into());
        };
        let node = &self.nodes[root];
        (node.children.as_ref().is_some_and(|c| c.is_empty()))
            .then(|| self.direction.empty_message(&node.item.name))
    }

    fn render_row(
        &self,
        ix: usize,
        list_focused: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = &self.colors;
        let node = &self.nodes[ix];
        let focused = self.focused == Some(ix);
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
        let (icon, icon_color) = crate::quick_access::symbol_icon(node.item.kind, colors);
        let leaf = node.children.as_ref().is_some_and(|c| c.is_empty());
        let twistie = if leaf {
            None
        } else if node.expanded {
            Some(Codicon::TreeItemExpanded)
        } else {
            Some(Codicon::ChevronRight)
        };
        let label = match &node.item.detail {
            Some(detail) => format!("{} {detail}", node.item.name),
            None => node.item.name.to_string(),
        };
        AriaNode::new(Role::TreeItem, label)
            .level(node.depth + 1)
            .selected(focused)
            .active_descendant(focused)
            .expanded(node.expanded)
            .apply(div().id(("hierarchy-row", ix)))
            .h(ROW_HEIGHT)
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .pl(INDENT * node.depth as f32)
            .text_color(
                selection_foreground
                    .filter(|_| focused)
                    .unwrap_or(colors.peek_view_result_line_foreground),
            )
            .when(focused, |row| {
                row.when_some(selection_background, |row, bg| row.bg(bg))
            })
            .child(
                div()
                    .id(("hierarchy-twistie", ix))
                    .w(TWISTIE_WIDTH)
                    .flex_none()
                    .flex()
                    .justify_end()
                    .pr(px(6.))
                    .children(twistie.map(codicon))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        window.focus(&this.focus, cx);
                        let expanded = this.nodes[ix].expanded;
                        this.set_expanded(ix, !expanded, cx);
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .mr(px(6.))
                    .text_color(icon_color)
                    .child(codicon(icon)),
            )
            .child(
                div()
                    .flex_none()
                    .when(node.item.deprecated, |name| name.line_through())
                    .child(node.item.name.clone()),
            )
            .when_some(node.item.detail.clone(), |row, detail| {
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .ml(px(6.5))
                        .text_size(px(11.7))
                        .opacity(0.7)
                        .text_ellipsis()
                        .child(detail),
                )
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                window.focus(&this.focus, cx);
                this.focus_row(ix, cx);
                if event.click_count() == 2 {
                    this.open_focused(cx);
                }
            }))
            .into_any_element()
    }
}

impl Focusable for HierarchyPeek {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for HierarchyPeek {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_preview(window, cx);
        if std::mem::take(&mut self.focus_pending) {
            window.focus(&self.focus, cx);
        }
        if let Some(editor) = self.editor.upgrade() {
            self.line_height = editor.read(cx).line_height();
        }
        let colors = self.colors.clone();
        let border = colors.peek_view_border;
        let body_height = self.list_height();
        let (title, description) = self.title();
        let meta = (!self.roots.is_empty()).then(|| self.direction.label());
        let message = self.message();
        let list_focused = self.focus.is_focused(window);
        let rows = self.rows();
        let max_scroll = (ROW_HEIGHT * rows.len() as f32 - body_height).max(px(0.));
        self.scroll_top = self.scroll_top.clamp(px(0.), max_scroll);
        let visible: Vec<_> = rows
            .iter()
            .map(|&ix| self.render_row(ix, list_focused, cx))
            .collect();
        let toggle_label = match self.direction.toggled() {
            HierarchyDirection::IncomingCalls => "Show Incoming Calls",
            HierarchyDirection::OutgoingCalls => "Show Outgoing Calls",
            HierarchyDirection::Supertypes => "Show Supertypes",
            HierarchyDirection::Subtypes => "Show Subtypes",
        };
        let toggle_icon = match self.direction.toggled() {
            HierarchyDirection::IncomingCalls => Codicon::CallIncoming,
            HierarchyDirection::OutgoingCalls => Codicon::CallOutgoing,
            HierarchyDirection::Supertypes => Codicon::TypeHierarchySuper,
            HierarchyDirection::Subtypes => Codicon::TypeHierarchySub,
        };
        let button = |id: &'static str, label: &'static str, icon: Codicon| {
            AriaNode::new(Role::Button, label)
                .apply(div().id(id))
                .debug_selector(move || id.into())
                .flex_none()
                .p(px(3.))
                .mr(px(2.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_color(colors.peek_view_title_foreground)
                .hover(|style| colors.toolbar_hover(style))
                .tooltip(crate::tooltip::tooltip(label, &colors))
                .child(codicon(icon))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let toggle = button("hierarchy-toggle", toggle_label, toggle_icon)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_direction(cx)));
        let close = button("hierarchy-close", "Close", Codicon::Close)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)));
        let head = div()
            .h(HEAD_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .bg(colors.peek_view_title_background)
            .child(
                div()
                    .debug_selector(|| "hierarchy-title".into())
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
                            .flex_none()
                            .text_color(colors.peek_view_title_foreground)
                            .child(title),
                    )
                    .when(!description.is_empty(), |title| {
                        title.child(
                            div()
                                .ml(px(6.5))
                                .text_size(px(11.7))
                                .min_w_0()
                                .text_ellipsis()
                                .text_color(colors.peek_view_title_description_foreground)
                                .child(description),
                        )
                    })
                    .when_some(meta, |title, meta| {
                        title.child(
                            div()
                                .flex_none()
                                .flex()
                                .text_color(colors.peek_view_title_description_foreground)
                                .child(div().px(px(3.9)).child("-"))
                                .child(meta),
                        )
                    }),
            )
            .when(!self.roots.is_empty(), |head| head.child(toggle))
            .child(close);
        let key_context = crate::context_keys::widget_key_context(
            PEEK_KEY_CONTEXT,
            &crate::context_keys::editor_keys_for_widgets(&self.editor, cx),
        );
        let message_context = key_context.clone();
        let list = AriaNode::new(Role::Tree, self.direction.label())
            .apply(div().id("hierarchy-tree"))
            .debug_selector(|| "hierarchy-tree".into())
            .key_context(key_context)
            .track_focus(&self.focus)
            .relative()
            .h_full()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .bg(colors.peek_view_result_background)
            .text_size(px(13.))
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll_wheel(event, cx)))
            .on_action(cx.listener(|this, _: &PeekSelectNext, _, cx| this.move_in_list(1, cx)))
            .on_action(cx.listener(|this, _: &PeekSelectPrevious, _, cx| this.move_in_list(-1, cx)))
            .on_action(cx.listener(|this, _: &PeekSelectFirst, _, cx| this.focus_edge(false, cx)))
            .on_action(cx.listener(|this, _: &PeekSelectLast, _, cx| this.focus_edge(true, cx)))
            .on_action(cx.listener(|this, _: &PeekOpen, _, cx| this.open_focused(cx)))
            .on_action(cx.listener(|this, _: &PeekToggle, _, cx| this.toggle_focused(cx)))
            .on_action(cx.listener(|this, _: &PeekCollapse, _, cx| this.collapse(cx)))
            .on_action(cx.listener(|this, _: &PeekExpand, _, cx| this.expand(cx)))
            .on_action(cx.listener(|this, _: &ClosePeek, _, cx| this.close(cx)))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(-self.scroll_top)
                    .children(visible),
            );
        let preview = self.preview.clone();
        div().size_full().child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .border_t_1()
                .border_b_1()
                .border_color(border)
                .child(head)
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .border_t_1()
                        .border_color(border)
                        .flex()
                        .map(|body| match message {
                            Some(message) if self.roots.is_empty() || self.loading => body.child(
                                // Holds the focus while there's no tree, so
                                // Escape closes the view.
                                div()
                                    .debug_selector(|| "hierarchy-message".into())
                                    .key_context(message_context)
                                    .track_focus(&self.focus)
                                    .on_action(
                                        cx.listener(|this, _: &ClosePeek, _, cx| this.close(cx)),
                                    )
                                    .size_full()
                                    .pt(px(39.))
                                    .flex()
                                    .justify_center()
                                    .text_size(px(13.))
                                    .text_color(colors.foreground)
                                    .child(message),
                            ),
                            message => body
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(PREVIEW_RATIO))
                                        .overflow_hidden()
                                        .children(preview),
                                )
                                .child(
                                    div()
                                        .h_full()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .border_l_1()
                                        .border_color(border)
                                        .child(list)
                                        .children(message.map(|message| {
                                            div()
                                                .debug_selector(|| "hierarchy-message".into())
                                                .flex_none()
                                                .p(px(6.))
                                                .text_size(px(12.))
                                                .bg(colors.peek_view_result_background)
                                                .text_color(colors.peek_view_result_line_foreground)
                                                .child(message)
                                        })),
                                ),
                        }),
                ),
        )
    }
}

impl CodeEditor {
    /// Peek Call Hierarchy (VS Code `editor.showCallHierarchy`): the calls
    /// to the function at the cursor, in a peek view. Run again with the
    /// call hierarchy open, it shows the other direction.
    pub fn show_call_hierarchy(&mut self, cx: &mut Context<Self>) {
        self.show_hierarchy(HierarchyDirection::IncomingCalls, cx);
    }

    /// Peek Type Hierarchy (VS Code `editor.showTypeHierarchy`): the
    /// supertypes of the type at the cursor, in a peek view. Run again with
    /// the type hierarchy open, it shows the other direction.
    pub fn show_type_hierarchy(&mut self, cx: &mut Context<Self>) {
        self.show_hierarchy(HierarchyDirection::Supertypes, cx);
    }

    /// Opens the hierarchy peek view at the cursor in `direction`, or, when
    /// one of the same hierarchy is open, toggles its direction.
    pub fn show_hierarchy(&mut self, direction: HierarchyDirection, cx: &mut Context<Self>) {
        if self.in_peek || self.options.single_line {
            return;
        }
        if let Some(view) = self.hierarchy.view.clone() {
            let open = view.read(cx).direction;
            if open.is_call() == direction.is_call() {
                view.update(cx, |view, cx| view.toggle_direction(cx));
                return;
            }
        }
        self.close_hierarchy(cx);
        let position = self.cursors.primary().position;
        let anchor = self.cursors.primary().selection().start();
        let context = self.model().read(cx).provider_context();
        let target = self.provider_target(cx);
        let prepare: Vec<(Provider, Task<anyhow::Result<Vec<HierarchyItem>>>)> =
            if direction.is_call() {
                call_hierarchy_providers(cx, target)
                    .into_iter()
                    .map(|provider| {
                        let task = provider.prepare_call_hierarchy(&context, position, cx);
                        (Provider::Call(provider), task)
                    })
                    .collect()
            } else {
                type_hierarchy_providers(cx, target)
                    .into_iter()
                    .map(|provider| {
                        let task = provider.prepare_type_hierarchy(&context, position, cx);
                        (Provider::Type(provider), task)
                    })
                    .collect()
            };
        if prepare.is_empty() {
            return;
        }
        let editor = cx.entity();
        let model = self.model().clone();
        let theme = self.theme().clone();
        let options = self.options().clone();
        let line_height = self.line_height();
        let view = cx.new(|cx| {
            let mut view = HierarchyPeek::new(&editor, model, theme, options, direction, cx);
            view.line_height = line_height;
            view
        });
        let max_lines = self.layout.as_ref().map_or(HEIGHT_IN_LINES, |layout| {
            (layout.text_bounds.size.height / line_height * 0.8).max(12.)
        });
        let height = HEIGHT_IN_LINES.min(max_lines);
        view.update(cx, |view, _| view.height_in_lines = height);
        let zone = crate::widgets::ViewZone::new(
            anchor.row + 1,
            ZoneHeight::Lines(height),
            view.clone().into(),
        );
        let id = self.add_zone(zone, true, cx);
        self.widgets.set_zone_captures_wheel(id);
        self.hierarchy.zone = Some(id);
        self.hierarchy.view = Some(view.clone());
        if anchor.row + 1 >= self.model().read(cx).line_count() {
            self.reveal_line_near_top(anchor.row, cx);
        } else {
            let start = PointUtf16::new(anchor.row, 0);
            let end = PointUtf16::new(anchor.row + 1, 0);
            self.reveal_range(start..end, RevealType::Simple, cx);
        }
        self.hierarchy.task = Some(cx.spawn(async move |this, cx| {
            // VS Code takes the first item of the first provider with any.
            let mut root = None;
            for (provider, task) in prepare {
                match task.await {
                    Ok(items) => {
                        if let Some(item) = items.into_iter().next() {
                            root = Some((provider, item));
                            break;
                        }
                    }
                    Err(err) => log::warn!("preparing a hierarchy failed: {err:#}"),
                }
            }
            this.update(cx, |this, cx| {
                if this.hierarchy.view.as_ref() != Some(&view) {
                    return;
                }
                this.hierarchy.task = None;
                view.update(cx, |view, cx| view.set_root(root, cx));
            })
            .ok();
        }));
    }

    /// Closes the hierarchy peek view; returns whether one was open.
    pub fn close_hierarchy(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(zone) = self.hierarchy.zone.take() else {
            return false;
        };
        self.hierarchy.view = None;
        self.hierarchy.task = None;
        self.remove_widget(zone, cx);
        if !self.in_peek {
            self.refocus = true;
        }
        true
    }

    /// Whether the hierarchy peek view is open, and which way.
    pub fn hierarchy_direction(&self, cx: &gpui::App) -> Option<HierarchyDirection> {
        Some(self.hierarchy.view.as_ref()?.read(cx).direction)
    }

    /// The hierarchy view's rows, indented two spaces a level, the focused
    /// one marked with `>`, then its message (for tests).
    #[doc(hidden)]
    pub fn hierarchy_rows_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        let Some(view) = self.hierarchy.view.as_ref() else {
            return Vec::new();
        };
        let view = view.read(cx);
        let mut rows: Vec<String> = view
            .rows()
            .into_iter()
            .map(|ix| {
                let node = &view.nodes[ix];
                let marker = if view.focused == Some(ix) { "> " } else { "" };
                format!("{}{marker}{}", "  ".repeat(node.depth), node.item.name)
            })
            .collect();
        if let Some(message) = view.message() {
            rows.push(format!("[{message}]"));
        }
        rows
    }

    /// The hierarchy view's preview editor (for tests).
    #[doc(hidden)]
    pub fn hierarchy_preview_for_tests(&self, cx: &gpui::App) -> Option<Entity<CodeEditor>> {
        self.hierarchy.view.as_ref()?.read(cx).preview.clone()
    }
}
