//! The multi-file diff editor (Monaco `MultiDiffEditorWidget`, MIT): one
//! scrolling list of diff editors, each under a collapsible file header.
//!
//! Like Monaco, each diff editor is as tall as its content (with unchanged
//! regions hidden), the list scrolls them as one, and only the visible part
//! of each is laid out: the list sizes the editor to what shows and scrolls
//! it to match.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, ScrollWheelEvent,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px,
};

use crate::diff_editor::{DiffEditor, DiffEditorEvent, DiffEditorOptions};
use crate::model::TextModel;
use crate::options::ScrollbarVisibility;

/// A file header's height (Monaco's `headerHeight`).
const HEADER_HEIGHT: f32 = 40.;
/// Space left and right of each diff editor (Monaco's `horizontalInsets`).
const HORIZONTAL_INSET: f32 = 9.;
/// The height assumed for a diff editor before it's laid out (Monaco's
/// initial `editorContentHeight`).
const INITIAL_CONTENT_HEIGHT: f32 = 500.;

/// A file in a multi-diff editor (Monaco `IDocumentDiffItem`).
#[derive(Clone)]
pub struct MultiDiffDocument {
    /// The header's title, usually the modified file's path.
    pub label: SharedString,
    /// The original file's path when it differs (a rename).
    pub original_label: Option<SharedString>,
    /// The original text; `None` if the file was created.
    pub original: Option<Entity<TextModel>>,
    /// The modified text; `None` if the file was deleted.
    pub modified: Option<Entity<TextModel>>,
}

impl MultiDiffDocument {
    /// A modified file.
    pub fn new(
        label: impl Into<SharedString>,
        original: Entity<TextModel>,
        modified: Entity<TextModel>,
    ) -> Self {
        Self {
            label: label.into(),
            original_label: None,
            original: Some(original),
            modified: Some(modified),
        }
    }
}

/// Events a [`MultiDiffEditor`] emits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MultiDiffEditorEvent {
    /// A file was collapsed or expanded.
    CollapsedChanged {
        /// The file's index.
        index: usize,
        /// Whether it's collapsed now.
        collapsed: bool,
    },
}

/// What happened to a file, shown in its header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
}

struct Item {
    label: SharedString,
    original_label: Option<SharedString>,
    status: FileStatus,
    diff: Entity<DiffEditor>,
    collapsed: bool,
    /// The diff editor's content height, as last measured.
    content_height: Pixels,
}

impl Item {
    /// Monaco `DiffEditorItemTemplate.size`.
    fn height(&self) -> Pixels {
        if self.collapsed {
            px(HEADER_HEIGHT)
        } else {
            px(HEADER_HEIGHT) + self.content_height
        }
    }
}

/// Monaco `MultiDiffEditorWidget`: diffs of many files in one list.
pub struct MultiDiffEditor {
    items: Vec<Item>,
    options: DiffEditorOptions,
    scroll_top: Pixels,
    scroll_left: Pixels,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    focus_handle: FocusHandle,
}

impl EventEmitter<MultiDiffEditorEvent> for MultiDiffEditor {}

impl Focusable for MultiDiffEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl MultiDiffEditor {
    /// A multi-diff editor showing `documents`.
    pub fn new(
        documents: Vec<MultiDiffDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            items: Vec::new(),
            options: DiffEditorOptions::default(),
            scroll_top: px(0.),
            scroll_left: px(0.),
            bounds: Default::default(),
            focus_handle: cx.focus_handle(),
        };
        this.set_documents(documents, window, cx);
        this
    }

    /// Replaces the files (Monaco `setViewModel`), scrolled to the top.
    pub fn set_documents(
        &mut self,
        documents: Vec<MultiDiffDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.items = documents
            .into_iter()
            .map(|document| self.create_item(document, window, cx))
            .collect();
        self.scroll_top = px(0.);
        self.scroll_left = px(0.);
        cx.notify();
    }

    fn create_item(
        &self,
        document: MultiDiffDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Item {
        let status = match (&document.original, &document.modified) {
            (None, _) => FileStatus::Added,
            (_, None) => FileStatus::Deleted,
            _ if document.original_label.is_some() => FileStatus::Renamed,
            _ => FileStatus::Modified,
        };
        let original = document
            .original
            .unwrap_or_else(|| cx.new(|_| TextModel::new("")));
        let modified = document
            .modified
            .unwrap_or_else(|| cx.new(|_| TextModel::new("")));
        let options = self.item_options();
        let diff = cx.new(|cx| {
            let mut diff = DiffEditor::new(original, modified, window, cx);
            diff.update_options(options, cx);
            // Monaco's item template: the list scrolls, not the editors.
            for editor in [
                diff.original_editor().clone(),
                diff.modified_editor().clone(),
            ] {
                editor.update(cx, |editor, cx| {
                    let mut options = editor.options().clone();
                    options.scroll_beyond_last_line = false;
                    options.scrollbar.vertical = ScrollbarVisibility::Hidden;
                    options.scrollbar.horizontal = ScrollbarVisibility::Hidden;
                    // Nothing shows there, and the item is sized to its
                    // content.
                    options.horizontal_scrollbar_size = px(0.);
                    options.scrollbar.handle_mouse_wheel = false;
                    options.minimap.enabled = false;
                    if status == FileStatus::Deleted {
                        options.read_only = true;
                    }
                    editor.update_options(options, cx);
                });
            }
            diff
        });
        cx.subscribe(&diff, |_, _, _: &DiffEditorEvent, cx| cx.notify())
            .detach();
        Item {
            label: document.label,
            original_label: document.original_label,
            status,
            diff,
            collapsed: false,
            content_height: px(INITIAL_CONTENT_HEIGHT),
        }
    }

    /// Monaco's item options: unchanged regions hidden, no overview ruler.
    fn item_options(&self) -> DiffEditorOptions {
        let mut options = self.options.clone();
        options.hide_unchanged_regions.enabled = true;
        options.render_overview_ruler = false;
        options
    }

    /// The options every file's diff editor uses (unchanged regions are
    /// always hidden).
    pub fn options(&self) -> &DiffEditorOptions {
        &self.options
    }

    /// Replaces the options of every file's diff editor.
    pub fn update_options(&mut self, options: DiffEditorOptions, cx: &mut Context<Self>) {
        self.options = options;
        let options = self.item_options();
        for item in &self.items {
            item.diff
                .update(cx, |diff, cx| diff.update_options(options.clone(), cx));
        }
        cx.notify();
    }

    /// The file count.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no files.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The diff editor of file `index`.
    pub fn diff_editor(&self, index: usize) -> Option<&Entity<DiffEditor>> {
        self.items.get(index).map(|item| &item.diff)
    }

    /// Whether file `index` is collapsed to its header.
    pub fn is_collapsed(&self, index: usize) -> bool {
        self.items.get(index).is_some_and(|item| item.collapsed)
    }

    /// Collapses or expands file `index` (Monaco's header chevron).
    pub fn set_collapsed(&mut self, index: usize, collapsed: bool, cx: &mut Context<Self>) {
        if let Some(item) = self.items.get_mut(index)
            && item.collapsed != collapsed
        {
            item.collapsed = collapsed;
            cx.emit(MultiDiffEditorEvent::CollapsedChanged { index, collapsed });
            cx.notify();
        }
    }

    /// Collapses or expands every file (Monaco `collapseAll`/`expandAll`).
    pub fn set_all_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        for index in 0..self.items.len() {
            self.set_collapsed(index, collapsed, cx);
        }
    }

    /// The vertical scroll position.
    pub fn scroll_top(&self) -> Pixels {
        self.scroll_top
    }

    /// Scrolls the list (clamped when next drawn).
    pub fn set_scroll_top(&mut self, scroll_top: Pixels, cx: &mut Context<Self>) {
        self.scroll_top = scroll_top.max(px(0.));
        cx.notify();
    }

    /// Scrolls so file `index`'s header is at the top (Monaco `reveal`).
    pub fn reveal(&mut self, index: usize, cx: &mut Context<Self>) {
        let top = self.items[..index.min(self.items.len())]
            .iter()
            .map(Item::height)
            .fold(px(0.), |a, b| a + b);
        self.set_scroll_top(top, cx);
    }

    fn total_height(&self) -> Pixels {
        self.items
            .iter()
            .map(Item::height)
            .fold(px(0.), |a, b| a + b)
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(18.));
        let viewport = self.bounds.get().size.height;
        let max_top = (self.total_height() - viewport).max(px(0.));
        // Monaco shares one horizontal scroll among the editors.
        let max_left = self
            .items
            .iter()
            .map(|item| item.diff.read(cx).max_scroll_left(cx))
            .fold(px(0.), |a, b| a.max(b));
        self.scroll_top = (self.scroll_top - delta.y).clamp(px(0.), max_top);
        self.scroll_left = (self.scroll_left - delta.x).clamp(px(0.), max_left);
        cx.stop_propagation();
        cx.notify();
    }

    /// Takes each laid-out diff editor's content height, redrawing when one
    /// changed (Monaco `_flushVerticalState`).
    fn measure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        for item in &mut self.items {
            if let Some(height) = item.diff.read(cx).content_height(cx)
                && (height - item.content_height).abs() > px(0.5)
            {
                item.content_height = height;
                changed = true;
            }
        }
        if changed {
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
    }

    /// File `index`'s top and height, for tests.
    #[doc(hidden)]
    pub fn item_bounds_for_tests(&self, index: usize) -> (Pixels, Pixels) {
        let top = self.items[..index]
            .iter()
            .map(Item::height)
            .fold(px(0.), |a, b| a + b);
        (top, self.items[index].height())
    }
}

impl Render for MultiDiffEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.measure(window, cx);
        let bounds = self.bounds.clone();
        let viewport = bounds.get().size.height;
        let max_top = (self.total_height() - viewport).max(px(0.));
        if viewport > px(0.) {
            self.scroll_top = self.scroll_top.clamp(px(0.), max_top);
        }
        let scroll_top = self.scroll_top;
        let scroll_left = self.scroll_left;
        let Some(first) = self.items.first() else {
            return div()
                .size_full()
                .track_focus(&self.focus_handle)
                .into_any_element();
        };
        let colors = first
            .diff
            .read(cx)
            .modified_editor()
            .read(cx)
            .colors()
            .clone();
        let mut children = Vec::new();
        let mut top = px(0.);
        for (index, item) in self.items.iter().enumerate() {
            let height = item.height();
            let item_top = top;
            top += height;
            // Monaco renders only the items in the viewport.
            let visible_start = item_top.max(scroll_top);
            let visible_end = (item_top + height).min(scroll_top + viewport.max(px(1.)));
            if visible_end <= visible_start {
                continue;
            }
            // The visible part: a sticky header, then the editor scrolled to
            // the part under it.
            let editor_scroll = visible_start - item_top;
            let editor_height = (visible_end - visible_start - px(HEADER_HEIGHT)).max(px(0.));
            if !item.collapsed {
                item.diff.update(cx, |diff, cx| {
                    diff.set_scroll_position_unclamped(point(scroll_left, editor_scroll), cx)
                });
            }
            let chevron = if item.collapsed { "›" } else { "⌄" };
            let status = match item.status {
                FileStatus::Modified => None,
                FileStatus::Added => Some("A"),
                FileStatus::Deleted => Some("D"),
                FileStatus::Renamed => Some("R"),
            };
            let collapsed = item.collapsed;
            let header = div()
                .id(("multi-diff-header", index))
                .h(px(HEADER_HEIGHT))
                .pt(px(8.))
                .flex_none()
                .bg(colors.background)
                .child(
                    div()
                        .size_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(6.))
                        .border_t_1()
                        .border_color(colors.multi_diff_border)
                        .bg(colors.multi_diff_header_background)
                        .text_color(colors.foreground)
                        .text_size(px(14.))
                        .child(
                            div()
                                .id(("multi-diff-collapse", index))
                                .role(gpui::Role::Button)
                                .aria_label(SharedString::from(format!(
                                    "{}, {}",
                                    item.label,
                                    if collapsed { "collapsed" } else { "expanded" }
                                )))
                                .w(px(16.))
                                .flex()
                                .justify_center()
                                .cursor_pointer()
                                .child(chevron)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_collapsed(index, !collapsed, cx)
                                })),
                        )
                        .child(div().whitespace_nowrap().child(item.label.clone()))
                        .when_some(status, |header, status| {
                            header.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(colors.foreground.opacity(0.7))
                                    .child(status),
                            )
                        })
                        .when_some(item.original_label.clone(), |header, original| {
                            header.child(
                                div()
                                    .whitespace_nowrap()
                                    .text_color(colors.foreground.opacity(0.7))
                                    .child(original),
                            )
                        }),
                );
            children.push(
                div()
                    .absolute()
                    .top(visible_start - scroll_top)
                    .left(px(0.))
                    .right(px(0.))
                    .h(visible_end - visible_start)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(header)
                    .when(!collapsed && editor_height > px(0.), |item_div| {
                        item_div.child(
                            div()
                                .h(editor_height)
                                .flex_none()
                                .mx(px(HORIZONTAL_INSET))
                                .child(item.diff.clone()),
                        )
                    }),
            );
        }
        div()
            .id("multi-diff-editor")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(colors.background)
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(children)
            .into_any_element()
    }
}
