//! The multi-file diff editor (Monaco `MultiDiffEditorWidget`, MIT): one
//! scrolling list of diff editors, each under a collapsible file header.
//!
//! Like Monaco, each diff editor is as tall as its content (with unchanged
//! regions hidden), the list scrolls them as one, and only the visible part
//! of each is laid out: the list sizes the editor to what shows and scrolls
//! it to match.
//!
//! Monaco's `VirtualizedItemManager` binds diff editors only to the files in
//! view: a file scrolled out gives its editor back to a pool (Monaco
//! `VirtualizedTemplatePool`, which keeps up to five unused ones), keeping
//! its view state and measured height for when it shows again. The list
//! has Monaco's `SmoothScrollableElement` scrollbars, and the headers follow
//! the `multiDiffEditor-standard` variant of Monaco's `style.css`.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Bounds, BoxShadow, Context, DispatchPhase, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    WeakEntity, Window, canvas, div, point, px, size,
};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::diff_editor::{DiffEditor, DiffEditorEvent, DiffEditorOptions, DiffEditorViewState};
use crate::model::TextModel;
use crate::options::ScrollbarVisibility;
use crate::theme::EditorColors;
use crate::view::animation::ScrollbarFade;

/// A file header's height (Monaco's `headerHeight`).
const HEADER_HEIGHT: f32 = 40.;
/// Space above a header's content (Monaco's `.header-content` margin).
const HEADER_MARGIN_TOP: f32 = 8.;
/// Space left and right of each diff editor (Monaco's `horizontalInsets`).
const HORIZONTAL_INSET: f32 = 9.;
/// The height assumed for a diff editor before it's laid out (Monaco's
/// initial `editorContentHeight`).
const INITIAL_CONTENT_HEIGHT: f32 = 500.;
/// The list's scrollbars' size (Monaco `ScrollableElement`'s default
/// `verticalScrollbarSize` and `horizontalScrollbarSize`).
const SCROLLBAR_SIZE: f32 = 10.;
/// Monaco `ScrollbarState`'s `MINIMUM_SLIDER_SIZE`.
const MIN_SLIDER: f32 = 20.;
/// How many unused diff editors the pool keeps (Monaco
/// `VirtualizedTemplatePool`).
const MAX_UNUSED_EDITORS: usize = 5;

/// A file in a multi-diff editor (Monaco `IDocumentDiffItem`).
#[derive(Clone)]
pub struct MultiDiffDocument {
    /// The header's title, usually the modified file's path. The part
    /// after the last `/` shows as the name, the rest as its description.
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

/// What a [`MultiDiffFileAction`] runs: it gets the file's index and
/// document.
pub type MultiDiffFileHandler = Rc<dyn Fn(usize, &MultiDiffDocument, &mut Window, &mut App)>;

/// A button in each file header's toolbar (a Monaco
/// `MultiDiffEditorFileToolbar` menu item), from
/// [`MultiDiffEditor::add_file_action`].
#[derive(Clone)]
pub struct MultiDiffFileAction {
    /// Its id, for [`MultiDiffEditor::remove_file_action`].
    pub id: SharedString,
    /// Its tooltip and screen reader label.
    pub label: SharedString,
    /// The button's text (Monaco shows a codicon).
    pub icon: SharedString,
    /// Runs it.
    pub run: MultiDiffFileHandler,
}

impl std::fmt::Debug for MultiDiffFileAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiDiffFileAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// What happened to a file, shown in its header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
}

/// A file (Monaco `DocumentDiffItemViewModel`), with what Monaco keeps in
/// its `lastTemplateData` while no editor shows it.
struct Item {
    document: MultiDiffDocument,
    status: FileStatus,
    /// The models shown, with empty placeholders for a missing side.
    original: Entity<TextModel>,
    modified: Entity<TextModel>,
    collapsed: bool,
    /// The diff editor's content height, as last measured (Monaco
    /// `expandedContentHeight`, less the header).
    content_height: Pixels,
    /// The view state its editor had when it was given back to the pool
    /// (Monaco keeps the selections).
    view_state: Option<DiffEditorViewState>,
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

/// A diff editor bound to a file or waiting in the pool (Monaco
/// `DiffEditorItemTemplate`).
struct Template {
    diff: Entity<DiffEditor>,
    /// The file shown, while bound.
    item: usize,
    /// The diff changed since the editors were last laid out, so their
    /// content height is not the new one yet.
    stale: bool,
    _subscription: Subscription,
}

impl Template {
    /// Monaco `DiffEditorItemTemplate.isFocused`: a bound template stays
    /// bound while focused (`shouldKeepAlive`).
    fn is_focused(&self, window: &Window, cx: &App) -> bool {
        let diff = self.diff.read(cx);
        [diff.original_editor(), diff.modified_editor()]
            .iter()
            .any(|editor| editor.focus_handle(cx).contains_focused(window, cx))
    }
}

/// A scrollbar's slider (Monaco `ScrollbarState`), along the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollbarState {
    /// The bar, in window coordinates.
    bar: Bounds<Pixels>,
    vertical: bool,
    /// The slider's offset from the bar's start, and its length.
    slider_position: Pixels,
    slider_size: Pixels,
    /// Slider pixels per scrolled pixel.
    ratio: f32,
}

impl ScrollbarState {
    /// Monaco `ScrollbarState._computeValues`: `None` when everything
    /// fits (`isNeeded`).
    fn new(
        bar: Bounds<Pixels>,
        vertical: bool,
        visible: Pixels,
        scroll_size: Pixels,
        scroll_position: Pixels,
    ) -> Option<Self> {
        let available = if vertical {
            bar.size.height
        } else {
            bar.size.width
        };
        if scroll_size <= visible || available <= px(0.) {
            return None;
        }
        let slider_size = (visible * (available / scroll_size))
            .floor()
            .max(px(MIN_SLIDER))
            .round()
            .min(available);
        let ratio = (available - slider_size) / (scroll_size - visible);
        Some(Self {
            bar,
            vertical,
            slider_position: scroll_position * ratio,
            slider_size,
            ratio,
        })
    }

    /// The bar's start along its axis.
    fn start(&self) -> Pixels {
        if self.vertical {
            self.bar.top()
        } else {
            self.bar.left()
        }
    }

    /// `position` along the bar's axis.
    fn along(&self, position: Point<Pixels>) -> Pixels {
        if self.vertical {
            position.y
        } else {
            position.x
        }
    }

    /// The slider, in window coordinates.
    fn slider(&self) -> Bounds<Pixels> {
        if self.vertical {
            Bounds::new(
                point(self.bar.left(), self.bar.top() + self.slider_position),
                size(self.bar.size.width, self.slider_size),
            )
        } else {
            Bounds::new(
                point(self.bar.left() + self.slider_position, self.bar.top()),
                size(self.slider_size, self.bar.size.height),
            )
        }
    }

    /// The scroll position putting the slider at `slider_position`.
    fn scroll_for_slider_position(&self, slider_position: Pixels) -> Pixels {
        if self.ratio > 0. {
            slider_position / self.ratio
        } else {
            px(0.)
        }
    }
}

/// A scrollbar slider drag: which bar, and where in the slider it was
/// grabbed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollbarDrag {
    vertical: bool,
    grab: Pixels,
}

/// What a pending reveal shows once its file's editor is laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RevealTarget {
    /// The modified editor's cursor, if the file has changes.
    Cursor,
    /// A line of one side.
    Line { row: u32, original: bool },
}

/// A reveal waiting for its file's diff and layout.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PendingReveal {
    item: usize,
    target: RevealTarget,
}

/// Monaco `MultiDiffEditorWidget`: diffs of many files in one list.
pub struct MultiDiffEditor {
    items: Vec<Item>,
    options: DiffEditorOptions,
    /// Diff editors bound to files (Monaco's acquired templates).
    bound: Vec<Template>,
    /// Unused diff editors (Monaco `VirtualizedTemplatePool._unused`).
    pool: Vec<Template>,
    /// Empty models for editors in the pool (Monaco `setDiffModel(null)`).
    empty_models: (Entity<TextModel>, Entity<TextModel>),
    /// Monaco `MultiDiffEditorViewModel.activeDiffItem`.
    active: Option<usize>,
    /// The file whose editor had focus when last drawn.
    focused_item: Option<usize>,
    file_actions: Vec<MultiDiffFileAction>,
    pending_reveal: Option<PendingReveal>,
    scroll_top: Pixels,
    scroll_left: Pixels,
    /// The scroll position last drawn, to reveal the scrollbars when it
    /// changes.
    drawn_scroll: Point<Pixels>,
    scrollbars: [Option<ScrollbarState>; 2],
    scrollbar_drag: Option<ScrollbarDrag>,
    scrollbar_fade: ScrollbarFade,
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
            bound: Vec::new(),
            pool: Vec::new(),
            empty_models: (
                cx.new(|_| TextModel::new_internal("")),
                cx.new(|_| TextModel::new_internal("")),
            ),
            active: None,
            focused_item: None,
            file_actions: Vec::new(),
            pending_reveal: None,
            scroll_top: px(0.),
            scroll_left: px(0.),
            drawn_scroll: Point::default(),
            scrollbars: [None, None],
            scrollbar_drag: None,
            scrollbar_fade: ScrollbarFade::default(),
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        while !self.bound.is_empty() {
            self.unbind(0, false, cx);
        }
        self.items = documents
            .into_iter()
            .map(|document| {
                let status = match (&document.original, &document.modified) {
                    (None, _) => FileStatus::Added,
                    (_, None) => FileStatus::Deleted,
                    _ if document.original_label.is_some() => FileStatus::Renamed,
                    _ => FileStatus::Modified,
                };
                // Placeholders for a missing side are the editor's own, not
                // listed models.
                let original = document
                    .original
                    .clone()
                    .unwrap_or_else(|| cx.new(|_| TextModel::new_internal("")));
                let modified = document
                    .modified
                    .clone()
                    .unwrap_or_else(|| cx.new(|_| TextModel::new_internal("")));
                Item {
                    document,
                    status,
                    original,
                    modified,
                    collapsed: false,
                    content_height: px(INITIAL_CONTENT_HEIGHT),
                    view_state: None,
                }
            })
            .collect();
        self.active = None;
        self.focused_item = None;
        self.pending_reveal = None;
        self.scroll_top = px(0.);
        self.scroll_left = px(0.);
        cx.notify();
    }

    /// A diff editor for the pool, with Monaco's item template options:
    /// the list scrolls, not the editors.
    fn create_template(&self, window: &mut Window, cx: &mut Context<Self>) -> Template {
        let options = self.item_options();
        let (original, modified) = self.empty_models.clone();
        let diff = cx.new(|cx| {
            let mut diff = DiffEditor::new(original, modified, window, cx);
            diff.update_options(options, cx);
            for editor in [
                diff.original_editor().clone(),
                diff.modified_editor().clone(),
            ] {
                editor.update(cx, |editor, cx| {
                    let mut options = editor.options().clone();
                    options.scroll_beyond_last_line = false;
                    options.scrollbar.vertical = ScrollbarVisibility::Hidden;
                    options.scrollbar.horizontal = ScrollbarVisibility::Hidden;
                    options.scrollbar.handle_mouse_wheel = false;
                    options.scrollbar.use_shadows = false;
                    // Nothing shows there, and the item is sized to its
                    // content.
                    options.horizontal_scrollbar_size = px(0.);
                    // Monaco's overview ruler is part of the hidden
                    // scrollbar, and `overviewRulerBorder` is off.
                    options.overview_ruler_lanes = 0;
                    options.overview_ruler_border = false;
                    options.minimap.enabled = false;
                    editor.update_options(options, cx);
                });
            }
            diff
        });
        let subscription = cx.subscribe(&diff, |this, diff, event: &DiffEditorEvent, cx| {
            if matches!(
                event,
                DiffEditorEvent::DiffUpdated | DiffEditorEvent::ModelChanged
            ) && let Some(template) = this.bound.iter_mut().find(|t| t.diff == diff)
            {
                template.stale = true;
            }
            cx.notify();
        });
        Template {
            diff,
            item: 0,
            stale: true,
            _subscription: subscription,
        }
    }

    /// Monaco `VirtualizedItemManager.acquire` and
    /// `DiffEditorItemTemplate.setItem`: shows file `index` in a pooled (or
    /// new) diff editor, with the view state it had.
    fn bind(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut template = match self.pool.pop() {
            Some(template) => template,
            None => self.create_template(window, cx),
        };
        let item = &self.items[index];
        let deleted = item.status == FileStatus::Deleted;
        let original_read_only = deleted || !self.options.original_editable;
        let (original, modified) = (item.original.clone(), item.modified.clone());
        let view_state = item.view_state.clone();
        template.diff.update(cx, |diff, cx| {
            diff.set_models(original, modified, cx);
            for (editor, read_only) in [
                (diff.original_editor().clone(), original_read_only),
                (diff.modified_editor().clone(), deleted),
            ] {
                editor.update(cx, |editor, cx| {
                    if editor.options().read_only != read_only {
                        let mut options = editor.options().clone();
                        options.read_only = read_only;
                        editor.update_options(options, cx);
                    }
                });
            }
            if let Some(state) = &view_state {
                diff.restore_view_state(state, cx);
            }
        });
        template.item = index;
        template.stale = true;
        self.bound.push(template);
    }

    /// Monaco `ManagedVirtualizedItem._clearBinding`: gives `self.bound[ix]`
    /// back to the pool, keeping the file's view state (Monaco
    /// `onWillUnbind`). The pool keeps up to five editors and drops the
    /// rest.
    fn unbind(&mut self, ix: usize, save: bool, cx: &mut Context<Self>) {
        let template = self.bound.remove(ix);
        if save && let Some(item) = self.items.get_mut(template.item) {
            item.view_state = Some(template.diff.read(cx).save_view_state(cx));
        }
        if self.pool.len() < MAX_UNUSED_EDITORS {
            let (original, modified) = self.empty_models.clone();
            template
                .diff
                .update(cx, |diff, cx| diff.set_models(original, modified, cx));
            self.pool.push(template);
        }
    }

    /// Binds the files in `wanted` and unbinds the others, unless focused
    /// or about to be revealed (Monaco `shouldKeepAlive`).
    fn update_bindings(&mut self, wanted: &[usize], window: &mut Window, cx: &mut Context<Self>) {
        let mut ix = 0;
        while ix < self.bound.len() {
            let item = self.bound[ix].item;
            let keep = wanted.contains(&item)
                || self.pending_reveal.is_some_and(|p| p.item == item)
                || self.bound[ix].is_focused(window, cx);
            if keep {
                ix += 1;
            } else {
                self.unbind(ix, true, cx);
            }
        }
        for &index in wanted {
            if !self.bound.iter().any(|t| t.item == index) {
                self.bind(index, window, cx);
            }
        }
    }

    /// Binds file `index` now, for navigation.
    fn ensure_bound(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<DiffEditor> {
        if !self.bound.iter().any(|t| t.item == index) {
            self.bind(index, window, cx);
        }
        self.bound
            .iter()
            .find(|t| t.item == index)
            .map(|t| t.diff.clone())
            .expect("just bound")
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
        let editable_changed = options.original_editable != self.options.original_editable;
        self.options = options;
        let options = self.item_options();
        for template in self.bound.iter().chain(&self.pool) {
            template
                .diff
                .update(cx, |diff, cx| diff.update_options(options.clone(), cx));
        }
        // A deleted file's original stays read-only.
        if editable_changed {
            for template in &self.bound {
                if self.items[template.item].status == FileStatus::Deleted {
                    let editor = template.diff.read(cx).original_editor().clone();
                    editor.update(cx, |editor, cx| {
                        let mut options = editor.options().clone();
                        options.read_only = true;
                        editor.update_options(options, cx);
                    });
                }
            }
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

    /// The diff editor showing file `index`, if one does: only the files in
    /// view (and a focused one) have an editor (Monaco
    /// `tryGetCodeEditor`).
    pub fn diff_editor(&self, index: usize) -> Option<&Entity<DiffEditor>> {
        self.bound.iter().find(|t| t.item == index).map(|t| &t.diff)
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

    /// The active file (Monaco `activeDiffItem`): the last one focused,
    /// revealed, navigated to or collapsed with its chevron. Its header and
    /// bottom border are in `focusBorder`.
    pub fn active_item(&self) -> Option<usize> {
        self.active
    }

    /// Adds a button to every file header's toolbar (Monaco's
    /// `MultiDiffEditorFileToolbar` menu). One with the same id is
    /// replaced.
    pub fn add_file_action(&mut self, action: MultiDiffFileAction, cx: &mut Context<Self>) {
        self.file_actions.retain(|a| a.id != action.id);
        self.file_actions.push(action);
        cx.notify();
    }

    /// Removes a [`MultiDiffEditor::add_file_action`] button; `false` if
    /// there was none with this id.
    pub fn remove_file_action(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let before = self.file_actions.len();
        self.file_actions.retain(|a| a.id != id);
        cx.notify();
        self.file_actions.len() != before
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

    /// The horizontal scroll position, shared by every file.
    pub fn scroll_left(&self) -> Pixels {
        self.scroll_left
    }

    /// Scrolls every file sideways (clamped when next drawn).
    pub fn set_scroll_left(&mut self, scroll_left: Pixels, cx: &mut Context<Self>) {
        self.scroll_left = scroll_left.max(px(0.));
        cx.notify();
    }

    /// Makes file `index` active and scrolls its header to the top (Monaco
    /// `reveal`).
    pub fn reveal(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.items.len() {
            self.active = Some(index);
        }
        let top = self.item_top(index.min(self.items.len()));
        self.set_scroll_top(top, cx);
    }

    /// Reveals file `index` and, once its diff is shown, centers `range`
    /// of its original or modified side (Monaco `reveal` with a `range`).
    pub fn reveal_range(
        &mut self,
        index: usize,
        range: Range<PointUtf16>,
        original: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.items.len() {
            return;
        }
        self.set_collapsed(index, false, cx);
        self.reveal(index, cx);
        self.ensure_bound(index, window, cx);
        self.pending_reveal = Some(PendingReveal {
            item: index,
            target: RevealTarget::Line {
                row: range.start.row,
                original,
            },
        });
    }

    /// Monaco `goToNextChange`: the next change of the active file, else
    /// the first change of the next file (wrapping around), expanding it
    /// and focusing its modified editor.
    pub fn go_to_next_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_to_change(true, window, cx);
    }

    /// Monaco `goToPreviousChange`: the previous change of the active
    /// file, else the last change of the previous file.
    pub fn go_to_previous_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_to_change(false, window, cx);
    }

    /// Goes to file `index`'s first change (Monaco `_goToFile`): expands
    /// and reveals it, puts the modified editor's cursor on the change and
    /// focuses it.
    pub fn go_to_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_file_at(index, true, window, cx);
    }

    /// Monaco `_navigateToChange`.
    fn navigate_to_change(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        let Some(current) = self.active.filter(|&ix| ix < self.items.len()) else {
            self.go_to_file_at(0, true, window, cx);
            return;
        };
        self.set_collapsed(current, false, cx);
        let diff = self.ensure_bound(current, window, cx);
        let has_more = {
            let diff = diff.read(cx);
            let line = diff.modified_editor().read(cx).selections()[0].head.row as usize + 1;
            diff.diff().is_some_and(|d| {
                if next {
                    d.changes
                        .iter()
                        .any(|c| c.modified.start_line_number > line)
                } else {
                    d.changes
                        .iter()
                        .any(|c| c.modified.end_line_number_exclusive <= line)
                }
            })
        };
        if has_more {
            diff.update(cx, |diff, cx| diff.go_to_diff(next, window, cx));
            self.pending_reveal = Some(PendingReveal {
                item: current,
                target: RevealTarget::Cursor,
            });
            cx.notify();
            return;
        }
        let count = self.items.len();
        let index = if next {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.go_to_file_at(index, next, window, cx);
    }

    /// Monaco `_goToFile`: the file's first or last change.
    fn go_to_file_at(
        &mut self,
        index: usize,
        first: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.items.len() {
            return;
        }
        self.set_collapsed(index, false, cx);
        self.reveal(index, cx);
        let diff = self.ensure_bound(index, window, cx);
        if first {
            diff.update(cx, |diff, cx| diff.reveal_first_diff(cx));
        } else if !diff.read(cx).is_computing_diff() {
            select_last_change(&diff, cx);
        } else {
            let wait = diff.update(cx, |diff, _| diff.wait_for_diff());
            let weak = diff.downgrade();
            cx.spawn(async move |_, cx| {
                wait.await;
                if let Some(diff) = weak.upgrade() {
                    cx.update(|cx| select_last_change(&diff, cx));
                }
            })
            .detach();
        }
        self.pending_reveal = Some(PendingReveal {
            item: index,
            target: RevealTarget::Cursor,
        });
        let modified = diff.read(cx).modified_editor().focus_handle(cx);
        window.focus(&modified, cx);
    }

    /// The top of file `index` in the list.
    fn item_top(&self, index: usize) -> Pixels {
        self.items[..index.min(self.items.len())]
            .iter()
            .map(Item::height)
            .fold(px(0.), |a, b| a + b)
    }

    fn total_height(&self) -> Pixels {
        self.item_top(self.items.len())
    }

    /// Monaco's `scrollWidth` less the width: the widest horizontal scroll
    /// range of the files with an editor.
    fn max_scroll_left(&self, cx: &App) -> Pixels {
        self.bound
            .iter()
            .map(|t| t.diff.read(cx).max_scroll_left(cx))
            .fold(px(0.), |a, b| a.max(b))
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(18.));
        let viewport = self.bounds.get().size;
        let max_top = (self.total_height() - viewport.height).max(px(0.));
        let max_left = self.max_scroll_left(cx);
        self.scroll_top = (self.scroll_top - delta.y).clamp(px(0.), max_top);
        self.scroll_left = (self.scroll_left - delta.x).clamp(px(0.), max_left);
        self.pending_reveal = None;
        cx.stop_propagation();
        cx.notify();
    }

    /// Monaco `AbstractScrollbar._onMouseDown`: grabbing the slider drags
    /// it; pressing the track centers the slider there, then drags it.
    fn on_scrollbar_mouse_down(
        &mut self,
        vertical: bool,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.scrollbars[usize::from(!vertical)] else {
            return;
        };
        let along = state.along(position) - state.start();
        let grab = if state.slider().contains(&position) {
            along - state.slider_position
        } else {
            state.slider_size / 2.
        };
        self.scrollbar_drag = Some(ScrollbarDrag { vertical, grab });
        self.pending_reveal = None;
        self.drag_scrollbar(position, cx);
    }

    /// Monaco `_sliderMouseDown`'s move handler.
    fn drag_scrollbar(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.scrollbar_drag else {
            return;
        };
        let Some(state) = self.scrollbars[usize::from(!drag.vertical)] else {
            return;
        };
        let slider_position = state.along(position) - state.start() - drag.grab;
        let scroll = state
            .scroll_for_slider_position(slider_position)
            .max(px(0.));
        if drag.vertical {
            self.scroll_top = scroll;
        } else {
            self.scroll_left = scroll;
        }
        cx.notify();
    }

    /// Takes each laid-out diff editor's content height once its diff is
    /// up to date, redrawing when one changed (Monaco
    /// `onDidContentSizeChange` and `_flushVerticalState`).
    fn measure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut redraw = false;
        for template in &mut self.bound {
            if std::mem::take(&mut template.stale) {
                // Laid out before the diff changed: measure after the next
                // layout.
                redraw = true;
                continue;
            }
            let diff = template.diff.read(cx);
            if diff.is_computing_diff() {
                continue;
            }
            let item = &mut self.items[template.item];
            if let Some(height) = diff.content_height(cx)
                && (height - item.content_height).abs() > px(0.5)
            {
                item.content_height = height;
                redraw = true;
            }
        }
        if redraw {
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
    }

    /// Scrolls a pending reveal's line to the middle of the list (Monaco
    /// `revealLineInCenter`), once the file's diff is shown and laid out.
    fn resolve_pending_reveal(&mut self, viewport: Pixels, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_reveal else {
            return;
        };
        let Some(template) = self.bound.iter().find(|t| t.item == pending.item) else {
            return;
        };
        let diff = template.diff.read(cx);
        if template.stale || diff.is_computing_diff() {
            return;
        }
        let (row, original) = match pending.target {
            RevealTarget::Cursor => {
                if diff.diff().is_none_or(|d| d.changes.is_empty()) {
                    self.pending_reveal = None;
                    return;
                }
                let row = diff.modified_editor().read(cx).selections()[0].head.row;
                (row, false)
            }
            RevealTarget::Line { row, original } => (row, original),
        };
        let editor = if original {
            diff.original_editor()
        } else {
            diff.modified_editor()
        }
        .read(cx);
        let (Some(top), Some(bottom)) = (
            editor.top_for_line_number(row, true),
            editor.bottom_for_line_number(row, false),
        ) else {
            return;
        };
        let offset = self.item_top(pending.item) + px(HEADER_HEIGHT);
        let center = offset + (top + bottom) / 2.;
        self.scroll_top = (center - viewport / 2.).max(px(0.));
        self.pending_reveal = None;
        cx.notify();
    }

    /// Follows the mouse for the scrollbars' visibility and hover color,
    /// redrawing only when they change.
    fn on_mouse_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let over = self.bounds.get().contains(&position);
        let slider_hovered = |fade: &ScrollbarFade, scrollbars: &[Option<ScrollbarState>; 2]| {
            scrollbars
                .iter()
                .flatten()
                .any(|s| fade.mouse_position.is_some_and(|m| s.slider().contains(&m)))
        };
        let hovered_before = slider_hovered(&self.scrollbar_fade, &self.scrollbars);
        let was_over = self.scrollbar_fade.mouse_over;
        self.scrollbar_fade.mouse_position = over.then_some(position);
        self.scrollbar_fade.mouse_over = over;
        if was_over && !over {
            self.scrollbar_fade.left_at = Some(web_time::Instant::now());
        }
        if was_over != over
            || hovered_before != slider_hovered(&self.scrollbar_fade, &self.scrollbars)
        {
            cx.notify();
        }
    }

    /// File `index`'s top and height, for tests.
    #[doc(hidden)]
    pub fn item_bounds_for_tests(&self, index: usize) -> (Pixels, Pixels) {
        (self.item_top(index), self.items[index].height())
    }

    /// The files with a diff editor, in order, for tests.
    #[doc(hidden)]
    pub fn bound_items_for_tests(&self) -> Vec<usize> {
        let mut items: Vec<usize> = self.bound.iter().map(|t| t.item).collect();
        items.sort_unstable();
        items
    }

    /// The diff editors alive (bound and pooled), for tests.
    #[doc(hidden)]
    pub fn diff_editor_count_for_tests(&self) -> usize {
        self.bound.len() + self.pool.len()
    }

    /// The vertical (or horizontal) scrollbar's bar and slider in window
    /// coordinates, as last drawn, for tests.
    #[doc(hidden)]
    pub fn scrollbar_for_tests(&self, vertical: bool) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
        let state = self.scrollbars[usize::from(!vertical)]?;
        Some((state.bar, state.slider()))
    }

    /// The header's content: chevron, file label, status and toolbar
    /// (Monaco `DiffEditorItemTemplate`'s `.header-content`).
    fn render_header(
        &self,
        index: usize,
        stuck: bool,
        pushed_up: bool,
        colors: &EditorColors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let item = &self.items[index];
        let collapsed = item.collapsed;
        let border = if self.active == Some(index) {
            colors.focus_border
        } else {
            colors.multi_diff_border
        };
        // TODO(#246): Monaco's `chevron-right`/`chevron-down` codicons.
        let chevron = if collapsed { "›" } else { "⌄" };
        let status = match item.status {
            FileStatus::Modified => None,
            FileStatus::Added => Some("A"),
            FileStatus::Deleted => Some("D"),
            FileStatus::Renamed => Some("R"),
        };
        let foreground = colors.multi_diff_header_foreground;
        let label = item.document.label.clone();
        let actions = self.file_actions.iter().enumerate().map(|(ix, action)| {
            let action = action.clone();
            let document = item.document.clone();
            let label = action.label.clone();
            div()
                .id(("multi-diff-file-action", ix))
                .tooltip(crate::tooltip::tooltip(label.clone(), colors))
                .role(gpui::Role::Button)
                .aria_label(label)
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(22.))
                .h(px(22.))
                .p(px(3.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(|style| colors.toolbar_hover(style))
                .child(action.icon.clone())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.active = Some(index);
                    (action.run)(index, &document, window, cx);
                    cx.notify();
                }))
        });
        // Monaco's `.header:not(.collapsed)` bottom border: a header pushed
        // up by the next file, or a collapsed file's, has none.
        let bottom_border = colors
            .multi_diff_header_border
            .filter(|_| !pushed_up && !collapsed);
        div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .size_full()
            .px(px(6.))
            .py(px(1.))
            .bg(colors.multi_diff_header_background)
            .child(
                div()
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .right(px(0.))
                    .h(px(1.))
                    .bg(border),
            )
            .when_some(bottom_border, |header, bottom| {
                header.child(
                    div()
                        .absolute()
                        .bottom(px(0.))
                        .left(px(0.))
                        .right(px(0.))
                        .h(px(1.))
                        .bg(bottom),
                )
            })
            .text_color(foreground)
            .text_size(px(14.))
            .line_height(px(22.))
            // Monaco's `.header-content.shadow`, once the file is scrolled
            // under its header.
            .when(stuck, |header| {
                header.shadow(vec![BoxShadow {
                    color: colors.scrollbar_shadow,
                    offset: point(px(0.), px(6.)),
                    blur_radius: px(6.),
                    spread_radius: px(-6.),
                    inset: false,
                }])
            })
            .child(
                div()
                    .id(("multi-diff-collapse", index))
                    .debug_selector(|| format!("multi-diff-collapse-{index}"))
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(format!(
                        "{}, {}",
                        label,
                        if collapsed { "collapsed" } else { "expanded" }
                    )))
                    .mx(px(6.))
                    .w(px(16.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(chevron)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // Monaco `activateItem`.
                        this.active = Some(index);
                        this.set_collapsed(index, !collapsed, cx)
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .child(resource_label(
                        &label,
                        item.status == FileStatus::Deleted,
                        true,
                        colors,
                    ))
                    .when_some(status, |path, status| {
                        // Monaco's `.status`: semibold at 0.75 opacity.
                        path.child(
                            div()
                                .flex_none()
                                .mx(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(foreground.opacity(0.75))
                                .child(status),
                        )
                    })
                    .when_some(item.document.original_label.clone(), |path, original| {
                        path.child(resource_label(&original, true, false, colors))
                    }),
            )
            .when(!self.file_actions.is_empty(), |header| {
                header.child(
                    div()
                        .id(("multi-diff-file-actions", index))
                        .flex()
                        .flex_row()
                        .flex_none()
                        .items_center()
                        .gap(px(4.))
                        .px(px(8.))
                        .children(actions),
                )
            })
    }

    /// The list's scrollbars (Monaco `SmoothScrollableElement`'s, `Auto`
    /// and without shadows).
    fn render_scrollbars(
        &self,
        origin: Point<Pixels>,
        colors: &EditorColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let (opacity, animating) = self.scrollbar_fade.opacity(self.scrollbar_drag.is_some());
        if animating {
            window.request_animation_frame();
        }
        let mut elements = Vec::new();
        for state in self.scrollbars.iter().flatten() {
            let vertical = state.vertical;
            let dragged = self.scrollbar_drag.is_some_and(|d| d.vertical == vertical);
            let slider = state.slider();
            let hovered = self
                .scrollbar_fade
                .mouse_position
                .is_some_and(|m| slider.contains(&m));
            let color = if dragged {
                colors.scrollbar_slider_active
            } else if hovered {
                colors.scrollbar_slider_hover
            } else {
                colors.scrollbar_slider
            };
            let bar = state.bar;
            elements.push(
                div()
                    .id(if vertical {
                        "multi-diff-vertical-scrollbar"
                    } else {
                        "multi-diff-horizontal-scrollbar"
                    })
                    .absolute()
                    .left(bar.left() - origin.x)
                    .top(bar.top() - origin.y)
                    .w(bar.size.width)
                    .h(bar.size.height)
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.on_scrollbar_mouse_down(vertical, event.position, cx);
                        }),
                    )
                    .when(opacity > 0., |bar_div| {
                        bar_div.child(
                            div()
                                .absolute()
                                .left(slider.left() - bar.left())
                                .top(slider.top() - bar.top())
                                .w(slider.size.width)
                                .h(slider.size.height)
                                .bg(color.opacity(opacity)),
                        )
                    })
                    .into_any_element(),
            );
        }
        if self.scrollbar_drag.is_some() {
            elements.push(window_drag_listener(cx.entity().downgrade()).into_any_element());
        }
        elements
    }
}

/// Monaco `_goToFile(_, 'last')`: puts the modified editor's cursor on the
/// last change.
fn select_last_change(diff: &Entity<DiffEditor>, cx: &mut App) {
    diff.update(cx, |diff, cx| {
        let Some(change) = diff.diff().and_then(|d| d.changes.last()) else {
            return;
        };
        let row = change.modified.start_line_number as u32 - 1;
        diff.modified_editor().update(cx, |editor, cx| {
            editor.set_selections(&[Selection::collapsed(PointUtf16::new(row, 0))], cx)
        });
    });
}

/// A file's label as Monaco's resource label shows it: a file icon, the
/// name, then the folder as a description.
fn resource_label(
    path: &str,
    strikethrough: bool,
    primary: bool,
    colors: &EditorColors,
) -> impl IntoElement {
    let (folder, name) = match path.rsplit_once('/') {
        Some((folder, name)) => (Some(folder.to_string()), name.to_string()),
        None => (None, path.to_string()),
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .min_w(px(0.))
        .overflow_hidden()
        .whitespace_nowrap()
        // Monaco: the primary title `flex: 0 1 auto`, the original
        // `1 1 auto`.
        .when(!primary, |label| label.flex_1())
        .child(
            // TODO(#246): the file's icon (Monaco's `file` codicon or the
            // icon theme's).
            div()
                .flex_none()
                .pr(px(6.))
                .text_color(colors.symbol_icons.file)
                .child("▤"),
        )
        .child(
            div()
                .flex_none()
                .when(strikethrough, |name| name.line_through())
                .child(name),
        )
        .when_some(folder, |label, folder| {
            // Monaco's `.label-description`.
            label.child(
                div()
                    .ml(px(7.))
                    .text_size(px(12.6))
                    .text_color(colors.multi_diff_header_foreground.opacity(0.7))
                    .overflow_hidden()
                    .text_ellipsis()
                    .when(strikethrough, |description| description.line_through())
                    .child(folder),
            )
        })
}

/// A canvas that listens to the window's mouse moves and ups while a
/// scrollbar drag goes on, wherever the mouse is (Monaco's global pointer
/// monitor).
fn window_drag_listener(this: WeakEntity<MultiDiffEditor>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let moves = this.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    moves
                        .update(cx, |this, cx| this.drag_scrollbar(event.position, cx))
                        .ok();
                }
            });
            let ups = this.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    ups.update(cx, |this, cx| {
                        this.scrollbar_drag = None;
                        cx.notify();
                    })
                    .ok();
                }
            });
        },
    )
    .absolute()
    .size_0()
}

/// The background, `colors` of any editor, the first bound one's if any.
fn list_colors(this: &MultiDiffEditor, cx: &App) -> EditorColors {
    let diff = this
        .bound
        .first()
        .or_else(|| this.pool.first())
        .map(|t| t.diff.clone());
    match diff {
        Some(diff) => diff.read(cx).modified_editor().read(cx).colors().clone(),
        None => crate::theme::global_theme(cx)
            .map(|theme| theme.colors.clone())
            .unwrap_or_else(|| crate::theme::Theme::vs_dark().colors),
    }
}

impl Render for MultiDiffEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.get();
        // Before the first layout, the window's height.
        let viewport = if bounds.size.height > px(0.) {
            bounds.size
        } else {
            window.viewport_size()
        };
        self.resolve_pending_reveal(viewport.height, cx);
        self.measure(window, cx);
        let max_top = (self.total_height() - viewport.height).max(px(0.));
        self.scroll_top = self.scroll_top.clamp(px(0.), max_top);
        let max_left = self.max_scroll_left(cx);
        if bounds.size.width > px(0.) {
            self.scroll_left = self.scroll_left.clamp(px(0.), max_left);
        }
        let scroll_top = self.scroll_top;
        let scroll_left = self.scroll_left;
        // Monaco reveals `auto` scrollbars whenever the list scrolls.
        if self.drawn_scroll != point(scroll_left, scroll_top) {
            self.drawn_scroll = point(scroll_left, scroll_top);
            self.scrollbar_fade.revealed_at = Some(web_time::Instant::now());
        }

        // The files in view (Monaco's rendered range).
        let mut visible = Vec::new();
        let mut top = px(0.);
        for (index, item) in self.items.iter().enumerate() {
            let height = item.height();
            let item_top = top;
            top += height;
            if item_top >= scroll_top + viewport.height {
                break;
            }
            if item_top + height > scroll_top {
                visible.push((index, item_top));
            }
        }
        let wanted: Vec<usize> = visible
            .iter()
            .map(|&(index, _)| index)
            .filter(|&index| !self.items[index].collapsed)
            .collect();
        self.update_bindings(&wanted, window, cx);
        // Monaco `setIsFocused`: a file whose editor gains focus becomes the
        // active one.
        let focused = self
            .bound
            .iter()
            .find(|t| t.is_focused(window, cx))
            .map(|t| t.item);
        if focused != self.focused_item {
            self.focused_item = focused;
            if focused.is_some() {
                self.active = focused;
            }
        }

        // Monaco `ScrollableElement`: each bar leaves room for the other.
        self.scrollbars = [
            ScrollbarState::new(
                Bounds::new(
                    point(bounds.right() - px(SCROLLBAR_SIZE), bounds.top()),
                    size(px(SCROLLBAR_SIZE), bounds.size.height - px(SCROLLBAR_SIZE)),
                ),
                true,
                bounds.size.height,
                self.total_height(),
                scroll_top,
            ),
            ScrollbarState::new(
                Bounds::new(
                    point(bounds.left(), bounds.bottom() - px(SCROLLBAR_SIZE)),
                    size(bounds.size.width - px(SCROLLBAR_SIZE), px(SCROLLBAR_SIZE)),
                ),
                false,
                bounds.size.width,
                bounds.size.width + max_left,
                scroll_left,
            ),
        ];

        let colors = list_colors(self, cx);
        let mut children = Vec::new();
        for &(index, item_top) in &visible {
            let item = &self.items[index];
            let height = item.height();
            let collapsed = item.collapsed;
            // The visible part: the editor scrolled to the part under the
            // header, which sticks to the top until the next file pushes it
            // up (Monaco's `translateY(delta)`).
            let visible_start = item_top.max(scroll_top);
            let visible_end = (item_top + height).min(scroll_top + viewport.height);
            let delta = (visible_start - item_top)
                .min(height - px(HEADER_HEIGHT))
                .max(px(0.));
            let header_top = item_top + delta - visible_start;
            let editor_scroll = visible_start - item_top;
            let editor_height = (visible_end - visible_start - px(HEADER_HEIGHT)).max(px(0.));
            let shows_bottom = visible_end >= item_top + height;
            let diff = self
                .bound
                .iter()
                .find(|t| t.item == index)
                .map(|t| t.diff.clone());
            if let Some(diff) = &diff {
                diff.update(cx, |diff, cx| {
                    diff.set_scroll_position_unclamped(point(scroll_left, editor_scroll), cx)
                });
            }
            let border = if self.active == Some(index) {
                colors.focus_border
            } else {
                colors.multi_diff_border
            };
            let header = self.render_header(
                index,
                visible_start > item_top,
                delta >= height - px(HEADER_HEIGHT),
                &colors,
                cx,
            );
            children.push(
                div()
                    .absolute()
                    .top(visible_start - scroll_top)
                    .left(px(0.))
                    .right(px(0.))
                    .h(visible_end - visible_start)
                    .overflow_hidden()
                    .when_some(
                        diff.filter(|_| !collapsed && editor_height > px(0.)),
                        |item_div, diff| {
                            item_div.child(
                                div()
                                    .absolute()
                                    .top(px(HEADER_HEIGHT))
                                    .left(px(HORIZONTAL_INSET))
                                    .right(px(HORIZONTAL_INSET))
                                    .h(editor_height)
                                    .child(diff),
                            )
                        },
                    )
                    // Monaco's `.editorParent` bottom border.
                    .when(!collapsed && shows_bottom, |item_div| {
                        item_div.child(
                            div()
                                .absolute()
                                .bottom(px(0.))
                                .left(px(0.))
                                .right(px(0.))
                                .h(px(1.))
                                .bg(border),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .top(header_top)
                            .left(px(0.))
                            .right(px(0.))
                            .h(px(HEADER_HEIGHT))
                            .pt(px(HEADER_MARGIN_TOP))
                            .bg(colors.multi_diff_background)
                            .child(header),
                    ),
            );
        }
        let scrollbars = self.render_scrollbars(bounds.origin, &colors, window, cx);
        let bounds_cell = self.bounds.clone();
        let this = cx.entity().downgrade();
        div()
            .id("multi-diff-editor")
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(colors.multi_diff_background)
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(
                canvas(
                    move |b, _, _| bounds_cell.set(b),
                    move |_, _, window, _| {
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                this.update(cx, |this, cx| this.on_mouse_move(event.position, cx))
                                    .ok();
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(children)
            .children(scrollbars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(length: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(10.), px(length)))
    }

    #[test]
    fn scrollbar_state() {
        // Monaco `ScrollbarState` tests: everything fits, no scrollbar.
        assert_eq!(
            ScrollbarState::new(bar(100.), true, px(100.), px(100.), px(0.)),
            None
        );
        // A 1000px list in a 100px viewport: a 20px minimum slider.
        let state = ScrollbarState::new(bar(90.), true, px(100.), px(1000.), px(450.)).unwrap();
        assert_eq!(state.slider_size, px(20.));
        assert!((state.ratio - 70. / 900.).abs() < 1e-6);
        assert_eq!(state.slider_position, px(35.));
        assert!((state.scroll_for_slider_position(px(70.)) - px(900.)).abs() < px(0.01));
        // Proportional when larger than the minimum.
        let state = ScrollbarState::new(bar(200.), true, px(200.), px(400.), px(0.)).unwrap();
        assert_eq!(state.slider_size, px(100.));
    }
}
