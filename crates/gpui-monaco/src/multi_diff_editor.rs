//! The multi-file diff editor (Monaco `MultiDiffEditorWidget`, MIT): one
//! scrolling list of diff editors, each under a collapsible file header.
//!
//! Like Monaco, each diff editor is as tall as its content (with unchanged
//! regions hidden), the list scrolls them as one, and only the visible part
//! of each is laid out: the list sizes the editor to what shows and scrolls
//! it to match (what Monaco's `CompressedVirtualizedScrollView` does by
//! capping each row at the viewport's height). When files change height,
//! the list keeps what's at the top of the viewport in place, as Monaco's
//! anchored geometry edits do (see `scroll_layout`).
//!
//! Monaco's `VirtualizedItemManager` binds diff editors only to the files in
//! view: a file scrolled out gives its editor back to a pool (Monaco
//! `VirtualizedTemplatePool`, which keeps up to five unused ones), keeping
//! its view state and measured height for when it shows again. Binary files
//! show Monaco's "Binary file changed" placeholder instead. The list has
//! Monaco's `SmoothScrollableElement` scrollbars and smooth wheel
//! scrolling, and the headers follow one of the three variants of Monaco's
//! `style.css` ([`MultiDiffEditorVariant`]).
//!
//! The files are keyed like Monaco's `mapObservableArrayCached` items:
//! updating the list keeps each file still there as it was. When files load
//! and none is active, the list goes to the first change; an empty list
//! says "No Changed Files" (or "Loading...").

mod scroll_layout;

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, BoxShadow, Context, DispatchPhase, Entity, EntityId,
    EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Point,
    Render, ScrollDelta, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, canvas, div, point, px, size,
};
use text::PointUtf16;
use web_time::Instant;

use crate::codicons::{Codicon, icon as codicon};
use crate::context_keys::ContextKeyValue;
use crate::cursor::Selection;
use crate::decorations::DecorationOptions;
use crate::diff_editor::{DiffEditor, DiffEditorEvent, DiffEditorOptions, DiffEditorViewState};
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::events::CursorChangeSource;
use crate::model::TextModel;
use crate::options::{AccessibilitySupport, ScrollbarVisibility, WordWrapOverride};
use crate::theme::EditorColors;
use crate::view::animation::{ScrollbarFade, ease_out_cubic};
use scroll_layout::ScrollAnchoring;

/// Space above the standard variant's header content (its `.header-content`
/// margin).
const HEADER_MARGIN_TOP: f32 = 8.;
/// The height assumed for a file before its diff editor is laid out,
/// header and bottom padding included (Monaco's initial
/// `lastTemplateData.expandedContentHeight`).
const INITIAL_EXPANDED_HEIGHT: f32 = 500.;
/// The list's scrollbars' size (Monaco `ScrollableElement`'s default
/// `verticalScrollbarSize` and `horizontalScrollbarSize`).
const SCROLLBAR_SIZE: f32 = 10.;
/// Monaco `ScrollbarState`'s `MINIMUM_SLIDER_SIZE`.
const MIN_SLIDER: f32 = 20.;
/// How many unused diff editors the pool keeps (Monaco
/// `VirtualizedTemplatePool`).
const MAX_UNUSED_EDITORS: usize = 5;
/// A binary file's placeholder height (Monaco
/// `binaryFilePlaceholderContentHeight`).
const BINARY_PLACEHOLDER_HEIGHT: f32 = 100.;
/// The list's smooth scroll duration (the `smoothScrollDuration` of
/// Monaco `CompressedVirtualizedScrollView`'s `Scrollable`).
const SMOOTH_SCROLL_DURATION: Duration = Duration::from_millis(100);
/// How long a revealed range stays highlighted (Monaco `highlightRange`).
const REVEAL_HIGHLIGHT_DURATION: Duration = Duration::from_millis(350);
/// `--vscode-cornerRadius-medium` (`cornerRadius.medium`, 6px).
const CORNER_RADIUS_MEDIUM: f32 = 6.;
/// The compact variants' header content height
/// (`--vscode-spacing-size320`).
const COMPACT_HEADER_CONTENT_HEIGHT: f32 = 32.;
/// The card variant's space around its cards (`--vscode-spacing-size80`
/// padding, `--vscode-spacing-size40` above the header).
const CARD_PADDING_X: f32 = 8.;
const CARD_MARGIN_TOP: f32 = 4.;
/// The inset of the compact variants' line between files
/// (`--vscode-spacing-size100`).
const COMPACT_SEPARATOR_INSET: f32 = 10.;

/// The layout and look of a multi-diff editor (Monaco
/// `MultiDiffEditorVariant`, from `multiDiffEditorOptions.ts` and the
/// variant classes of `style.css`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MultiDiffEditorVariant {
    /// Monaco `cards`: each file in a bordered card with a rounded header
    /// (`multiDiffEditor-compact multiDiffEditor-card`); headers collapse
    /// their file when clicked, and original line numbers are hidden.
    Cards,
    /// Monaco `noCards`: compact 32px headers on the list's background with
    /// a line between files (`multiDiffEditor-compact`); headers collapse
    /// their file when clicked, original line numbers are hidden and the
    /// diffs span the whole width.
    NoCards,
    /// Monaco `noCardsNonCompact` (`multiDiffEditor-standard`), what
    /// standalone Monaco's `createMultiFileDiffEditor` uses: 40px headers
    /// with a border above, collapsed with their chevron only.
    #[default]
    NoCardsNonCompact,
}

/// Monaco `IMultiDiffEditorVariantConfiguration`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct VariantConfiguration {
    /// The `multiDiffEditor-compact` class (else `-standard`).
    compact: bool,
    /// The `multiDiffEditor-card` class.
    card: bool,
    /// `horizontalInsets` (the same on both sides).
    horizontal_inset: Pixels,
    /// `headerHeight`.
    header_height: Pixels,
    /// `contentBottomPadding`.
    content_bottom_padding: Pixels,
    /// `headerClickToCollapse`.
    header_click_to_collapse: bool,
    /// `diffEditorOptions.hideOriginalLineNumbers`.
    hide_original_line_numbers: bool,
    /// Where the editor starts: the header's height in `style.css` (the
    /// card's header is 4px shorter than `headerHeight`).
    editor_top: Pixels,
}

impl MultiDiffEditorVariant {
    /// Monaco `getMultiDiffEditorVariantConfiguration`.
    fn configuration(self) -> VariantConfiguration {
        match self {
            Self::NoCardsNonCompact => VariantConfiguration {
                compact: false,
                card: false,
                horizontal_inset: px(9.),
                header_height: px(40.),
                content_bottom_padding: px(0.),
                header_click_to_collapse: false,
                hide_original_line_numbers: false,
                editor_top: px(40.),
            },
            Self::NoCards => VariantConfiguration {
                compact: true,
                card: false,
                horizontal_inset: px(0.),
                header_height: px(32.),
                content_bottom_padding: px(8.),
                header_click_to_collapse: true,
                hide_original_line_numbers: true,
                editor_top: px(COMPACT_HEADER_CONTENT_HEIGHT),
            },
            Self::Cards => VariantConfiguration {
                compact: true,
                card: true,
                horizontal_inset: px(9.),
                header_height: px(40.),
                content_bottom_padding: px(0.),
                header_click_to_collapse: true,
                hide_original_line_numbers: true,
                editor_top: px(CARD_MARGIN_TOP + COMPACT_HEADER_CONTENT_HEIGHT),
            },
        }
    }
}

/// A file in a multi-diff editor (Monaco `IDocumentDiffItem`).
///
/// [`MultiDiffEditor::set_documents`] matches files to the ones shown by
/// their [`key`](Self::key), so a file still there keeps its state.
#[derive(Clone, Default)]
pub struct MultiDiffDocument {
    /// The header's title, usually the modified file's path. The part
    /// after the last `/` shows as the name, the rest as its description.
    pub label: SharedString,
    /// The original file's path when it differs (a rename). The header
    /// shows `R` and this path only when both sides exist and it isn't
    /// `label` (Monaco compares the two uris' paths).
    pub original_label: Option<SharedString>,
    /// The original text; `None` if the file was created.
    pub original: Option<Entity<TextModel>>,
    /// The modified text; `None` if the file was deleted.
    pub modified: Option<Entity<TextModel>>,
    /// The file isn't text (Monaco `isBinary`: a side without a text
    /// model). It shows a "Binary file changed" placeholder instead of a
    /// diff; `original` and `modified` still say which sides exist, and
    /// their text isn't shown.
    pub binary: bool,
    /// This file's diff editor options (Monaco `IDocumentDiffItem.options`),
    /// in place of [`MultiDiffEditor::options`]; `None` uses those. Change
    /// them with [`MultiDiffEditor::set_document_options`] (Monaco
    /// `onOptionsDidChange`).
    pub options: Option<DiffEditorOptions>,
    /// Whether this file hides its original line numbers in the inline
    /// view (Monaco's `hideOriginalLineNumbers` in `options`), over the
    /// variant's default; `None` leaves it to the variant (and to
    /// [`DiffEditorOptions::hide_original_line_numbers`], which can only
    /// hide them).
    pub hide_original_line_numbers: Option<bool>,
    /// Context keys set on this file's editors while it has them (Monaco
    /// `IDocumentDiffItem.contextKeys`), for keybindings' `when`.
    pub context_keys: Vec<(SharedString, ContextKeyValue)>,
    /// Editor options for this file's editors (the editor options in
    /// Monaco's `IDocumentDiffItem.options`), such as read-only. Change
    /// them with [`MultiDiffEditor::set_document_editor_options`].
    pub editor_options: MultiDiffDocumentEditorOptions,
}

/// Editor options for one file's editors: the editor options part of
/// Monaco's `IDocumentDiffItem.options` (an `IDiffEditorOptions`, which
/// extends the editor options), which its template applies with
/// `updateOptions` each time it shows the file. `None` leaves the list's
/// default, which a reused editor gets back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MultiDiffDocumentEditorOptions {
    /// The modified side can't be edited (Monaco `readOnly`; default
    /// `false`). The original side follows
    /// [`DiffEditorOptions::original_editable`], as in Monaco, and a deleted
    /// file's (empty) modified side is always read-only.
    pub read_only: Option<bool>,
    /// Both editors' screen reader support (Monaco `accessibilitySupport`;
    /// default the editors' own, [`AccessibilitySupport::Auto`]).
    pub accessibility_support: Option<AccessibilitySupport>,
}

/// The layout every file's diff takes, over its own options (Monaco's
/// `_diffLayoutOptions`, from `setRenderSideBySide`, `setViewMode`,
/// `setDiffLayoutOptions` and `setDiffWordWrap`). `None` leaves each file's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MultiDiffLayoutOptions {
    /// [`DiffEditorOptions::render_side_by_side`].
    pub render_side_by_side: Option<bool>,
    /// [`DiffEditorOptions::use_inline_view_when_space_is_limited`].
    pub use_inline_view_when_space_is_limited: Option<bool>,
    /// [`DiffEditorOptions::diff_word_wrap`].
    pub diff_word_wrap: Option<WordWrapOverride>,
}

/// How the diffs show (Monaco `DiffEditorViewMode`), for
/// [`MultiDiffEditor::set_view_mode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffEditorViewMode {
    /// Deleted lines inline in the modified editor.
    Inline,
    /// The sides next to each other.
    SideBySide,
    /// Side by side, inline when a diff is narrower than its
    /// [`DiffEditorOptions::render_side_by_side_inline_breakpoint`].
    Automatic,
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
            ..Self::default()
        }
    }

    /// The file's key in a [`MultiDiffEditorViewState`] (Monaco
    /// `DocumentDiffItemViewModel.getKey`).
    pub fn key(&self) -> MultiDiffDocumentKey {
        MultiDiffDocumentKey {
            original: self.original.as_ref().map(|_| {
                self.original_label
                    .clone()
                    .unwrap_or_else(|| self.label.clone())
            }),
            modified: self.modified.as_ref().map(|_| self.label.clone()),
        }
    }
}

/// Which file a [`MultiDiffDocState`] is for (Monaco's item key, its
/// original and modified uris): the original and modified paths, `None`
/// for a side that doesn't exist.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MultiDiffDocumentKey {
    /// The original path (`original_label`, else `label`).
    pub original: Option<SharedString>,
    /// The modified path (`label`).
    pub modified: Option<SharedString>,
}

/// A file's part of a [`MultiDiffEditorViewState`] (Monaco
/// `IMultiDiffDocState`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiDiffDocState {
    /// Whether it's collapsed to its header.
    pub collapsed: bool,
    /// Its modified editor's selections, if it had an editor.
    pub selections: Option<Vec<Selection>>,
}

/// The whole list's view state (Monaco `IMultiDiffEditorViewState`), from
/// [`MultiDiffEditor::save_view_state`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiDiffEditorViewState {
    /// The vertical scroll position (Monaco `scrollState.top`).
    pub scroll_top: Pixels,
    /// The horizontal one (`scrollState.left`).
    pub scroll_left: Pixels,
    /// Each file's state (Monaco `docStates`).
    pub doc_states: HashMap<MultiDiffDocumentKey, MultiDiffDocState>,
    /// The active file (Monaco `activeDiffItemKey`).
    pub active_item: Option<MultiDiffDocumentKey>,
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
    /// [`MultiDiffEditor::active_control`] changed (Monaco
    /// `onDidChangeActiveControl`): another file became active, or the
    /// active file got or lost its diff editor.
    ActiveControlChanged,
}

/// What a [`MultiDiffFileAction`] runs: it gets the file's index and
/// document.
pub type MultiDiffFileHandler = Rc<dyn Fn(usize, &MultiDiffDocument, &mut Window, &mut App)>;

/// Options every file takes over its own (Monaco's widget-level
/// `diffEditorOptions`, spread after each file's), from
/// [`MultiDiffEditor::set_options_override`]: it changes the options a
/// file would otherwise have, such as `|o| o.ignore_trim_whitespace = true`.
pub type MultiDiffOptionsOverride = Rc<dyn Fn(&mut DiffEditorOptions)>;

/// A button in each file header's toolbar (a Monaco
/// `MultiDiffEditorFileToolbar` menu item), from
/// [`MultiDiffEditor::add_file_action`].
///
/// Its [`when`](Self::when) and [`toggled`](Self::toggled) conditions are
/// context key expressions (GPUI's syntax: `a && !b`, `key == value`) over
/// each file's keys: the list's ([`MultiDiffEditor::context_keys`]), the
/// file's [`MultiDiffDocument::context_keys`] and
/// `multiDiffEditorItemAllUnchangedRegionsShown` (Monaco's toolbar context
/// key scope).
#[derive(Clone)]
pub struct MultiDiffFileAction {
    /// Its id, for [`MultiDiffEditor::remove_file_action`].
    pub id: SharedString,
    /// Its tooltip and screen reader label.
    pub label: SharedString,
    /// The button's label: text, where `$(name)` is the codicon `name`,
    /// such as `"$(unfold)"` (Monaco shows the menu item's codicon).
    pub icon: SharedString,
    /// The files whose header shows it (Monaco's menu item `when`); `None`
    /// shows it on every file, and one that doesn't parse on none.
    pub when: Option<SharedString>,
    /// When it shows as toggled on (Monaco's menu item `toggled`, with the
    /// toolbar's `highlightToggledItems`); `None` for a plain button.
    pub toggled: Option<MultiDiffFileActionToggle>,
    /// Runs it.
    pub run: MultiDiffFileHandler,
}

impl MultiDiffFileAction {
    /// A button shown on every file, never toggled.
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        run: MultiDiffFileHandler,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: icon.into(),
            when: None,
            toggled: None,
            run,
        }
    }

    /// Sets [`MultiDiffFileAction::when`].
    pub fn with_when(mut self, when: impl Into<SharedString>) -> Self {
        self.when = Some(when.into());
        self
    }

    /// Sets [`MultiDiffFileAction::toggled`].
    pub fn with_toggled(mut self, toggled: MultiDiffFileActionToggle) -> Self {
        self.toggled = Some(toggled);
        self
    }
}

impl std::fmt::Debug for MultiDiffFileAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiDiffFileAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("icon", &self.icon)
            .field("when", &self.when)
            .field("toggled", &self.toggled)
            .finish_non_exhaustive()
    }
}

/// When a [`MultiDiffFileAction`] is toggled on, and how it shows then
/// (Monaco `ICommandActionToggleInfo`): highlighted
/// (`actionBar.toggledBackground`), with its own icon and label if set.
#[derive(Clone, Debug, PartialEq)]
pub struct MultiDiffFileActionToggle {
    /// The condition (Monaco `condition`); one that doesn't parse never
    /// holds.
    pub condition: SharedString,
    /// The icon while toggled on (Monaco `icon`), else the action's.
    pub icon: Option<SharedString>,
    /// The tooltip and label while toggled on (Monaco `title`), else the
    /// action's.
    pub label: Option<SharedString>,
}

impl MultiDiffFileActionToggle {
    /// Toggled on while `condition` holds, keeping the action's icon and
    /// label.
    pub fn new(condition: impl Into<SharedString>) -> Self {
        Self {
            condition: condition.into(),
            icon: None,
            label: None,
        }
    }
}

/// A parsed context key condition; `None` if it doesn't parse (never
/// holds).
type Condition = Option<Rc<gpui::KeyBindingContextPredicate>>;

fn parse_condition(when: &str) -> Condition {
    gpui::KeyBindingContextPredicate::parse(when)
        .ok()
        .map(Rc::new)
}

fn holds(condition: &Condition, context: &gpui::KeyContext) -> bool {
    condition
        .as_ref()
        .is_some_and(|p| p.eval(std::slice::from_ref(context)))
}

/// A [`MultiDiffFileAction`] with its conditions parsed.
struct FileAction {
    action: MultiDiffFileAction,
    when: Option<Condition>,
    toggled: Option<Condition>,
}

impl FileAction {
    fn new(action: MultiDiffFileAction) -> Self {
        Self {
            when: action.when.as_deref().map(parse_condition),
            toggled: action
                .toggled
                .as_ref()
                .map(|t| parse_condition(&t.condition)),
            action,
        }
    }

    /// Whether it shows on a file with `context` (Monaco's menu `when`).
    fn visible(&self, context: &gpui::KeyContext) -> bool {
        self.when.as_ref().is_none_or(|when| holds(when, context))
    }

    /// Whether it's toggled on for a file with `context`; `None` for a
    /// plain button.
    fn toggled(&self, context: &gpui::KeyContext) -> Option<bool> {
        self.toggled
            .as_ref()
            .map(|condition| holds(condition, context))
    }

    /// Whether it tests the file's keys at all.
    fn conditional(&self) -> bool {
        self.when.is_some() || self.toggled.is_some()
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
    /// `expandedContentHeight`, less the header and bottom padding);
    /// `None` until then.
    content_height: Option<Pixels>,
    /// The view state its editor had when it was given back to the pool
    /// (Monaco keeps the selections).
    view_state: Option<DiffEditorViewState>,
    /// Selections from a restored [`MultiDiffEditorViewState`], for when
    /// it next gets an editor (Monaco `lastTemplateData.selections`).
    selections: Option<Vec<Selection>>,
    /// The header, which takes focus in the variants that collapse on a
    /// header click (Monaco's `tabIndex = 0`).
    header_focus: FocusHandle,
    /// The header's chevron, a button in every variant (Monaco's
    /// `collapseButton`).
    chevron_focus: FocusHandle,
    /// A binary file's placeholder, or its "Open Diff" button.
    placeholder_focus: FocusHandle,
}

impl Item {
    /// A new file, expanded and not measured yet.
    fn new(document: MultiDiffDocument, cx: &mut App) -> Self {
        let (status, original, modified) = Self::models(&document, cx);
        Self {
            document,
            status,
            original,
            modified,
            collapsed: false,
            content_height: None,
            view_state: None,
            selections: None,
            header_focus: cx.focus_handle().tab_stop(true),
            chevron_focus: cx.focus_handle().tab_stop(true),
            placeholder_focus: cx.focus_handle().tab_stop(true),
        }
    }

    /// What happened to the file.
    fn status(document: &MultiDiffDocument) -> FileStatus {
        match (&document.original, &document.modified) {
            (None, _) => FileStatus::Added,
            (_, None) => FileStatus::Deleted,
            _ if document
                .original_label
                .as_ref()
                .is_some_and(|original| *original != document.label) =>
            {
                FileStatus::Renamed
            }
            _ => FileStatus::Modified,
        }
    }

    /// The file's status and the models shown, with empty placeholders
    /// for a missing side (the editor's own, not listed models).
    fn models(
        document: &MultiDiffDocument,
        cx: &mut App,
    ) -> (FileStatus, Entity<TextModel>, Entity<TextModel>) {
        let status = Self::status(document);
        let original = document
            .original
            .clone()
            .unwrap_or_else(|| cx.new(|_| TextModel::new_internal("")));
        let modified = document
            .modified
            .clone()
            .unwrap_or_else(|| cx.new(|_| TextModel::new_internal("")));
        (status, original, modified)
    }

    /// The same file's new document, keeping the state.
    fn set_document(&mut self, document: MultiDiffDocument, cx: &mut App) {
        let same_models = self.document.original == document.original
            && self.document.modified == document.modified;
        if same_models {
            self.status = Self::status(&document);
        } else {
            (self.status, self.original, self.modified) = Self::models(&document, cx);
        }
        self.document = document;
    }

    /// Monaco `DiffEditorItemTemplate.size` (and `getUnboundSize` for
    /// binary files).
    fn height(&self, config: &VariantConfiguration) -> Pixels {
        if self.collapsed {
            config.header_height
        } else if self.document.binary {
            config.header_height + config.content_bottom_padding + px(BINARY_PLACEHOLDER_HEIGHT)
        } else {
            config.header_height + self.content_height(config) + config.content_bottom_padding
        }
    }

    /// The diff editor's content height: as measured, else Monaco's
    /// 500px for the whole file less the header and bottom padding
    /// (`DiffEditorItemTemplate.setItem`'s `editorContentHeight`).
    fn content_height(&self, config: &VariantConfiguration) -> Pixels {
        self.content_height.unwrap_or_else(|| {
            (px(INITIAL_EXPANDED_HEIGHT) - config.header_height - config.content_bottom_padding)
                .max(px(0.))
        })
    }

    /// Whether the header or its chevron has focus (Monaco's `trackFocus`
    /// on the header, which sees its chevron's focus too).
    fn header_has_focus(&self, window: &Window) -> bool {
        self.header_focus.is_focused(window) || self.chevron_focus.is_focused(window)
    }

    /// Whether both sides exist (Monaco: the "Open Diff" button shows).
    fn has_both_sides(&self) -> bool {
        self.document.original.is_some() && self.document.modified.is_some()
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
    /// The editors' scroll position the list last set.
    expected_scroll: Point<Pixels>,
    /// Frames left in which a scroll of the editors, from revealing a cursor
    /// moved by the keyboard, scrolls the list along.
    follow_cursor: u8,
    /// The file's context keys set on the editors (Monaco's item
    /// template's scoped keys), removed when unbound.
    context_keys: Vec<SharedString>,
    /// The editors' screen reader support when created, for files that
    /// don't set theirs.
    default_accessibility_support: AccessibilitySupport,
    _subscriptions: Vec<Subscription>,
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

/// A smooth scroll of the list towards its scroll position (Monaco
/// `SmoothScrollingOperation`, easing out over 100ms).
#[derive(Clone, Copy, Debug, PartialEq)]
struct SmoothScroll {
    from: Pixels,
    start: Instant,
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

/// A restored scroll position, per axis until it lands (Monaco
/// `_pendingScrollState`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct PendingScroll {
    top: Option<Pixels>,
    left: Option<Pixels>,
}

/// The per-item state the headers are drawn with.
#[derive(Clone, Copy, Debug)]
struct EntryState {
    /// The file is the active one (Monaco's `.active`).
    active: bool,
    /// Its editor, header or placeholder has focus (`:focus-within`).
    focused: bool,
    /// The list has focus and the file is active (Monaco's
    /// `.multiDiffEditor:focus-within .multiDiffEntry.active`).
    selected: bool,
    /// The mouse is over the header (`.header-hovered`) or it has focus
    /// (`.header-focused`).
    header_highlighted: bool,
    /// The mouse is over the file (`.multiDiffEntry:hover`).
    hovered: bool,
}

/// Monaco `MultiDiffEditorWidget`: diffs of many files in one list.
pub struct MultiDiffEditor {
    items: Vec<Item>,
    options: DiffEditorOptions,
    variant: MultiDiffEditorVariant,
    /// Diff editors bound to files (Monaco's acquired templates).
    bound: Vec<Template>,
    /// Unused diff editors (Monaco `VirtualizedTemplatePool._unused`).
    pool: Vec<Template>,
    /// Empty models for editors in the pool (Monaco `setDiffModel(null)`).
    empty_models: (Entity<TextModel>, Entity<TextModel>),
    /// Monaco `MultiDiffEditorViewModel.activeDiffItem`.
    active: Option<usize>,
    /// The file whose editor, header or placeholder had focus when last
    /// drawn.
    focused_item: Option<usize>,
    /// The file whose header the mouse is over, and the file it's over.
    hovered_header: Option<usize>,
    hovered_entry: Option<usize>,
    file_actions: Vec<FileAction>,
    /// Opens a binary file's diff elsewhere (Monaco
    /// `IWorkbenchUIElementFactory.openDiffEditor`).
    open_binary_diff: Option<MultiDiffFileHandler>,
    pending_reveal: Option<PendingReveal>,
    /// A restored scroll position, applied until the list is tall enough
    /// for it (Monaco `_pendingScrollState`).
    pending_scroll: Option<PendingScroll>,
    /// Where the list scrolls to (Monaco `getFutureScrollPosition`); it
    /// shows `smooth_scroll`'s position on the way.
    scroll_top: Pixels,
    scroll_left: Pixels,
    smooth_scroll: Option<SmoothScroll>,
    anchoring: ScrollAnchoring,
    /// The scroll position last drawn, to reveal the scrollbars when it
    /// changes.
    drawn_scroll: Point<Pixels>,
    /// The files whose header was last drawn with a shadow, for tests.
    shadowed_headers: Vec<usize>,
    scrollbars: [Option<ScrollbarState>; 2],
    scrollbar_drag: Option<ScrollbarDrag>,
    scrollbar_fade: ScrollbarFade,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    focus_handle: FocusHandle,
    /// The documents are loading (Monaco's `'loading'` documents): the
    /// list is empty and says "Loading...".
    loading: bool,
    /// Empty space below the last file (Monaco `setPaddingBottom`).
    padding_bottom: Pixels,
    /// Shows or hides original line numbers in every file, over the files'
    /// and the variant's choice (Monaco `diffEditorOptions`'
    /// `hideOriginalLineNumbers`).
    hide_original_line_numbers: Option<bool>,
    /// Options over every file's own (Monaco's widget `diffEditorOptions`
    /// in `_optionsOverride`).
    options_override: Option<MultiDiffOptionsOverride>,
    /// Don't focus the first change when files load (Monaco
    /// `setPreserveFocusOnLoad`).
    preserve_focus_on_load: bool,
    /// The restored files' states, for files not listed yet (Monaco
    /// `_lastDocStates`): a file that comes later takes its state.
    pending_doc_states: Option<HashMap<MultiDiffDocumentKey, MultiDiffDocState>>,
    /// The restored active file, for when files load (Monaco
    /// `_lastActiveDiffItemKey`); consumed once.
    pending_active_item: Option<MultiDiffDocumentKey>,
    /// A restored active file to focus once drawn (Monaco
    /// `_restorePendingActiveDiffItem`'s `binding.focus()`).
    pending_focus: Option<usize>,
    /// The layout over every file's options (Monaco `_diffLayoutOptions`).
    diff_layout: MultiDiffLayoutOptions,
    /// The diff editor last reported by `ActiveControlChanged`.
    reported_active_control: Option<EntityId>,
    /// The files in view when last drawn (Monaco's bound items, binary
    /// files included).
    drawn_items: Vec<usize>,
}

impl EventEmitter<MultiDiffEditorEvent> for MultiDiffEditor {}

impl Focusable for MultiDiffEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl MultiDiffEditor {
    /// A multi-diff editor showing `documents`, in the
    /// [`MultiDiffEditorVariant::NoCardsNonCompact`] variant as standalone
    /// Monaco's, at the first change and with it focused (see
    /// [`MultiDiffEditor::set_documents`]). With no documents, it says "No
    /// Changed Files".
    pub fn new(
        documents: Vec<MultiDiffDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            items: Vec::new(),
            options: DiffEditorOptions::default(),
            variant: MultiDiffEditorVariant::default(),
            bound: Vec::new(),
            pool: Vec::new(),
            empty_models: (
                cx.new(|_| TextModel::new_internal("")),
                cx.new(|_| TextModel::new_internal("")),
            ),
            active: None,
            focused_item: None,
            hovered_header: None,
            hovered_entry: None,
            file_actions: Vec::new(),
            open_binary_diff: None,
            pending_reveal: None,
            pending_scroll: None,
            scroll_top: px(0.),
            scroll_left: px(0.),
            smooth_scroll: None,
            anchoring: ScrollAnchoring::default(),
            drawn_scroll: Point::default(),
            shadowed_headers: Vec::new(),
            scrollbars: [None, None],
            scrollbar_drag: None,
            scrollbar_fade: ScrollbarFade::default(),
            bounds: Default::default(),
            focus_handle: cx.focus_handle(),
            loading: false,
            padding_bottom: px(0.),
            hide_original_line_numbers: None,
            options_override: None,
            preserve_focus_on_load: false,
            pending_doc_states: None,
            pending_active_item: None,
            pending_focus: None,
            diff_layout: MultiDiffLayoutOptions::default(),
            reported_active_control: None,
            drawn_items: Vec::new(),
        };
        this.set_documents(documents, window, cx);
        this
    }

    /// Updates the files (Monaco's observable `IMultiDiffEditorModel.documents`,
    /// mapped with `mapObservableArrayCached`). A file with the
    /// [`key`](MultiDiffDocument::key) of one shown keeps its collapsed
    /// state, view state, measured height and editor, and the list keeps
    /// its scroll position; if its models changed, it keeps its collapsed
    /// state and selections only, as its revealed regions were for the
    /// old text. Other files start expanded, or with their part of a view
    /// state restored before (see [`MultiDiffEditor::restore_view_state`]).
    /// Ends [`MultiDiffEditor::set_documents_loading`].
    ///
    /// When no file is active (as when the first files load), it makes a
    /// restored view state's active file active, else goes to the first
    /// change (Monaco's "Initialize first change"), focusing it unless
    /// [`MultiDiffEditor::set_preserve_focus_on_load`] says not to.
    pub fn set_documents(
        &mut self,
        documents: Vec<MultiDiffDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.loading = false;
        self.replace_documents(documents, cx);
        // Monaco's "Initialize first change" autorun, after a restored
        // active file.
        if !self.items.is_empty() && self.active.is_none() && !self.restore_pending_active_item(cx)
        {
            self.navigate_to_change(true, !self.preserve_focus_on_load, window, cx);
        }
        self.report_active_control(cx);
    }

    /// Empties the list until the next [`MultiDiffEditor::set_documents`],
    /// saying "Loading..." (Monaco's `'loading'` documents). Like Monaco,
    /// the files shown are dropped with their state; a view state restored
    /// before or while loading waits for the files.
    pub fn set_documents_loading(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.replace_documents(Vec::new(), cx);
        self.report_active_control(cx);
    }

    /// Whether the documents are loading (Monaco `isLoading`).
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Monaco `setPreserveFocusOnLoad`: whether going to the first change
    /// when files load leaves the focus where it is (default `false`: it
    /// focuses the change, as Monaco's `setViewModel` without
    /// `preserveFocus`).
    pub fn set_preserve_focus_on_load(&mut self, preserve_focus: bool) {
        self.preserve_focus_on_load = preserve_focus;
    }

    /// The files' new list, keyed by [`MultiDiffDocument::key`]: kept files
    /// keep their state and editors, removed ones give their editors back.
    fn replace_documents(&mut self, documents: Vec<MultiDiffDocument>, cx: &mut Context<Self>) {
        let old_keys: Vec<MultiDiffDocumentKey> =
            self.items.iter().map(|item| item.document.key()).collect();
        let mut old_items: Vec<Option<Item>> = std::mem::take(&mut self.items)
            .into_iter()
            .map(Some)
            .collect();
        let mut by_key: HashMap<&MultiDiffDocumentKey, VecDeque<usize>> = HashMap::new();
        for (index, key) in old_keys.iter().enumerate() {
            by_key.entry(key).or_default().push_back(index);
        }
        // Each old file's new index, and the kept files whose models
        // changed.
        let mut moved_to = vec![None; old_keys.len()];
        let mut new_models = Vec::new();
        let mut new_keys = Vec::with_capacity(documents.len());
        for document in documents {
            let key = document.key();
            let old = by_key.get_mut(&key).and_then(|indices| indices.pop_front());
            let item = match old {
                Some(index) => {
                    let mut item = old_items[index].take().expect("each file is kept once");
                    moved_to[index] = Some(self.items.len());
                    if item.document.original != document.original
                        || item.document.modified != document.modified
                        || item.document.binary != document.binary
                    {
                        new_models.push(self.items.len());
                    }
                    item.set_document(document, cx);
                    item
                }
                None => {
                    let mut item = Item::new(document, cx);
                    // Monaco applies `_lastDocStates` to each new item.
                    if let Some(state) = self
                        .pending_doc_states
                        .as_ref()
                        .and_then(|states| states.get(&key))
                    {
                        item.collapsed = state.collapsed;
                        item.selections = state.selections.clone();
                    }
                    item
                }
            };
            new_keys.push(key);
            self.items.push(item);
        }
        // Editors follow their files; a removed file's go back to the pool,
        // and a file showing other models gets an editor anew.
        let mut ix = 0;
        while ix < self.bound.len() {
            match moved_to[self.bound[ix].item] {
                Some(index) if !new_models.contains(&index) => {
                    self.bound[ix].item = index;
                    self.configure_template(ix, cx);
                    ix += 1;
                }
                Some(index) => {
                    self.bound[ix].item = index;
                    self.unbind(ix, true, cx);
                }
                None => self.unbind(ix, false, cx),
            }
        }
        // Monaco gives new documents a new view model: the revealed
        // regions and scroll position were for the old text. The collapsed
        // state and selections stay, as the file is still the same one.
        for &index in &new_models {
            let item = &mut self.items[index];
            if let Some(state) = item.view_state.take() {
                item.selections.get_or_insert(state.modified.selections);
            }
        }
        let remap = |index: Option<usize>| index.and_then(|index| moved_to.get(index).copied()?);
        // Monaco `activeDiffItem` keeps its last value while it's listed.
        self.active = remap(self.active);
        self.focused_item = remap(self.focused_item);
        self.pending_focus = remap(self.pending_focus);
        self.pending_reveal = self.pending_reveal.and_then(|pending| {
            Some(PendingReveal {
                item: remap(Some(pending.item))?,
                ..pending
            })
        });
        self.hovered_header = None;
        self.hovered_entry = None;
        // Monaco `CompressedVirtualizedScrollView._update`: other items
        // drop the scroll slack instead of anchoring.
        if old_keys != new_keys {
            self.anchoring.items_changed();
        }
        cx.notify();
    }

    /// The variant (Monaco `IMultiDiffEditorWidgetOptions.variant`).
    pub fn variant(&self) -> MultiDiffEditorVariant {
        self.variant
    }

    /// Switches to another variant (Monaco `setVariant`), keeping the view
    /// state.
    pub fn set_variant(&mut self, variant: MultiDiffEditorVariant, cx: &mut Context<Self>) {
        if self.variant == variant {
            return;
        }
        self.variant = variant;
        self.configure_bound_templates(cx);
        self.hovered_header = None;
        cx.notify();
    }

    fn config(&self) -> VariantConfiguration {
        self.variant.configuration()
    }

    /// A diff editor for the pool, with Monaco's item template options:
    /// the list scrolls, not the editors.
    fn create_template(&self, window: &mut Window, cx: &mut Context<Self>) -> Template {
        let options = self.item_options(None);
        let (original, modified) = self.empty_models.clone();
        let diff = cx.new(|cx| {
            let mut diff = DiffEditor::new(original, modified, window, cx);
            diff.multi_diff_all_collapsed = Some(false);
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
        let mut subscriptions =
            vec![
                cx.subscribe(&diff, |this, diff, event: &DiffEditorEvent, cx| {
                    if matches!(
                        event,
                        DiffEditorEvent::DiffUpdated | DiffEditorEvent::ModelChanged
                    ) && let Some(template) = this.bound.iter_mut().find(|t| t.diff == diff)
                    {
                        template.stale = true;
                    }
                    cx.notify();
                }),
            ];
        let editors = {
            let diff = diff.read(cx);
            [
                diff.original_editor().clone(),
                diff.modified_editor().clone(),
            ]
        };
        let default_accessibility_support = editors[1].read(cx).options().accessibility_support;
        for editor in editors {
            let diff = diff.downgrade();
            subscriptions.push(cx.subscribe(
                &editor,
                move |this, editor, event: &CodeEditorEvent, cx| {
                    if let Some(diff) = diff.upgrade() {
                        this.on_item_editor_event(&diff, &editor, event, cx);
                    }
                },
            ));
        }
        Template {
            diff,
            item: 0,
            stale: true,
            expected_scroll: Point::default(),
            follow_cursor: 0,
            context_keys: Vec::new(),
            default_accessibility_support,
            _subscriptions: subscriptions,
        }
    }

    /// Keeps a cursor moved by the keyboard in view: the editor reveals it
    /// by scrolling itself (it's as tall as what shows of its file), and
    /// the list scrolls along, as Monaco's list keeps the editor's own
    /// scroll position until it next lays the file out.
    fn on_item_editor_event(
        &mut self,
        diff: &Entity<DiffEditor>,
        editor: &Entity<CodeEditor>,
        event: &CodeEditorEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(template) = self.bound.iter_mut().find(|t| t.diff == *diff) else {
            return;
        };
        match event {
            CodeEditorEvent::SelectionsChanged(change)
                if change.source == CursorChangeSource::Keyboard =>
            {
                // The editor reveals the cursor in its next layout, after
                // the list's next render.
                template.follow_cursor = 2;
            }
            CodeEditorEvent::ScrollChanged(_) if template.follow_cursor > 0 => {
                // Where the reveal is going, not a smooth scroll's first
                // step (Monaco's `getFutureScrollPosition`): the list takes
                // the whole move at once.
                let scroll = editor.read(cx).future_scroll_position();
                let delta = scroll - template.expected_scroll;
                if delta.x.abs() < px(0.5) && delta.y.abs() < px(0.5) {
                    return;
                }
                // The other editor follows with the same scroll.
                template.follow_cursor = 0;
                template.expected_scroll = scroll;
                self.scroll_top = (self.scroll_top + delta.y).max(px(0.));
                self.scroll_left = (self.scroll_left + delta.x).max(px(0.));
                self.smooth_scroll = None;
                self.pending_scroll = None;
                self.pending_reveal = None;
                cx.notify();
            }
            _ => {}
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
        template.item = index;
        template.stale = true;
        template.follow_cursor = 0;
        self.bound.push(template);
        // Monaco applies the file's options before its models.
        self.configure_template(self.bound.len() - 1, cx);
        let item = &mut self.items[index];
        let (original, modified) = (item.original.clone(), item.modified.clone());
        let view_state = item.view_state.clone();
        // Monaco `onDidBind`: the selections it had.
        let selections = item.selections.take();
        let diff = self.bound[self.bound.len() - 1].diff.clone();
        diff.update(cx, |diff, cx| {
            diff.set_models(original, modified, cx);
            if let Some(state) = &view_state {
                diff.restore_view_state(state, cx);
            }
            if let Some(selections) = selections.filter(|s| !s.is_empty()) {
                diff.modified_editor()
                    .update(cx, |editor, cx| editor.set_selections(&selections, cx));
            }
        });
    }

    /// Gives `self.bound[ix]`'s editor its file's options, editor options,
    /// read-only sides and context keys (Monaco
    /// `DiffEditorItemTemplate.setItem`'s `updateOptions` and
    /// `contextKeys`). Editor options the file doesn't set go back to the
    /// list's defaults, so none carry over from the editor's last file.
    fn configure_template(&mut self, ix: usize, cx: &mut Context<Self>) {
        let index = self.bound[ix].item;
        let item = &self.items[index];
        let options = self.item_options(Some(&item.document));
        let deleted = item.status == FileStatus::Deleted;
        let original_read_only = deleted || !options.original_editable;
        let editor_options = &item.document.editor_options;
        let modified_read_only = deleted || editor_options.read_only.unwrap_or(false);
        let keys = item.document.context_keys.clone();
        let template = &mut self.bound[ix];
        let accessibility_support = editor_options
            .accessibility_support
            .unwrap_or(template.default_accessibility_support);
        let stale_keys = std::mem::take(&mut template.context_keys);
        template.context_keys = keys.iter().map(|(key, _)| key.clone()).collect();
        template.diff.update(cx, |diff, cx| {
            if *diff.options() != options {
                diff.update_options(options, cx);
            }
            for (editor, read_only) in [
                (diff.original_editor().clone(), original_read_only),
                (diff.modified_editor().clone(), modified_read_only),
            ] {
                editor.update(cx, |editor, cx| {
                    let current = editor.options();
                    if current.read_only != read_only
                        || current.accessibility_support != accessibility_support
                    {
                        let mut options = current.clone();
                        options.read_only = read_only;
                        options.accessibility_support = accessibility_support;
                        editor.update_options(options, cx);
                    }
                    if !stale_keys.is_empty() || !keys.is_empty() {
                        set_host_keys(editor, &stale_keys, &keys, cx);
                    }
                });
            }
        });
    }

    /// [`Self::configure_template`] for every bound editor.
    fn configure_bound_templates(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.bound.len() {
            self.configure_template(ix, cx);
        }
    }

    /// Monaco `ManagedVirtualizedItem._clearBinding`: gives `self.bound[ix]`
    /// back to the pool, keeping the file's view state (Monaco
    /// `onWillUnbind`). The pool keeps up to five editors and drops the
    /// rest.
    fn unbind(&mut self, ix: usize, save: bool, cx: &mut Context<Self>) {
        let mut template = self.bound.remove(ix);
        if save && let Some(item) = self.items.get_mut(template.item) {
            item.view_state = Some(template.diff.read(cx).save_view_state(cx));
        }
        let keys = std::mem::take(&mut template.context_keys);
        if !keys.is_empty() {
            let diff = template.diff.read(cx);
            for editor in [
                diff.original_editor().clone(),
                diff.modified_editor().clone(),
            ] {
                editor.update(cx, |editor, cx| set_host_keys(editor, &keys, &[], cx));
            }
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

    /// Binds file `index` now, for navigation; `None` for a binary file,
    /// which has no editor.
    fn ensure_bound(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<DiffEditor>> {
        if self.items[index].document.binary {
            return None;
        }
        if !self.bound.iter().any(|t| t.item == index) {
            self.bind(index, window, cx);
        }
        self.bound
            .iter()
            .find(|t| t.item == index)
            .map(|t| t.diff.clone())
    }

    /// Monaco's item options (`DiffEditorItemTemplate.setItem`): the
    /// file's options, else the list's, then the list's override and
    /// layout (`_optionsOverride`), with unchanged regions hidden and no
    /// overview ruler. Original line numbers follow the list's choice, else
    /// the file's, else the variant (Monaco's `override` before
    /// `options.hideOriginalLineNumbers` before the variant).
    fn item_options(&self, document: Option<&MultiDiffDocument>) -> DiffEditorOptions {
        let mut options = document
            .and_then(|document| document.options.clone())
            .unwrap_or_else(|| self.options.clone());
        if let Some(options_override) = &self.options_override {
            options_override(&mut options);
        }
        let layout = self.diff_layout;
        if let Some(side_by_side) = layout.render_side_by_side {
            options.render_side_by_side = side_by_side;
        }
        if let Some(inline_when_limited) = layout.use_inline_view_when_space_is_limited {
            options.use_inline_view_when_space_is_limited = inline_when_limited;
        }
        if let Some(word_wrap) = layout.diff_word_wrap {
            options.diff_word_wrap = word_wrap;
        }
        options.hide_unchanged_regions.enabled = true;
        options.render_overview_ruler = false;
        options.hide_original_line_numbers = self
            .hide_original_line_numbers
            .or(document.and_then(|document| document.hide_original_line_numbers))
            .unwrap_or(
                options.hide_original_line_numbers || self.config().hide_original_line_numbers,
            );
        options
    }

    /// The options of the files without their own
    /// ([`MultiDiffDocument::options`]); unchanged regions are always
    /// hidden.
    pub fn options(&self) -> &DiffEditorOptions {
        &self.options
    }

    /// Replaces the options of the files without their own. To change
    /// options in every file, over their own (Monaco's widget-level
    /// `diffEditorOptions`), use [`MultiDiffEditor::set_options_override`].
    pub fn update_options(&mut self, options: DiffEditorOptions, cx: &mut Context<Self>) {
        self.options = options;
        self.configure_bound_templates(cx);
        cx.notify();
    }

    /// Sets the options every file takes over its own (Monaco's widget
    /// `diffEditorOptions`, which `_optionsOverride` spreads after each
    /// file's options): `options_override` changes each file's options
    /// (its [`MultiDiffDocument::options`], else
    /// [`MultiDiffEditor::options`]) before the list's layout
    /// ([`MultiDiffEditor::diff_layout_options`]) applies. `None` removes
    /// it.
    pub fn set_options_override(
        &mut self,
        options_override: Option<MultiDiffOptionsOverride>,
        cx: &mut Context<Self>,
    ) {
        self.options_override = options_override;
        self.configure_bound_templates(cx);
        cx.notify();
    }

    /// Whether every file shows (`Some(false)`) or hides (`Some(true)`)
    /// its original line numbers in the inline view, over the files' and
    /// the variant's choice (Monaco's widget `diffEditorOptions`); `None`
    /// leaves it to them.
    pub fn hide_original_line_numbers(&self) -> Option<bool> {
        self.hide_original_line_numbers
    }

    /// Sets [`MultiDiffEditor::hide_original_line_numbers`].
    pub fn set_hide_original_line_numbers(&mut self, hide: Option<bool>, cx: &mut Context<Self>) {
        self.hide_original_line_numbers = hide;
        self.configure_bound_templates(cx);
        cx.notify();
    }

    /// The layout every file's diff takes over its own options (Monaco
    /// `_diffLayoutOptions`); none until set.
    pub fn diff_layout_options(&self) -> MultiDiffLayoutOptions {
        self.diff_layout
    }

    /// Monaco `setRenderSideBySide`: every file side by side (`true`) or
    /// inline (`false`), whatever its own options say. Side by side, it
    /// shows inline when space is limited only with
    /// `use_inline_view_when_space_is_limited` (Monaco's option, default
    /// `false`).
    pub fn set_render_side_by_side(
        &mut self,
        render_side_by_side: bool,
        use_inline_view_when_space_is_limited: bool,
        cx: &mut Context<Self>,
    ) {
        self.update_diff_layout(
            MultiDiffLayoutOptions {
                render_side_by_side: Some(render_side_by_side),
                use_inline_view_when_space_is_limited: Some(use_inline_view_when_space_is_limited),
                diff_word_wrap: None,
            },
            cx,
        );
    }

    /// Monaco `toggleRenderSideBySide`: inline if every file shows side by
    /// side (as when none was set), else side by side.
    pub fn toggle_render_side_by_side(&mut self, cx: &mut Context<Self>) {
        let side_by_side = self.diff_layout.render_side_by_side.unwrap_or(true);
        self.set_render_side_by_side(!side_by_side, false, cx);
    }

    /// Monaco `setViewMode`: [`MultiDiffEditor::set_diff_layout_options`]
    /// without a word wrap.
    pub fn set_view_mode(&mut self, mode: DiffEditorViewMode, cx: &mut Context<Self>) {
        self.set_diff_layout_options(mode, None, cx);
    }

    /// Monaco `setDiffLayoutOptions`: every file in `mode`, and with
    /// `diff_word_wrap` if given. (Monaco also ends a diff editor's
    /// temporary inline view on switching to automatic; the port's diff
    /// editor has none.)
    pub fn set_diff_layout_options(
        &mut self,
        mode: DiffEditorViewMode,
        diff_word_wrap: Option<WordWrapOverride>,
        cx: &mut Context<Self>,
    ) {
        self.update_diff_layout(
            MultiDiffLayoutOptions {
                render_side_by_side: Some(mode != DiffEditorViewMode::Inline),
                use_inline_view_when_space_is_limited: Some(mode == DiffEditorViewMode::Automatic),
                diff_word_wrap,
            },
            cx,
        );
    }

    /// Monaco `setDiffWordWrap`: every file's
    /// [`diff_word_wrap`](DiffEditorOptions::diff_word_wrap);
    /// [`WordWrapOverride::Inherit`] leaves the editors' own.
    pub fn set_diff_word_wrap(&mut self, diff_word_wrap: WordWrapOverride, cx: &mut Context<Self>) {
        self.update_diff_layout(
            MultiDiffLayoutOptions {
                diff_word_wrap: Some(diff_word_wrap),
                ..MultiDiffLayoutOptions::default()
            },
            cx,
        );
    }

    /// Monaco `_updateDiffLayoutOptions`: merges the given fields into the
    /// layout, and applies it to the files' editors if it changed.
    fn update_diff_layout(&mut self, update: MultiDiffLayoutOptions, cx: &mut Context<Self>) {
        let layout = MultiDiffLayoutOptions {
            render_side_by_side: update
                .render_side_by_side
                .or(self.diff_layout.render_side_by_side),
            use_inline_view_when_space_is_limited: update
                .use_inline_view_when_space_is_limited
                .or(self.diff_layout.use_inline_view_when_space_is_limited),
            diff_word_wrap: update.diff_word_wrap.or(self.diff_layout.diff_word_wrap),
        };
        if layout == self.diff_layout {
            return;
        }
        self.diff_layout = layout;
        self.configure_bound_templates(cx);
        cx.notify();
    }

    /// File `index`'s document.
    pub fn document(&self, index: usize) -> Option<&MultiDiffDocument> {
        self.items.get(index).map(|item| &item.document)
    }

    /// Changes file `index`'s own options (Monaco
    /// `IDocumentDiffItem.onOptionsDidChange`): its
    /// [`options`](MultiDiffDocument::options) and
    /// [`hide_original_line_numbers`](MultiDiffDocument::hide_original_line_numbers).
    /// Its editor, if it has one, takes them at once.
    pub fn set_document_options(
        &mut self,
        index: usize,
        options: Option<DiffEditorOptions>,
        hide_original_line_numbers: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.items.get_mut(index) else {
            return;
        };
        item.document.options = options;
        item.document.hide_original_line_numbers = hide_original_line_numbers;
        if let Some(ix) = self.bound.iter().position(|t| t.item == index) {
            self.configure_template(ix, cx);
        }
        cx.notify();
    }

    /// Changes file `index`'s
    /// [`editor_options`](MultiDiffDocument::editor_options) (Monaco
    /// `onOptionsDidChange`). Its editors, if it has them, take them at
    /// once.
    pub fn set_document_editor_options(
        &mut self,
        index: usize,
        editor_options: MultiDiffDocumentEditorOptions,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.items.get_mut(index) else {
            return;
        };
        item.document.editor_options = editor_options;
        if let Some(ix) = self.bound.iter().position(|t| t.item == index) {
            self.configure_template(ix, cx);
        }
        cx.notify();
    }

    /// Reserves empty space below the last file (Monaco
    /// `setPaddingBottom`), as part of the scrollable height.
    pub fn set_padding_bottom(&mut self, padding: Pixels, cx: &mut Context<Self>) {
        let padding = padding.max(px(0.));
        if padding != self.padding_bottom {
            self.padding_bottom = padding;
            // Monaco's padding item is a new item for each size.
            self.anchoring.items_changed();
            cx.notify();
        }
    }

    /// The space below the last file.
    pub fn padding_bottom(&self) -> Pixels {
        self.padding_bottom
    }

    /// The active file's diff editor (Monaco `getActiveControl`), if it
    /// has one: only files in view (and a focused one) do, collapsed or
    /// not, and binary files never do.
    pub fn active_control(&self) -> Option<&Entity<DiffEditor>> {
        self.diff_editor(self.active?)
    }

    /// Focuses the active file (Monaco `focus`): its modified editor, or a
    /// binary file's placeholder (or "Open Diff" button). `false`, leaving
    /// the focus, when there's no active file or it isn't in view. A
    /// collapsed file's editor is hidden and can't take the focus, as in
    /// Monaco, but it still returns `true`.
    pub fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.active.filter(|&index| index < self.items.len()) else {
            return false;
        };
        let item = &self.items[index];
        if item.document.binary {
            if !self.drawn_items.contains(&index) {
                return false;
            }
            if !item.collapsed {
                window.focus(&item.placeholder_focus, cx);
            }
            return true;
        }
        let Some(diff) = self.diff_editor(index) else {
            return false;
        };
        if !item.collapsed {
            let modified = diff.read(cx).modified_editor().focus_handle(cx);
            window.focus(&modified, cx);
        }
        true
    }

    /// The list's context keys, which GPUI sets on its element and its
    /// files' editors inherit: `inMultiDiffEditor`,
    /// `multiDiffEditorAllCollapsed`, `multiDiffEditorRenderSideBySide`
    /// (the list's layout shows side by side, true until set otherwise)
    /// and `diffEditorAutomaticRenderSideBySide` (the active file's, true
    /// without one), as Monaco's widget sets them.
    pub fn context_keys(&self, cx: &App) -> gpui::KeyContext {
        let mut key_context = gpui::KeyContext::new_with_defaults();
        key_context.add("inMultiDiffEditor");
        if self.items.iter().all(|item| item.collapsed) {
            key_context.add("multiDiffEditorAllCollapsed");
        }
        if self.diff_layout.render_side_by_side.unwrap_or(true) {
            key_context.add("multiDiffEditorRenderSideBySide");
        }
        if self
            .active_control()
            .is_none_or(|diff| diff.read(cx).renders_side_by_side_in_automatic_mode())
        {
            key_context.add("diffEditorAutomaticRenderSideBySide");
        }
        key_context
    }

    /// Emits `ActiveControlChanged` when the active file's diff editor
    /// changed since it was last reported.
    fn report_active_control(&mut self, cx: &mut Context<Self>) {
        let control = self.active_control().map(|diff| diff.entity_id());
        if control != self.reported_active_control {
            self.reported_active_control = control;
            cx.emit(MultiDiffEditorEvent::ActiveControlChanged);
        }
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
    /// view (and a focused one) have an editor, collapsed or not (hidden
    /// then), and binary files never do (Monaco `tryGetCodeEditor`).
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
    /// revealed, navigated to or collapsed with its chevron or header. It's
    /// outlined (standard variant) or highlighted (compact variants).
    pub fn active_item(&self) -> Option<usize> {
        self.active
    }

    /// Adds a button to the file headers' toolbar (Monaco's
    /// `MultiDiffEditorFileToolbar` menu): every file's, or those its
    /// [`when`](MultiDiffFileAction::when) holds for. One with the same id
    /// is replaced. The compact variants show the toolbar only while the
    /// mouse is over the file or it has focus.
    pub fn add_file_action(&mut self, action: MultiDiffFileAction, cx: &mut Context<Self>) {
        self.file_actions.retain(|a| a.action.id != action.id);
        self.file_actions.push(FileAction::new(action));
        cx.notify();
    }

    /// The keys file `index`'s toolbar tests (Monaco's context key scope
    /// for the item's actions): the list's
    /// ([`MultiDiffEditor::context_keys`]), the file's
    /// [`MultiDiffDocument::context_keys`] and
    /// `multiDiffEditorItemAllUnchangedRegionsShown` (Monaco
    /// `allUnchangedRegionsShown` of the file's diff editor; true for a file
    /// without one, which has no unchanged regions).
    pub fn file_context_keys(&self, index: usize, cx: &App) -> gpui::KeyContext {
        let mut context = self.context_keys(cx);
        let Some(item) = self.items.get(index) else {
            return context;
        };
        let all_shown = self
            .bound
            .iter()
            .find(|t| t.item == index)
            .is_none_or(|t| t.diff.read(cx).all_unchanged_regions_shown());
        if all_shown {
            context.add("multiDiffEditorItemAllUnchangedRegionsShown");
        }
        for (key, value) in &item.document.context_keys {
            value.apply(key, &mut context);
        }
        context
    }

    /// The ids of the actions file `index`'s toolbar shows (their `when`
    /// holds), each with whether it's toggled on (`None` for a plain
    /// button).
    pub fn visible_file_actions(
        &self,
        index: usize,
        cx: &App,
    ) -> Vec<(SharedString, Option<bool>)> {
        let context = self.file_context_keys(index, cx);
        self.file_actions
            .iter()
            .filter(|a| a.visible(&context))
            .map(|a| (a.action.id.clone(), a.toggled(&context)))
            .collect()
    }

    /// Removes a [`MultiDiffEditor::add_file_action`] button; `false` if
    /// there was none with this id.
    pub fn remove_file_action(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let before = self.file_actions.len();
        self.file_actions.retain(|a| a.action.id != id);
        cx.notify();
        self.file_actions.len() != before
    }

    /// Shows an "Open Diff" button on binary files with both sides, which
    /// runs `handler` (Monaco `IWorkbenchUIElementFactory.openDiffEditor`;
    /// standalone Monaco has none). `None` removes it.
    pub fn set_open_binary_diff_handler(
        &mut self,
        handler: Option<MultiDiffFileHandler>,
        cx: &mut Context<Self>,
    ) {
        self.open_binary_diff = handler;
        cx.notify();
    }

    /// The vertical scroll position: where the list is going during a
    /// smooth scroll (Monaco `getFutureScrollPosition`). It includes any
    /// room left above the first file by the anchoring (Monaco
    /// `leadingScrollSlack`).
    pub fn scroll_top(&self) -> Pixels {
        self.scroll_top
    }

    /// Scrolls the list at once (clamped when next drawn). It wins over a
    /// change still waiting to be revealed, such as the first change the
    /// list goes to when files load: Monaco reveals that at once, so a
    /// later scroll comes after it.
    pub fn set_scroll_top(&mut self, scroll_top: Pixels, cx: &mut Context<Self>) {
        self.scroll_top = scroll_top.max(px(0.));
        self.smooth_scroll = None;
        self.pending_scroll = None;
        self.pending_reveal = None;
        cx.notify();
    }

    /// The horizontal scroll position, shared by every file.
    pub fn scroll_left(&self) -> Pixels {
        self.scroll_left
    }

    /// Scrolls every file sideways (clamped when next drawn).
    pub fn set_scroll_left(&mut self, scroll_left: Pixels, cx: &mut Context<Self>) {
        self.scroll_left = scroll_left.max(px(0.));
        self.pending_scroll = None;
        cx.notify();
    }

    /// The scrollable height: the files, the bottom padding and the
    /// anchoring's room above and below them (Monaco's layout
    /// `scrollHeight`).
    pub fn scroll_height(&self) -> Pixels {
        self.anchoring.scroll_height(&self.layout_heights())
    }

    /// Whether a smooth scroll is on its way (Monaco
    /// `hasPendingScrollAnimation`).
    pub fn has_pending_scroll_animation(&self) -> bool {
        self.smooth_scroll.is_some()
    }

    /// The whole list's view state (Monaco `getViewState`): the scroll
    /// position, each file's collapsed state and selections, and the
    /// active file.
    pub fn save_view_state(&self, cx: &App) -> MultiDiffEditorViewState {
        let doc_states = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let selections = match self.diff_editor(index) {
                    Some(diff) => Some(diff.read(cx).modified_editor().read(cx).selections()),
                    None => item.selections.clone().or_else(|| {
                        item.view_state
                            .as_ref()
                            .map(|state| state.modified.selections.clone())
                    }),
                };
                (
                    item.document.key(),
                    MultiDiffDocState {
                        collapsed: item.collapsed,
                        selections,
                    },
                )
            })
            .collect();
        MultiDiffEditorViewState {
            scroll_top: self.scroll_top,
            scroll_left: self.scroll_left,
            doc_states,
            active_item: self
                .active
                .and_then(|index| self.items.get(index))
                .map(|item| item.document.key()),
        }
    }

    /// Restores a [`MultiDiffEditor::save_view_state`] (Monaco
    /// `setViewState`) onto the files with the same keys. The scroll
    /// position is applied once the list is tall enough for it (Monaco
    /// `_applyPendingScrollState`), as files get measured.
    ///
    /// Like Monaco, it can come before or with the files: files listed
    /// later (by [`MultiDiffEditor::set_documents`], after
    /// [`MultiDiffEditor::set_documents_loading`] or in the same update)
    /// take their part of it, and its active file, once listed, becomes
    /// active in place of going to the first change. The active file gets
    /// the focus unless [`MultiDiffEditor::set_preserve_focus_on_load`]
    /// says not to (when next drawn, unless collapsed). Until
    /// [`MultiDiffEditor::clear_pending_restoration_state`], files that
    /// show up later still take their state from it.
    pub fn restore_view_state(&mut self, state: &MultiDiffEditorViewState, cx: &mut Context<Self>) {
        self.pending_doc_states = Some(state.doc_states.clone());
        self.pending_active_item = state.active_item.clone();
        for index in 0..self.items.len() {
            let Some(doc) = state.doc_states.get(&self.items[index].document.key()) else {
                continue;
            };
            self.set_collapsed(index, doc.collapsed, cx);
            let Some(selections) = doc.selections.clone() else {
                continue;
            };
            match self.diff_editor(index).cloned() {
                Some(diff) => {
                    // The restored selections win over a first change still
                    // waiting for its diff.
                    diff.update(cx, |diff, _| diff.cancel_reveal_first_diff());
                    let editor = diff.read(cx).modified_editor().clone();
                    editor.update(cx, |editor, cx| editor.set_selections(&selections, cx));
                }
                None => {
                    let item = &mut self.items[index];
                    if let Some(view_state) = &mut item.view_state {
                        view_state.modified.selections = selections.clone();
                    }
                    item.selections = Some(selections);
                }
            }
        }
        self.pending_reveal = None;
        self.smooth_scroll = None;
        self.scroll_top = state.scroll_top.max(px(0.));
        self.scroll_left = state.scroll_left.max(px(0.));
        self.pending_scroll = Some(PendingScroll {
            top: Some(self.scroll_top),
            left: Some(self.scroll_left),
        });
        // With the files listed, the active file now (over a first change
        // gone to); else when they load.
        self.restore_pending_active_item(cx);
        self.report_active_control(cx);
        cx.notify();
    }

    /// Monaco `clearPendingRestorationState`: forgets what a
    /// [`MultiDiffEditor::restore_view_state`] left for files not listed
    /// yet (their states, the active file and a scroll position not
    /// reached yet), so files loaded next start fresh.
    pub fn clear_pending_restoration_state(&mut self) {
        self.pending_doc_states = None;
        self.pending_active_item = None;
        self.pending_scroll = None;
    }

    /// Monaco `_restorePendingActiveDiffItem`: makes the restored active
    /// file active once it's listed, to be focused unless
    /// `preserve_focus_on_load`. Whether it did; the key is used once, found
    /// or not.
    fn restore_pending_active_item(&mut self, cx: &mut Context<Self>) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let Some(key) = self.pending_active_item.take() else {
            return false;
        };
        let Some(index) = self.items.iter().position(|i| i.document.key() == key) else {
            return false;
        };
        self.active = Some(index);
        if !self.preserve_focus_on_load {
            self.pending_focus = Some(index);
        }
        cx.notify();
        true
    }

    /// Makes file `index` active and scrolls its header to the top (Monaco
    /// `reveal`).
    pub fn reveal(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.items.len() {
            self.active = Some(index);
        }
        let top = self.item_top(index.min(self.items.len()));
        // Monaco `setLogicalScrollPosition`.
        self.set_scroll_top(self.anchoring.leading_slack + top, cx);
    }

    /// Reveals file `index` and, once its diff is shown, centers `range`
    /// of its original or modified side, highlighting it for 350ms (Monaco
    /// `reveal` with a `range`, and `highlightRange`).
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
        let Some(diff) = self.ensure_bound(index, window, cx) else {
            return;
        };
        let editor = if original {
            diff.read(cx).original_editor().clone()
        } else {
            diff.read(cx).modified_editor().clone()
        };
        highlight_range(&editor, range.clone(), cx);
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
    /// and focusing its modified editor (a binary file's placeholder).
    pub fn go_to_next_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_to_change(true, true, window, cx);
    }

    /// Monaco `goToPreviousChange`: the previous change of the active
    /// file, else the last change of the previous file.
    pub fn go_to_previous_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_to_change(false, true, window, cx);
    }

    /// Goes to file `index`'s first change (Monaco `_goToFile`): expands
    /// and reveals it, puts the modified editor's cursor on the change and
    /// focuses it.
    pub fn go_to_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_file_at(index, true, true, window, cx);
    }

    /// Monaco `_navigateToChange`; `focus` focuses the file's editor
    /// when it goes to another file.
    fn navigate_to_change(
        &mut self,
        next: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.items.is_empty() {
            return;
        }
        let Some(current) = self.active.filter(|&ix| ix < self.items.len()) else {
            self.go_to_file_at(0, true, focus, window, cx);
            return;
        };
        self.set_collapsed(current, false, cx);
        // A binary file has no changes to go to.
        if let Some(diff) = self.ensure_bound(current, window, cx) {
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
                diff.update(cx, |diff, cx| diff.go_to_diff(next, cx));
                self.pending_reveal = Some(PendingReveal {
                    item: current,
                    target: RevealTarget::Cursor,
                });
                cx.notify();
                return;
            }
        }
        let count = self.items.len();
        let index = if next {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.go_to_file_at(index, next, focus, window, cx);
    }

    /// Monaco `_goToFile`: the file's first or last change, focusing its
    /// editor if `focus`.
    fn go_to_file_at(
        &mut self,
        index: usize,
        first: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.items.len() {
            return;
        }
        self.set_collapsed(index, false, cx);
        self.reveal(index, cx);
        let Some(diff) = self.ensure_bound(index, window, cx) else {
            // Monaco `DiffEditorItemBinding.focus`: a binary file's
            // placeholder (or its "Open Diff" button).
            if focus {
                let handle = self.items[index].placeholder_focus.clone();
                window.focus(&handle, cx);
            }
            return;
        };
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
        if focus {
            let modified = diff.read(cx).modified_editor().focus_handle(cx);
            window.focus(&modified, cx);
        }
    }

    /// Each file's height.
    fn heights(&self) -> Vec<Pixels> {
        let config = self.config();
        self.items.iter().map(|item| item.height(&config)).collect()
    }

    /// The scroll layout's item heights: each file's, then the bottom
    /// padding (Monaco's `paddingBottomItem`).
    fn layout_heights(&self) -> Vec<Pixels> {
        let mut heights = self.heights();
        heights.push(self.padding_bottom);
        heights
    }

    /// The top of file `index` in the list, below any room the anchoring
    /// left above the first file.
    fn item_top(&self, index: usize) -> Pixels {
        let config = self.config();
        self.items[..index.min(self.items.len())]
            .iter()
            .map(|item| item.height(&config))
            .fold(px(0.), |a, b| a + b)
    }

    /// The largest vertical scroll position for a `viewport` this tall.
    fn max_scroll_top(&self, viewport: Pixels) -> Pixels {
        (self.scroll_height() - viewport).max(px(0.))
    }

    /// Monaco's `scrollWidth` less the width: the widest horizontal scroll
    /// range of the files with an editor.
    fn max_scroll_left(&self, cx: &App) -> Pixels {
        self.bound
            .iter()
            .map(|t| t.diff.read(cx).max_scroll_left(cx))
            .fold(px(0.), |a, b| a.max(b))
    }

    /// The vertical scroll position shown at `now`: on the way to
    /// `scroll_top` during a smooth scroll.
    fn shown_scroll_top(&self, now: Instant) -> Pixels {
        match self.smooth_scroll {
            Some(smooth) => {
                let t = now.duration_since(smooth.start).as_secs_f32()
                    / SMOOTH_SCROLL_DURATION.as_secs_f32();
                if t >= 1. {
                    self.scroll_top
                } else {
                    smooth.from + (self.scroll_top - smooth.from) * ease_out_cubic(t)
                }
            }
            None => self.scroll_top,
        }
    }

    /// Monaco `SmoothScrollableElement._onMouseWheel`: from where the list
    /// is going, and smoothly for a physical wheel's line deltas
    /// (`mouseWheelSmoothScroll`), at once for a trackpad's pixel deltas.
    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(18.));
        let viewport = self.bounds.get().size;
        let max_top = self.max_scroll_top(viewport.height);
        let max_left = self.max_scroll_left(cx);
        let now = cx.background_executor().now();
        let top = (self.scroll_top - delta.y).clamp(px(0.), max_top);
        if matches!(event.delta, ScrollDelta::Lines(_)) && top != self.scroll_top {
            // Monaco `SmoothScrollingOperation.combine`: on from where it
            // shows now.
            self.smooth_scroll = Some(SmoothScroll {
                from: self.shown_scroll_top(now),
                start: now,
            });
        } else if !matches!(event.delta, ScrollDelta::Lines(_)) {
            self.smooth_scroll = None;
        }
        self.scroll_top = top;
        self.scroll_left = (self.scroll_left - delta.x).clamp(px(0.), max_left);
        self.pending_reveal = None;
        self.pending_scroll = None;
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
        self.pending_scroll = None;
        self.drag_scrollbar(position, cx);
    }

    /// The pointer over the whole window during a scrollbar drag (Monaco's
    /// pointer capture on the slider): the arrow, over the editors' text
    /// too.
    pub(crate) fn captured_cursor(&self) -> Option<gpui::CursorStyle> {
        self.scrollbar_drag.map(|_| gpui::CursorStyle::Arrow)
    }

    /// [`Self::captured_cursor`], for tests: GPUI's test platform doesn't
    /// keep the cursor.
    #[doc(hidden)]
    pub fn captured_cursor_for_tests(&self) -> Option<gpui::CursorStyle> {
        self.captured_cursor()
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
            self.smooth_scroll = None;
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
            // A collapsed file's editor isn't laid out: measure it once
            // it shows.
            if self.items[template.item].collapsed {
                continue;
            }
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
                && item
                    .content_height
                    .is_none_or(|measured| (height - measured).abs() > px(0.5))
            {
                item.content_height = Some(height);
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
        // Not measured yet (as when files load): the file's height, so the
        // line's place in the list, is about to change.
        if diff
            .content_height(cx)
            .zip(self.items[pending.item].content_height)
            .is_none_or(|(height, measured)| (height - measured).abs() > px(0.5))
        {
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
        // The hidden unchanged lines lay out on the next frame: measure
        // once they have.
        if editor.view_lines.hidden() != editor.hidden_areas() {
            return;
        }
        let (Some(top), Some(bottom)) = (
            editor.top_for_line_number(row, true),
            editor.bottom_for_line_number(row, false),
        ) else {
            return;
        };
        let offset = self.item_top(pending.item) + self.config().editor_top;
        let center = offset + (top + bottom) / 2.;
        self.scroll_top = (self.anchoring.leading_slack + center - viewport / 2.).max(px(0.));
        self.smooth_scroll = None;
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
            self.scrollbar_fade.left_at = Some(cx.background_executor().now());
        }
        if was_over != over
            || hovered_before != slider_hovered(&self.scrollbar_fade, &self.scrollbars)
        {
            cx.notify();
        }
    }

    /// Monaco's header click and Enter/Space on a focused header
    /// (`headerClickToCollapse`): activates the file and toggles it.
    fn toggle_from_header(&mut self, index: usize, cx: &mut Context<Self>) {
        self.active = Some(index);
        let collapsed = self.is_collapsed(index);
        self.set_collapsed(index, !collapsed, cx);
    }

    /// File `index`'s top and height, for tests.
    #[doc(hidden)]
    pub fn item_bounds_for_tests(&self, index: usize) -> (Pixels, Pixels) {
        (
            self.item_top(index),
            self.items[index].height(&self.config()),
        )
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

    /// The message shown over an empty list (Monaco's `.placeholder`):
    /// "Loading..." or "No Changed Files".
    pub fn placeholder_message(&self) -> Option<&'static str> {
        match (self.items.is_empty(), self.loading) {
            (false, _) => None,
            (true, true) => Some("Loading..."),
            (true, false) => Some("No Changed Files"),
        }
    }

    /// The files whose header was last drawn floating with a shadow, for
    /// tests.
    #[doc(hidden)]
    pub fn shadowed_headers_for_tests(&self) -> Vec<usize> {
        self.shadowed_headers.clone()
    }

    /// The vertical scroll position last drawn and the anchoring's room
    /// above and below the files, for tests.
    #[doc(hidden)]
    pub fn scroll_state_for_tests(&self) -> (Pixels, Pixels, Pixels) {
        (
            self.drawn_scroll.y,
            self.anchoring.leading_slack,
            self.anchoring.trailing_slack,
        )
    }

    /// The per-file state the header and borders are drawn with.
    fn entry_state(&self, index: usize, list_focused: bool, window: &Window) -> EntryState {
        let active = self.active == Some(index);
        let header_focused =
            self.config().header_click_to_collapse && self.items[index].header_has_focus(window);
        EntryState {
            active,
            focused: self.focused_item == Some(index),
            selected: list_focused && active,
            header_highlighted: self.hovered_header == Some(index) || header_focused,
            hovered: self.hovered_entry == Some(index),
        }
    }

    /// The header (Monaco `DiffEditorItemTemplate`'s `.header`) for the
    /// variant, at `top` in its file's visible part.
    #[allow(clippy::too_many_arguments)]
    fn render_header(
        &self,
        index: usize,
        top: Pixels,
        stuck: bool,
        pushed_up: bool,
        state: EntryState,
        colors: &EditorColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let config = self.config();
        let item = &self.items[index];
        let collapsed = item.collapsed;
        let content = self.render_header_content(index, state, colors, cx);
        if !config.compact {
            // `.multiDiffEditor-standard`: an 8px margin above the content,
            // a border above it (`focusBorder` when active) and one below
            // (a header pushed up by the next file, or a collapsed file's,
            // has none). No shadow when the file is scrolled under it:
            // Monaco toggles `shadow` on `.header`, but its rule targets
            // `.header-content.shadow`, so only the cards variant's applies.
            let border = if state.active {
                colors.focus_border
            } else {
                colors.multi_diff_border
            };
            let bottom_border = colors
                .multi_diff_header_border
                .filter(|_| !pushed_up && !collapsed);
            return div()
                .absolute()
                .top(top)
                .left(px(0.))
                .right(px(0.))
                .h(config.header_height)
                .pt(px(HEADER_MARGIN_TOP))
                .bg(colors.multi_diff_background)
                .child(
                    content
                        .relative()
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
                        }),
                )
                .into_any_element();
        }
        // The compact variants: the header collapses its file when clicked
        // and takes focus (`tabIndex = 0`, `role="button"`,
        // `aria-expanded`).
        let label = item.document.label.clone();
        let header = div()
            .id(("multi-diff-header", index))
            .debug_selector(move || format!("multi-diff-header-{index}"))
            .track_focus(&item.header_focus.clone().tab_index(header_tab_index(index)))
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_expanded(!collapsed)
            .absolute()
            .top(top)
            .left(px(0.))
            .right(px(0.))
            .h(config.header_height)
            .bg(colors.multi_diff_background)
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_header = Some(index);
                } else if this.hovered_header == Some(index) {
                    this.hovered_header = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_from_header(index, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.toggle_from_header(index, cx);
                } else {
                    tab_moves_focus(event, window, cx);
                }
            }));
        let hover_background = colors.list_hover_background;
        let pressed_background = colors.toolbar_active_background;
        if !config.card {
            // `.multiDiffEditor-compact`: the content fills the header,
            // highlighted when hovered or pressed, and in
            // `list.inactiveSelectionBackground` while the list has focus
            // and the file is active.
            let selected = state
                .selected
                .then_some(colors.list_inactive_selection_background)
                .flatten();
            let focus_border = colors.focus_border;
            return header
                .focus_visible(move |style| style.border_1().border_color(focus_border))
                .child(
                    content
                        .size_full()
                        .h(px(COMPACT_HEADER_CONTENT_HEIGHT))
                        .when_some(selected, |content, background| content.bg(background))
                        .when(selected.is_none(), |content| {
                            content
                                .when_some(hover_background, |content, background| {
                                    content.hover(move |style| style.bg(background))
                                })
                                .when_some(pressed_background, |content, background| {
                                    content.active(move |style| style.bg(background))
                                })
                        }),
                )
                .into_any_element();
        }
        // `.multiDiffEditor-card`: a bordered, rounded header 4px below the
        // file's top and 8px in from the sides. Its background layers the
        // hover, pressed and focus colors over
        // `sideBarSectionHeader.background`; its border is `focusBorder`
        // while the file has focus. Stuck to the top of its expanded file,
        // it's square, 4px taller, without the top border and with a
        // shadow.
        let base = colors
            .multi_diff_card_header_background
            .unwrap_or(colors.multi_diff_background);
        let focused = state.focused || state.selected;
        let background = match colors.list_focus_background.filter(|_| focused) {
            Some(focus) => base.blend(focus),
            None => base,
        };
        let border = if state.focused {
            Some(colors.focus_border)
        } else {
            colors.multi_diff_card_border
        };
        let border_color = border.unwrap_or_else(gpui::transparent_black);
        let floating = header_floats(&config, stuck, collapsed);
        let radius = px(CORNER_RADIUS_MEDIUM);
        let content = content
            .mx(px(CARD_PADDING_X))
            .mt(if floating {
                px(0.)
            } else {
                px(CARD_MARGIN_TOP)
            })
            .h(if floating {
                px(COMPACT_HEADER_CONTENT_HEIGHT + CARD_MARGIN_TOP)
            } else {
                px(COMPACT_HEADER_CONTENT_HEIGHT)
            })
            .border_l_1()
            .border_r_1()
            .when(collapsed, |content| content.border_b_1())
            .border_color(border_color)
            .bg(background)
            .when(!focused, |content| {
                content
                    .when_some(colors.toolbar_hover_background, |content, hover| {
                        content.hover(move |style| style.bg(base.blend(hover)))
                    })
                    .when_some(pressed_background, |content, pressed| {
                        content.active(move |style| style.bg(base.blend(pressed)))
                    })
            });
        // `border-top-color: transparent` over the background: no top
        // border shows.
        let content = if floating {
            content.shadow(header_shadow(colors.scrollbar_shadow))
        } else if collapsed {
            content.border_t_1().rounded(radius)
        } else {
            content.border_t_1().rounded_t(radius)
        };
        let focus_border = colors.focus_border;
        header
            .flex()
            .flex_col()
            .focus_visible(move |style| style.border_1().border_color(focus_border))
            .child(content.relative())
            .into_any_element()
    }

    /// The header's content: chevron, file label, status and toolbar
    /// (Monaco `DiffEditorItemTemplate`'s `.header-content`).
    fn render_header_content(
        &self,
        index: usize,
        state: EntryState,
        colors: &EditorColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let config = self.config();
        let item = &self.items[index];
        let collapsed = item.collapsed;
        // Monaco's `chevron-right` / `chevron-down` toggle.
        let chevron = codicon(if collapsed {
            Codicon::ChevronRight
        } else {
            Codicon::ChevronDown
        });
        let status = match item.status {
            FileStatus::Modified => None,
            FileStatus::Added => Some(("A", colors.multi_diff_added_foreground)),
            FileStatus::Deleted => Some(("D", colors.multi_diff_deleted_foreground)),
            FileStatus::Renamed => Some(("R", colors.multi_diff_renamed_foreground)),
        };
        let foreground = colors.multi_diff_header_foreground;
        let label = item.document.label.clone();
        // The compact variants show the toolbar only while the mouse is
        // over the file or it has focus.
        let show_actions = !config.compact || state.hovered || state.focused;
        // Monaco's toolbar context key scope, read only when an action tests
        // it.
        let context = (show_actions && self.file_actions.iter().any(FileAction::conditional))
            .then(|| self.file_context_keys(index, cx));
        // Monaco's `actionBar.toggledBackground` (`inputOption.activeBackground`).
        let checked_background = colors.input_option_active_background;
        let actions = self
            .file_actions
            .iter()
            .enumerate()
            .filter_map(|(ix, file_action)| {
                let toggled = match &context {
                    Some(context) if !file_action.visible(context) => return None,
                    Some(context) => file_action.toggled(context),
                    None => None,
                };
                Some((ix, file_action.action.clone(), toggled))
            })
            .collect::<Vec<_>>();
        let has_actions = !actions.is_empty();
        let actions = actions.into_iter().map(|(ix, action, toggled)| {
            let document = item.document.clone();
            let on = toggled == Some(true);
            let toggle = action.toggled.as_ref().filter(|_| on);
            let label = toggle
                .and_then(|t| t.label.clone())
                .unwrap_or_else(|| action.label.clone());
            let icon = toggle
                .and_then(|t| t.icon.clone())
                .unwrap_or_else(|| action.icon.clone());
            let id = action.id.clone();
            // Monaco's `ActionViewItem`: a checkbox for an action with a
            // toggled state.
            let aria = match toggled {
                Some(checked) => {
                    crate::accessibility::AriaNode::new(gpui::Role::CheckBox, label.clone())
                        .checked(checked)
                }
                None => crate::accessibility::AriaNode::new(gpui::Role::Button, label.clone()),
            };
            aria.apply(
                div()
                    .id(("multi-diff-file-action", ix))
                    .debug_selector(move || format!("multi-diff-file-action-{index}-{id}")),
            )
            .tooltip(crate::tooltip::tooltip(label, colors))
            .when(on, |button| button.bg(checked_background))
            .flex()
            .items_center()
            .justify_center()
            .min_w(px(22.))
            .h(px(22.))
            .p(if config.compact { px(4.) } else { px(3.) })
            .rounded(px(5.))
            .cursor_pointer()
            .hover(|style| colors.toolbar_hover(style))
            .children(crate::diff_editor::label_with_icons(&icon))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.active = Some(index);
                (action.run)(index, &document, window, cx);
                cx.notify();
            }))
        });
        let focus_border = colors.focus_border;
        let chevron_button = div()
            .id(("multi-diff-collapse", index))
            .debug_selector(|| format!("multi-diff-collapse-{index}"))
            .track_focus(
                &item
                    .chevron_focus
                    .clone()
                    .tab_index(header_tab_index(index)),
            )
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(format!(
                "{}, {}",
                label,
                if collapsed { "collapsed" } else { "expanded" }
            )))
            .mx(px(6.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .map(|button| {
                if config.compact {
                    // The compact `.collapse-button a`: 4px padding and a
                    // rounded hover background.
                    let pressed = colors.toolbar_active_background;
                    button
                        .p(px(4.))
                        .rounded(px(CORNER_RADIUS_MEDIUM))
                        .hover(|style| colors.toolbar_hover(style))
                        .when_some(pressed, |button, pressed| {
                            button.active(move |style| style.bg(pressed))
                        })
                } else {
                    button.w(px(16.)).h(px(22.))
                }
            })
            .focus_visible(move |style| style.border_1().border_color(focus_border))
            .child(chevron)
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                // Monaco `activateItem`.
                this.active = Some(index);
                this.set_collapsed(index, !collapsed, cx)
            }))
            // Monaco's `Button`: Enter or Space clicks it.
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.toggle_from_header(index, cx);
                } else {
                    tab_moves_focus(event, window, cx);
                }
            }));
        div()
            .id(("multi-diff-header-content", index))
            .flex()
            .flex_row()
            .items_center()
            .text_color(foreground)
            .text_size(if config.compact { px(13.) } else { px(14.) })
            .line_height(px(22.))
            .child(chevron_button)
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
                    .when_some(status, |path, (status, color)| {
                        if config.compact {
                            // The compact `.status`: 16px wide, 12px,
                            // semibold at 0.9 opacity, in the git
                            // decoration colors.
                            path.child(
                                div()
                                    .debug_selector(move || format!("multi-diff-status-{index}"))
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .w(px(16.))
                                    .mx(px(10.))
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(color.opacity(0.9))
                                    .child(status),
                            )
                        } else {
                            // Monaco's `.status`: semibold at 0.75 opacity.
                            path.child(
                                div()
                                    .debug_selector(move || format!("multi-diff-status-{index}"))
                                    .flex_none()
                                    .mx(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(foreground.opacity(0.75))
                                    .child(status),
                            )
                        }
                    })
                    // Monaco's second label: the original path, struck
                    // through, for a rename.
                    .when_some(
                        item.document
                            .original_label
                            .clone()
                            .filter(|_| item.status == FileStatus::Renamed),
                        |path, original| {
                            path.child(
                                resource_label(&original, true, false, colors).debug_selector(
                                    move || format!("multi-diff-original-label-{index}"),
                                ),
                            )
                        },
                    ),
            )
            .when(has_actions && show_actions, |header| {
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

    /// A binary file's placeholder (Monaco's `.binary-file-placeholder`):
    /// "Binary file changed" and, with a handler and both sides, an "Open
    /// Diff" button. The button, else the placeholder, takes focus.
    fn render_binary_placeholder(
        &self,
        index: usize,
        colors: &EditorColors,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let item = &self.items[index];
        let open_diff = self
            .open_binary_diff
            .clone()
            .filter(|_| item.has_both_sides());
        let focus_border = colors.focus_border;
        let placeholder = div()
            .id(("multi-diff-binary", index))
            .debug_selector(move || format!("multi-diff-binary-{index}"))
            .role(gpui::Role::Group)
            .aria_label("Binary file changed")
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_color(colors.description_foreground)
            .text_size(px(13.))
            .child("Binary file changed");
        let Some(handler) = open_diff else {
            return placeholder
                .track_focus(
                    &item
                        .placeholder_focus
                        .clone()
                        .tab_index(header_tab_index(index) + 1),
                )
                .focus_visible(move |style| style.border_1().border_color(focus_border))
                .on_key_down(|event: &KeyDownEvent, window, cx| {
                    tab_moves_focus(event, window, cx)
                });
        };
        let document = item.document.clone();
        let run = {
            let document = document.clone();
            let handler = handler.clone();
            move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                this.active = Some(index);
                handler(index, &document, window, cx);
                cx.notify();
            }
        };
        let run_on_key = run.clone();
        let button = div()
            .id(("multi-diff-open-binary-diff", index))
            .debug_selector(move || format!("multi-diff-open-binary-diff-{index}"))
            .track_focus(
                &item
                    .placeholder_focus
                    .clone()
                    .tab_index(header_tab_index(index) + 1),
            )
            .role(gpui::Role::Button)
            .px(px(14.))
            .py(px(4.))
            .rounded(px(2.))
            .cursor_pointer()
            .text_color(colors.button_secondary_foreground)
            .when_some(colors.button_secondary_background, |button, background| {
                button.bg(background)
            })
            .when_some(colors.button_secondary_border, |button, border| {
                button.border_1().border_color(border)
            })
            .when_some(colors.button_secondary_hover_background, |button, hover| {
                button.hover(move |style| style.bg(hover))
            })
            .focus_visible(move |style| style.border_1().border_color(focus_border))
            .child("Open Diff")
            .on_click(cx.listener(move |this, _, window, cx| run(this, window, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    run_on_key(this, window, cx);
                } else {
                    tab_moves_focus(event, window, cx);
                }
            }));
        placeholder.child(button)
    }

    /// The list's scrollbars (Monaco `SmoothScrollableElement`'s, `Auto`
    /// and without shadows).
    fn render_scrollbars(
        &mut self,
        origin: Point<Pixels>,
        colors: &EditorColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
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
        // While dragging: follow the mouse anywhere in the window, with the
        // slider's arrow (Monaco's pointer capture).
        if self.scrollbar_drag.is_some() {
            elements.push(
                crate::pointer_capture::capture(
                    cx.entity().downgrade(),
                    self.captured_cursor(),
                    |this: &mut Self, event, cx| this.drag_scrollbar(event.position, cx),
                    |this, cx| {
                        this.scrollbar_drag = None;
                        cx.notify();
                    },
                )
                .into_any_element(),
            );
        }
        elements
    }
}

/// File `index`'s header and chevron tab stops' index in the list's tab
/// group, so a file's header, chevron and editor (`+ 1`) come in that
/// order, then the next file's (Monaco's DOM order).
fn header_tab_index(index: usize) -> isize {
    isize::try_from(index).unwrap_or(isize::MAX / 2) * 2
}

/// The browser's Tab and Shift+Tab on a header, chevron or binary
/// placeholder, which the editor's keymap doesn't reach: they move the
/// focus to the next or previous tab stop.
fn tab_moves_focus(event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let keystroke = &event.keystroke;
    let modifiers = keystroke.modifiers;
    if keystroke.key != "tab" || modifiers.control || modifiers.alt || modifiers.platform {
        return;
    }
    cx.stop_propagation();
    if modifiers.shift {
        window.focus_prev(cx);
    } else {
        window.focus_next(cx);
    }
}

/// Whether a header floats over its file with a shadow: Monaco's
/// `.header.shadow[aria-expanded='true'] .header-content`, which only the
/// cards variant has (the standard variant's `.header-content.shadow`
/// never matches).
fn header_floats(config: &VariantConfiguration, stuck: bool, collapsed: bool) -> bool {
    config.card && stuck && !collapsed
}

/// `box-shadow: var(--vscode-scrollbar-shadow) 0 6px 6px -6px`, under a
/// header with its file scrolled under it.
fn header_shadow(color: Hsla) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color,
        offset: point(px(0.), px(6.)),
        blur_radius: px(6.),
        spread_radius: px(-6.),
        inset: false,
    }]
}

/// Monaco `highlightRange`: `range` in `symbolHighlight` for 350ms, unless
/// the editor shows another model by then.
fn highlight_range(editor: &Entity<CodeEditor>, range: Range<PointUtf16>, cx: &mut App) {
    let model = editor.read(cx).model().clone();
    let collection = editor.update(cx, |editor, cx| {
        let colors = editor.colors();
        let options = DecorationOptions {
            background: Some(colors.symbol_highlight_background),
            border: colors.symbol_highlight_border,
            ..Default::default()
        };
        editor.create_decorations_collection(vec![(range, options)], cx)
    });
    let editor = editor.downgrade();
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(REVEAL_HIGHLIGHT_DURATION)
            .await;
        let mut collection = collection;
        cx.update(|cx| {
            if let Some(editor) = editor.upgrade()
                && *editor.read(cx).model() == model
            {
                collection.clear(cx);
            }
        });
    })
    .detach();
}

/// Removes the context keys `stale` from `editor`, then sets `keys` (a
/// file's [`MultiDiffDocument::context_keys`]).
fn set_host_keys(
    editor: &mut CodeEditor,
    stale: &[SharedString],
    keys: &[(SharedString, ContextKeyValue)],
    cx: &mut Context<CodeEditor>,
) {
    for key in stale {
        editor.commands.host_keys.remove(key);
    }
    for (key, value) in keys {
        editor.commands.host_keys.insert(key.clone(), value.clone());
    }
    cx.notify();
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
            editor.set_selections(&[Selection::collapsed(PointUtf16::new(row, 0))], cx);
            editor.reveal_line_in_center(row, cx);
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
) -> gpui::Div {
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
            // Monaco's `file` codicon (the workbench shows the icon
            // theme's).
            div()
                .flex_none()
                .pr(px(6.))
                .text_color(colors.symbol_icons.file)
                .child(codicon(Codicon::File)),
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
            .unwrap_or_else(|| crate::theme::Theme::vs().colors),
    }
}

impl Render for MultiDiffEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = cx.background_executor().now();
        let bounds = self.bounds.get();
        // Before the first layout, the window's height.
        let viewport = if bounds.size.height > px(0.) {
            bounds.size
        } else {
            window.viewport_size()
        };
        let config = self.config();
        self.measure(window, cx);
        // Monaco `CompressedVirtualizedScrollView._update`: keep the anchor
        // in place through height changes; a correction ends a smooth
        // scroll.
        let shown = self.shown_scroll_top(now);
        let (anchored, geometry_changed) =
            self.anchoring
                .update(&self.layout_heights(), viewport.height, shown, now);
        let heights = self.heights();
        if geometry_changed || anchored != shown {
            self.scroll_top = anchored;
            self.smooth_scroll = None;
        }
        self.resolve_pending_reveal(viewport.height, cx);
        // Monaco `_applyPendingScrollState`: a restored position, until the
        // list is tall (or wide) enough for it.
        if let Some(pending) = self.pending_scroll {
            if let Some(top) = pending.top {
                self.scroll_top = top;
            }
            if let Some(left) = pending.left {
                self.scroll_left = left;
            }
        }
        let max_top = self.max_scroll_top(viewport.height);
        self.scroll_top = self.scroll_top.clamp(px(0.), max_top);
        let max_left = self.max_scroll_left(cx);
        if bounds.size.width > px(0.) {
            self.scroll_left = self.scroll_left.clamp(px(0.), max_left);
        }
        if let Some(pending) = &mut self.pending_scroll {
            pending.top = pending.top.filter(|&top| self.scroll_top < top);
            pending.left = pending.left.filter(|&left| self.scroll_left < left);
            if pending.top.is_none() && pending.left.is_none() {
                self.pending_scroll = None;
            }
        }
        let mut shown = self.shown_scroll_top(now).clamp(px(0.), max_top);
        if self.smooth_scroll.is_some() {
            if now.duration_since(self.smooth_scroll.map_or(now, |s| s.start))
                >= SMOOTH_SCROLL_DURATION
            {
                self.smooth_scroll = None;
                shown = self.scroll_top;
            } else {
                window.request_animation_frame();
            }
        }
        self.anchoring.last_scroll_top = shown;
        let scroll_left = self.scroll_left;
        // Monaco reveals `auto` scrollbars whenever the list scrolls.
        if self.drawn_scroll != point(scroll_left, shown) {
            self.drawn_scroll = point(scroll_left, shown);
            self.scrollbar_fade.revealed_at = Some(cx.background_executor().now());
        }
        // The list's top in the files' coordinates.
        let scroll_top = shown - self.anchoring.leading_slack;

        // The files in view (Monaco's rendered range).
        let mut visible = Vec::new();
        let mut top = px(0.);
        for (index, &height) in heights.iter().enumerate() {
            let item_top = top;
            top += height;
            if item_top >= scroll_top + viewport.height {
                break;
            }
            if item_top + height > scroll_top {
                visible.push((index, item_top));
            }
        }
        // Monaco binds every file in view, collapsed or not; a collapsed
        // file's editor is hidden.
        let wanted: Vec<usize> = visible
            .iter()
            .map(|&(index, _)| index)
            .filter(|&index| !self.items[index].document.binary)
            .collect();
        self.drawn_items = visible.iter().map(|&(index, _)| index).collect();
        self.update_bindings(&wanted, window, cx);
        // A restored active file takes the focus (Monaco
        // `DiffEditorItemBinding.focus`), with an editor if it has none.
        if let Some(index) = self.pending_focus.take()
            && self.items.get(index).is_some_and(|item| !item.collapsed)
        {
            match self.ensure_bound(index, window, cx) {
                Some(diff) => {
                    let modified = diff.read(cx).modified_editor().focus_handle(cx);
                    window.focus(&modified, cx);
                }
                None => {
                    let placeholder = self.items[index].placeholder_focus.clone();
                    window.focus(&placeholder, cx);
                }
            }
        }
        for template in &mut self.bound {
            template.follow_cursor = template.follow_cursor.saturating_sub(1);
        }
        // Monaco `setIsFocused`: a file whose editor, header or binary
        // placeholder gains focus becomes the active one.
        let focused = self
            .bound
            .iter()
            .find(|t| t.is_focused(window, cx))
            .map(|t| t.item)
            .or_else(|| {
                self.items.iter().position(|item| {
                    item.placeholder_focus.is_focused(window)
                        || (config.header_click_to_collapse && item.header_has_focus(window))
                })
            });
        if focused != self.focused_item {
            self.focused_item = focused;
            if focused.is_some() {
                self.active = focused;
            }
        }
        self.report_active_control(cx);
        let list_focused = self.focus_handle.contains_focused(window, cx);

        // Monaco `ScrollableElement`: each bar leaves room for the other.
        self.scrollbars = [
            ScrollbarState::new(
                Bounds::new(
                    point(bounds.right() - px(SCROLLBAR_SIZE), bounds.top()),
                    size(px(SCROLLBAR_SIZE), bounds.size.height - px(SCROLLBAR_SIZE)),
                ),
                true,
                bounds.size.height,
                self.scroll_height(),
                shown,
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
        self.shadowed_headers.clear();
        for (position, &(index, item_top)) in visible.iter().enumerate() {
            let height = heights[index];
            let collapsed = self.items[index].collapsed;
            let binary = self.items[index].document.binary;
            // The visible part: the editor scrolled to the part under the
            // header, which sticks to the top until the next file pushes it
            // up (Monaco's `translateY(delta)`).
            let visible_start = item_top.max(scroll_top);
            let visible_end = (item_top + height).min(scroll_top + viewport.height);
            let delta = (visible_start - item_top)
                .min(height - config.header_height)
                .max(px(0.));
            let header_top = item_top + delta - visible_start;
            let editor_scroll = visible_start - item_top;
            let content_bottom = item_top + height - config.content_bottom_padding;
            let editor_height =
                (visible_end.min(content_bottom) - visible_start - config.editor_top).max(px(0.));
            let shows_bottom = visible_end >= item_top + height;
            let diff = self.bound.iter_mut().find(|t| t.item == index).map(|t| {
                t.expected_scroll = point(scroll_left, editor_scroll);
                t.diff.clone()
            });
            if let Some(diff) = &diff {
                diff.update(cx, |diff, cx| {
                    diff.set_scroll_position_unclamped(point(scroll_left, editor_scroll), cx)
                });
            }
            let state = self.entry_state(index, list_focused, window);
            // The next file's header highlight draws the line above it,
            // over this file's (Monaco's `::before` at -1px).
            let next_highlighted = visible.get(position + 1).is_some_and(|&(next, _)| {
                let next = self.entry_state(next, list_focused, window);
                next.header_highlighted || next.selected
            });
            let stuck = visible_start > item_top;
            if header_floats(&config, stuck, collapsed) {
                self.shadowed_headers.push(index);
            }
            let header = self.render_header(
                index,
                header_top,
                stuck,
                delta >= height - config.header_height,
                state,
                &colors,
                cx,
            );
            let border = if state.active {
                colors.focus_border
            } else {
                colors.multi_diff_border
            };
            // The editor's (or placeholder's) part, under the header.
            let body_top = item_top + config.editor_top - visible_start;
            let placeholder =
                (binary && !collapsed).then(|| self.render_binary_placeholder(index, &colors, cx));
            children.push(
                div()
                    .id(("multi-diff-entry", index))
                    .absolute()
                    .top(visible_start - scroll_top)
                    .left(px(0.))
                    .right(px(0.))
                    .h(visible_end - visible_start)
                    .overflow_hidden()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.hovered_entry = Some(index);
                        } else if this.hovered_entry == Some(index) {
                            this.hovered_entry = None;
                        }
                        cx.notify();
                    }))
                    .when_some(
                        diff.filter(|_| !collapsed && editor_height > px(0.)),
                        |item_div, diff| {
                            item_div.child(
                                div()
                                    .debug_selector(move || format!("multi-diff-editor-{index}"))
                                    // After its header's tab stops (an
                                    // element without an id: a group
                                    // only, not focusable).
                                    .tab_group()
                                    .tab_index(header_tab_index(index) + 1)
                                    .absolute()
                                    .top(config.editor_top)
                                    .left(config.horizontal_inset)
                                    .right(config.horizontal_inset)
                                    .h(editor_height)
                                    .child(diff),
                            )
                        },
                    )
                    .when_some(placeholder, |item_div, placeholder| {
                        item_div.child(
                            placeholder
                                .absolute()
                                .top(body_top)
                                .left(config.horizontal_inset)
                                .right(config.horizontal_inset)
                                .h(px(BINARY_PLACEHOLDER_HEIGHT)),
                        )
                    })
                    .map(|item_div| {
                        if config.card {
                            // `.multiDiffEditor-card .editorParent`: the
                            // card's sides and rounded bottom, in
                            // `focusBorder` while the file has focus.
                            let card_border = if state.focused {
                                Some(colors.focus_border)
                            } else {
                                colors.multi_diff_card_border
                            };
                            item_div.when_some(
                                card_border.filter(|_| !collapsed),
                                |item_div, card_border| {
                                    item_div.child(
                                        div()
                                            .absolute()
                                            .top(body_top)
                                            .left(px(CARD_PADDING_X))
                                            .right(px(CARD_PADDING_X))
                                            .h(height - config.editor_top)
                                            .border_l_1()
                                            .border_r_1()
                                            .border_b_1()
                                            .border_color(card_border)
                                            .rounded_b(px(CORNER_RADIUS_MEDIUM)),
                                    )
                                },
                            )
                        } else if config.compact {
                            // `.multiDiffEntry::after`: a line 10px in from
                            // the sides below each file, the whole width
                            // while its header (or the next one) is
                            // hovered or focused, or the file is selected.
                            let full =
                                state.header_highlighted || state.selected || next_highlighted;
                            let inset = if full {
                                px(0.)
                            } else {
                                px(COMPACT_SEPARATOR_INSET)
                            };
                            let first_line = (index == 0
                                && (state.header_highlighted || state.selected)
                                && visible_start == item_top)
                                .then_some(colors.multi_diff_entry_border)
                                .flatten();
                            item_div
                                .when_some(
                                    colors.multi_diff_entry_border.filter(|_| shows_bottom),
                                    |item_div, line| {
                                        item_div.child(
                                            div()
                                                .absolute()
                                                .bottom(px(0.))
                                                .left(inset)
                                                .right(inset)
                                                .h(px(1.))
                                                .bg(line),
                                        )
                                    },
                                )
                                .when_some(first_line, |item_div, line| {
                                    item_div.child(
                                        div()
                                            .absolute()
                                            .top(px(0.))
                                            .left(px(0.))
                                            .right(px(0.))
                                            .h(px(1.))
                                            .bg(line),
                                    )
                                })
                        } else {
                            // Monaco's `.editorParent` bottom border.
                            item_div.when(!collapsed && shows_bottom, |item_div| {
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
                        }
                    })
                    .child(header),
            );
        }
        let scrollbars = self.render_scrollbars(bounds.origin, &colors, window, cx);
        // Monaco's `.placeholder`, centered over an empty list.
        let placeholder = self.placeholder_message().map(|message| {
            div()
                .debug_selector(|| "multi-diff-placeholder".into())
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(colors.multi_diff_header_foreground)
                .child(message)
        });
        // Monaco `inMultiDiffEditor`, and `multiDiffEditorAllCollapsed`
        // (which Monaco sets on the parent scope), for the list and its
        // editors.
        let all_collapsed = self.items.iter().all(|item| item.collapsed);
        for template in &self.bound {
            template.diff.update(cx, |diff, cx| {
                if diff.multi_diff_all_collapsed != Some(all_collapsed) {
                    diff.multi_diff_all_collapsed = Some(all_collapsed);
                    cx.notify();
                }
            });
        }
        let key_context = self.context_keys(cx);
        let bounds_cell = self.bounds.clone();
        let this = cx.entity().downgrade();
        div()
            .id("multi-diff-editor")
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            // The files' tab stops (`header_tab_index`) are ordered within
            // the list.
            .tab_group()
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
            .children(placeholder)
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

    #[test]
    fn variant_configurations() {
        // Monaco `getMultiDiffEditorVariantConfiguration`.
        let standard = MultiDiffEditorVariant::NoCardsNonCompact.configuration();
        assert_eq!(
            (standard.header_height, standard.content_bottom_padding),
            (px(40.), px(0.))
        );
        assert!(!standard.header_click_to_collapse && !standard.hide_original_line_numbers);
        let compact = MultiDiffEditorVariant::NoCards.configuration();
        assert_eq!(
            (
                compact.header_height,
                compact.content_bottom_padding,
                compact.horizontal_inset
            ),
            (px(32.), px(8.), px(0.))
        );
        assert!(compact.compact && !compact.card && compact.header_click_to_collapse);
        let cards = MultiDiffEditorVariant::Cards.configuration();
        assert_eq!(
            (
                cards.header_height,
                cards.horizontal_inset,
                cards.editor_top
            ),
            (px(40.), px(9.), px(36.))
        );
        assert!(cards.compact && cards.card && cards.hide_original_line_numbers);
        assert_eq!(
            MultiDiffEditorVariant::default(),
            MultiDiffEditorVariant::NoCardsNonCompact
        );
    }
}
