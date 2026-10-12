//! Hover (Monaco's `contrib/hover`): after the mouse rests over text, shows
//! decoration hover messages (e.g. diagnostics) and hover provider results in
//! a content widget.

use std::ops::Range;
use std::time::Duration;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use futures::future::LocalBoxFuture;
use futures::stream::FuturesUnordered;
use futures::{FutureExt as _, StreamExt as _};

use gpui::{
    AppContext as _, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, Modifiers, ParentElement, Pixels, Render, ScrollHandle, Size,
    StatefulInteractiveElement, Styled, Task, Window, canvas, div, point, px, size,
};
use text::PointUtf16;

use crate::actions::{
    FocusNextElement, FocusPreviousElement, GoToBottomHover, GoToTopHover, HideHover,
    PageDownHover, PageUpHover, ScrollDownHover, ScrollLeftHover, ScrollRightHover, ScrollUpHover,
};
use crate::codicons::{Codicon, icon as codicon};
use crate::cursor;
use crate::decorations::GlyphMarginLane;
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_markdown};
use crate::options::{HoverEnabled, MultiCursorModifier};
use crate::providers::{
    Hover, HoverContext, HoverProvider, HoverVerbosityRequest, MarkdownString, hover_providers,
};
use crate::widgets::{BesideGutter, ContentWidgetPreference, WidgetId, card};

/// Monaco's default `hover.delay`.
pub const HOVER_DELAY: Duration = Duration::from_millis(300);

/// The key context of a focused hover (Monaco `hoverFocused`).
pub const HOVER_KEY_CONTEXT: &str = "HoverWidget";

#[derive(Default)]
pub(crate) struct HoverState {
    task: Option<Task<()>>,
    widget: Option<(Entity<HoverWidget>, WidgetId)>,
    /// The range the visible (or pending) hover is about.
    range: Option<Range<PointUtf16>>,
    /// The row and lane or line number a glyph hover is about (Monaco
    /// `GlyphHoverWidget._hoverComputerOptions`).
    glyph: Option<(u32, LaneOrLineNumber)>,
    /// Shown from the keyboard (Monaco `isVisibleFromKeyboard`).
    from_keyboard: bool,
    /// Monaco's `hidingDelay` grace period before reacting to the mouse.
    hiding: Option<Task<()>>,
    /// The last mouse target and modifiers, for `onKeyboardModifier`.
    last_mouse: Option<(HoverTarget, Modifiers)>,
    /// The size the card was last resized to (Monaco
    /// `ContentHoverWidget._lastDimensions`): later cards may grow to it.
    last_size: Rc<Cell<Option<Size<Pixels>>>>,
    /// The inlay hint label part it's about (Monaco `InlayHintsHover`).
    inlay: Option<crate::inlay_hints::InlayHintPart>,
    /// The `hoverHighlight` decoration on the visible hover's range
    /// (Monaco `RenderedContentHoverParts._createEditorDecorations`).
    highlight: Option<crate::api::DecorationsCollection>,
    /// The marker whose quick fixes were last asked for, and whether it had
    /// any (Monaco `MarkerHoverParticipant.recentMarkerCodeActionsInfo`).
    recent_marker_code_actions: Option<(crate::Marker, bool)>,
    /// The visible card shows a partial result, still computing (Monaco
    /// `ContentHoverResult.isComplete` off).
    partial: bool,
}

/// A provider's hover, kept to ask it for more or less detail (Monaco
/// `HoverSource`).
#[derive(Clone)]
pub(crate) struct HoverPartSource {
    provider: Arc<dyn HoverProvider>,
    hover: Hover,
    position: PointUtf16,
}

impl HoverPartSource {
    /// Monaco `HoverSource.supportsVerbosityAction`.
    fn supports(&self, increase: bool) -> bool {
        if increase {
            self.hover.can_increase_verbosity
        } else {
            self.hover.can_decrease_verbosity
        }
    }
}

/// A markdown hover part (Monaco `MarkdownHover`): a decoration's hover
/// message, a provider's hover, or a long line warning.
#[derive(Clone)]
pub(crate) struct MarkdownHoverPart {
    contents: Vec<MarkdownString>,
    source: Option<HoverPartSource>,
    /// A long line warning, with Monaco's "Don't Show Again" link.
    long_line_warning: bool,
    /// The range it's about, for the `hoverHighlight` decoration (Monaco
    /// `IHoverPart.range`); `None` for parts that aren't about text.
    range: Option<Range<PointUtf16>>,
    /// Where it goes among the markdown parts (Monaco `MarkdownHover.ordinal`):
    /// a provider's index, [`SYNC_PART_ORDINAL`] on for decoration messages
    /// and warnings, [`LOADING_PART_ORDINAL`] for "Loading...".
    ordinal: usize,
}

impl MarkdownHoverPart {
    fn new(contents: Vec<MarkdownString>) -> Self {
        Self {
            contents,
            source: None,
            long_line_warning: false,
            range: None,
            ordinal: 0,
        }
    }
}

/// Monaco `MarkdownHoverParticipant.computeSync`'s first ordinal: after the
/// providers' parts.
const SYNC_PART_ORDINAL: usize = 1000;
/// Monaco `createLoadingMessage`'s ordinal: last.
const LOADING_PART_ORDINAL: usize = 2000;
/// Monaco's `modesContentHover.loading` message.
const LOADING_MESSAGE: &str = "Loading...";

/// Monaco's message when rendering stops on a long line.
const STOPPED_RENDERING: &str = "Rendering paused for long line for performance reasons. This can be configured via `stop_rendering_line_after`.";
/// Monaco's message when tokenization skips a long line.
const TOO_MANY_CHARACTERS: &str = "Tokenization is skipped for long lines for performance reasons. This can be configured via `max_tokenization_line_length`.";

/// The unicode hover part's "Adjust settings" link (Monaco's
/// `ShowExcludeOptions` command link).
const ADJUST_UNICODE_SETTINGS: &str = "command:editor.action.unicodeHighlight.showExcludeOptions";

/// The hover card's group, whose hover shows its scrollbars.
const HOVER_SCROLL_GROUP: &str = "hover-scrollable";

/// Monaco's minimum hover width when resizing.
const MIN_HOVER_WIDTH: f32 = 150.;

/// Monaco `MarkerHoverParticipant`: "Checking for quick fixes..." shows
/// once the request takes this long.
const CHECKING_QUICK_FIXES_DELAY: Duration = Duration::from_millis(200);

/// The width of a resize sash's hit area (Monaco's `Sash` size).
const SASH_SIZE: f32 = 4.;

/// The quick fix entry of the hover's status bar (Monaco
/// `MarkerHoverParticipant._renderMarkerStatusbar`).
#[derive(Clone)]
enum QuickFixStatus {
    /// A read-only editor, or no marker: no entry.
    Hidden,
    /// Asked for: a blank placeholder, so the bar doesn't flicker.
    Pending,
    /// Still asking after 200ms: "Checking for quick fixes...".
    Checking,
    /// The marker has none: "No quick fixes available".
    Unavailable,
    /// "Quick Fix...", which lists these, and the first AI action.
    Available(Rc<crate::code_action::CodeActionSet>),
}

/// Which edges a resize drag moves (Monaco `ResizableHTMLElement`'s
/// sashes: east, and north above the line or south below it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResizeEdges {
    /// The east edge: the width.
    east: bool,
    /// The north or south edge (away from the line): the height.
    vertical: bool,
}

impl ResizeEdges {
    /// The sash's pointer (Monaco's `ew-resize`, `ns-resize`, and the
    /// corner's diagonal), kept over the whole window while it's dragged.
    fn cursor(self, above: bool) -> gpui::CursorStyle {
        match (self.east, self.vertical) {
            (true, false) => gpui::CursorStyle::ResizeLeftRight,
            (false, _) => gpui::CursorStyle::ResizeUpDown,
            (true, true) if above => gpui::CursorStyle::ResizeUpRightDownLeft,
            (true, true) => gpui::CursorStyle::ResizeUpLeftDownRight,
        }
    }
}

/// What the mouse is over, for hovers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HoverTarget {
    /// Text at a position.
    Text(PointUtf16),
    /// A glyph margin lane or the line number of a row (Monaco's glyph
    /// hover).
    Gutter(u32, LaneOrLineNumber),
    /// Nothing that hovers.
    None,
}

/// What part of a row's gutter a glyph hover is about (Monaco
/// `LaneOrLineNumber`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaneOrLineNumber {
    /// A glyph margin lane: its decorations' `glyph_margin_hover_message`.
    Lane(GlyphMarginLane),
    /// The line number: its decorations' `line_number_hover_message`.
    LineNumber,
}

/// The glyph hover's widest: Monaco's markdown hover contents'
/// `max-width: var(--vscode-hover-maxWidth, 500px)`, plus their 8px
/// padding each side and the 1px border.
const GLYPH_HOVER_MAX_WIDTH: f32 = 500. + 16. + 2.;

/// Monaco `isEmptyMarkdownString` for `IMarkdownString | IMarkdownString[]`:
/// no message, or only empty ones.
fn is_empty_markdown(messages: &[MarkdownString]) -> bool {
    messages.iter().all(|message| message.value.is_empty())
}

/// A row of the hover card (Monaco `IRenderedContentHoverPartOrStatusBar`),
/// in render order: the participants' parts, then the status bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderedPart {
    Marker(usize),
    ColorPicker,
    Markdown(usize),
    InlineToolbar,
    Unicode(usize),
    Command,
    StatusBar,
}

/// Whether `part`'s row has Monaco's copy button (`HoverCopyButton`, on
/// every part but the color picker's, the inline suggestion toolbar's and
/// the inlay hint command's).
fn has_copy_button(part: RenderedPart) -> bool {
    matches!(
        part,
        RenderedPart::Marker(_) | RenderedPart::Markdown(_) | RenderedPart::Unicode(_)
    )
}

/// A tab stop inside the hover card (Monaco's elements with `tabIndex` 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoverStop {
    /// A row (`hover-row`), by [`HoverWidget::rendered_parts`] index.
    Row(usize),
    /// A row's copy button, by row.
    Copy(usize),
    /// A status bar action (`HoverAction`), by status bar entry.
    StatusAction(usize),
}

/// The copy button's label (Monaco `hover.copy`).
const COPY_LABEL: &str = "Copy";

/// A status bar action's text: `label (keys)`.
fn status_action_text(label: &str, keys: &Option<gpui::SharedString>) -> String {
    match keys {
        Some(keys) => format!("{label} ({keys})"),
        None => label.to_string(),
    }
}

/// `handles[ix]`, made (with any before it) if it's new.
fn lazy_focus<T>(handles: &mut Vec<FocusHandle>, ix: usize, cx: &mut Context<T>) -> FocusHandle {
    while handles.len() <= ix {
        handles.push(cx.focus_handle());
    }
    handles[ix].clone()
}

/// Monaco `SimpleButton` and `HoverAction`: Enter or Space presses a
/// focused button.
fn presses_button(keystroke: &gpui::Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    !(modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift)
        && matches!(keystroke.key.as_str(), "enter" | "space")
}

/// The hover card.
pub struct HoverWidget {
    /// Diagnostics at the position (Monaco's marker hover part).
    markers: Vec<crate::Marker>,
    /// The status bar's quick fix entry, for the most severe marker.
    quick_fix: QuickFixStatus,
    /// Ask for the most severe marker's quick fixes.
    quick_fix_tasks: Vec<Task<()>>,
    /// The model's URI, naming same-file related information.
    uri: Option<gpui::SharedString>,
    editor: gpui::WeakEntity<CodeEditor>,
    /// Markdown parts: decoration messages, provider hovers, warnings.
    parts: Vec<MarkdownHoverPart>,
    /// The focused row, an index into [`Self::rendered_parts`], for the
    /// verbosity actions (Monaco `focusedHoverPartIndex`).
    focused_part: Option<usize>,
    /// Each row's focus (Monaco gives every `hover-row` `tabIndex` 0), made
    /// as rows appear.
    part_focus: Vec<FocusHandle>,
    /// Each row's copy button's focus (Monaco's `HoverCopyButton` is a
    /// `SimpleButton`, `tabIndex` 0), by row, made as rows appear.
    copy_focus: Vec<FocusHandle>,
    /// Each status bar action's focus (Monaco `HoverAction`, `tabIndex` 0),
    /// by entry, made as entries appear.
    status_focus: Vec<FocusHandle>,
    /// Verbosity requests in flight per part, with their accumulated
    /// delta (Monaco `_ongoingHoverOperations`).
    verbosity_requests: HashMap<usize, (i32, Task<()>)>,
    /// Where the card is anchored, to tell which side it's on.
    anchor: PointUtf16,
    /// Monaco's glyph hover (`GlyphHoverWidget`), beside the gutter: a
    /// plain hover that isn't resizable.
    glyph: bool,
    /// The size the user resized the card to.
    size: Option<Size<Pixels>>,
    /// The largest size before a resize (Monaco `_updateMaxDimensions`).
    max_size: Size<Pixels>,
    /// A resize drag: (mouse start, size at start, card above its line,
    /// the edges it moves).
    resizing: Option<(gpui::Point<Pixels>, Size<Pixels>, bool, ResizeEdges)>,
    last_size: Rc<Cell<Option<Size<Pixels>>>>,
    /// Monaco's color picker hover part.
    color_picker: Option<Entity<crate::ColorPickerWidget>>,
    /// Monaco's unicode highlighter hover parts.
    unicode: Vec<crate::unicode_highlighter::UnicodeHover>,
    /// An inlay hint label part's command, offered as a link with the
    /// keys that run it (Monaco's "Execute Command").
    command: Option<(crate::providers::Command, String)>,
    /// The inline suggestion toolbar, over ghost text with
    /// `show_toolbar: OnHover` (Monaco `InlineCompletionsHoverParticipant`).
    inline_toolbar: Option<Entity<crate::inline_completions::toolbar::InlineToolbar>>,
    style: MarkdownStyle,
    theme: Arc<crate::Theme>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The editor's line height: one scroll step (Monaco `scrollUp`).
    line_height: Pixels,
    /// Where the card was painted, so the editor knows the mouse is on it.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The room it has in the window (it overflows the editor).
    room: crate::widgets::OverflowRoomCell,
    /// The status bar's keybinding labels, looked up in the editor's key
    /// context when rendering (Monaco `keybindingService.lookupKeybinding`
    /// for `editor.action.marker.next` and `editor.action.quickFix`).
    status_keys: StatusKeys,
    /// Where each status bar entry was last painted: the clicked link
    /// Monaco's "Quick Fix..." opens the code action menu at.
    status_links: Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>,
}

/// The labels of the keys the problem hover's actions name.
#[derive(Clone, Default)]
struct StatusKeys {
    view_problem: Option<gpui::SharedString>,
    quick_fix: Option<gpui::SharedString>,
}

impl Focusable for HoverWidget {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl HoverWidget {
    /// Monaco `ContentHoverWidget.scrollUp` and friends: scrolls by `delta`
    /// (positive scrolls down), or to the top or bottom.
    fn scroll_by(&mut self, delta: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let offset = self.scroll.offset();
        let max = self.scroll.max_offset();
        // Scroll offsets are negative, down to `-max`.
        let x = (offset.x - delta.x).clamp(-max.x, px(0.));
        let y = (offset.y - delta.y).clamp(-max.y, px(0.));
        self.scroll.set_offset(point(x, y));
        cx.notify();
    }

    fn page(&self) -> Pixels {
        self.scroll.bounds().size.height
    }

    /// Monaco `RenderedContentHoverParts._renderedParts`: the rows, the
    /// participants' parts by `hoverOrdinal` (markers 1, the color picker 2,
    /// markdown 3, inline suggestions 4, unicode 5, inlay hints 6), then the
    /// status bar if it has content.
    fn rendered_parts(&self) -> Vec<RenderedPart> {
        let mut parts: Vec<RenderedPart> =
            (0..self.markers.len()).map(RenderedPart::Marker).collect();
        if self.color_picker.is_some() {
            parts.push(RenderedPart::ColorPicker);
        }
        parts.extend((0..self.parts.len()).map(RenderedPart::Markdown));
        if self.inline_toolbar.is_some() {
            parts.push(RenderedPart::InlineToolbar);
        }
        parts.extend((0..self.unicode.len()).map(RenderedPart::Unicode));
        if self.command.is_some() {
            parts.push(RenderedPart::Command);
        }
        if !self.status_bar_entries().is_empty() {
            parts.push(RenderedPart::StatusBar);
        }
        parts
    }

    /// Row `ix`'s focus, made if it's new.
    fn part_focus(&mut self, ix: usize, cx: &mut Context<Self>) -> FocusHandle {
        lazy_focus(&mut self.part_focus, ix, cx)
    }

    /// The card's tab stops after the card itself, in Monaco's DOM order:
    /// each row, then its copy button (a marker, markdown or unicode
    /// part's); the status bar, then its actions.
    fn tab_stops(&self) -> Vec<HoverStop> {
        let mut stops = Vec::new();
        for (ix, part) in self.rendered_parts().into_iter().enumerate() {
            stops.push(HoverStop::Row(ix));
            if has_copy_button(part) {
                stops.push(HoverStop::Copy(ix));
            }
            if part == RenderedPart::StatusBar {
                stops.extend(
                    self.status_bar_entries()
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| matches!(entry, StatusBarEntry::Action { .. }))
                        .map(|(entry, _)| HoverStop::StatusAction(entry)),
                );
            }
        }
        stops
    }

    /// `stop`'s focus, if it has been made.
    fn stop_handle(&self, stop: HoverStop) -> Option<&FocusHandle> {
        match stop {
            HoverStop::Row(ix) => self.part_focus.get(ix),
            HoverStop::Copy(ix) => self.copy_focus.get(ix),
            HoverStop::StatusAction(entry) => self.status_focus.get(entry),
        }
    }

    /// The focused stop with the row it's in, if one has the focus.
    fn focused_stop_in(&self, window: &Window) -> Option<(HoverStop, usize)> {
        let mut row = 0;
        for stop in self.tab_stops() {
            if let HoverStop::Row(ix) = stop {
                row = ix;
            }
            if self.stop_handle(stop).is_some_and(|h| h.is_focused(window)) {
                return Some((stop, row));
            }
        }
        None
    }

    /// The row the focus is in, if one has it (Monaco
    /// `focusedHoverPartIndex`, set by focus events that bubble from a
    /// row's copy button or actions).
    fn focused_part_in(&self, window: &Window) -> Option<usize> {
        self.focused_stop_in(window).map(|(_, row)| row)
    }

    /// Scrolls row `ix` into view, as the browser does for a focused
    /// element (the first row to the top, rather than to the card's
    /// border).
    fn reveal_part(&mut self, ix: usize) {
        if ix == 0 {
            let offset = self.scroll.offset();
            self.scroll.set_offset(point(offset.x, px(0.)));
        } else {
            self.scroll.scroll_to_item(ix);
        }
    }

    /// Focuses row `ix` (Monaco `focusHoverPartWithIndex`), or the card with
    /// `None` (Monaco `ContentHoverWidget.focus`); returns what screen
    /// readers read for it, as they read a focused `hover-row` or the
    /// card's contents. `None` for a row that doesn't exist.
    fn focus_part(
        &mut self,
        ix: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let parts = self.rendered_parts();
        let (handle, content) = match ix {
            Some(ix) => {
                let part = *parts.get(ix)?;
                self.reveal_part(ix);
                (self.part_focus(ix, cx), self.part_accessible_content(part))
            }
            None => (self.focus_handle.clone(), self.accessible_content()),
        };
        window.focus(&handle, cx);
        self.focused_part = ix;
        cx.notify();
        Some(content)
    }

    /// Focuses `stop` (the card with `None`); returns what screen readers
    /// read for it: a row's or the card's contents, a button's label.
    fn focus_stop(
        &mut self,
        stop: Option<HoverStop>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let (handle, row, label) = match stop {
            None => return self.focus_part(None, window, cx),
            Some(HoverStop::Row(ix)) => return self.focus_part(Some(ix), window, cx),
            Some(HoverStop::Copy(ix)) => (
                lazy_focus(&mut self.copy_focus, ix, cx),
                ix,
                COPY_LABEL.to_string(),
            ),
            Some(HoverStop::StatusAction(entry)) => {
                let row = self
                    .rendered_parts()
                    .iter()
                    .position(|part| *part == RenderedPart::StatusBar)?;
                let label = match self.status_bar_entries().into_iter().nth(entry)? {
                    StatusBarEntry::Action { label, keys, .. } => status_action_text(&label, &keys),
                    StatusBarEntry::Text(_) => return None,
                };
                (lazy_focus(&mut self.status_focus, entry, cx), row, label)
            }
        };
        self.reveal_part(row);
        window.focus(&handle, cx);
        self.focused_part = Some(row);
        cx.notify();
        Some(label)
    }

    /// Tab and Shift+Tab: the browser's tab order over Monaco's card, its
    /// `hover-row`s and their buttons (each `tabIndex` 0), from the card
    /// into the first row and back; past either end, the editor. Screen
    /// readers read where the focus lands. (Markdown links aren't stops.)
    fn move_part_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let stops = self.tab_stops();
        let current = self
            .focused_stop_in(window)
            .and_then(|(stop, _)| stops.iter().position(|s| *s == stop));
        let target = match (current, forward) {
            (None, true) => Some(Some(0)),
            (Some(ix), true) => Some(Some(ix + 1)),
            (Some(0), false) => Some(None),
            (Some(ix), false) => Some(Some(ix - 1)),
            (None, false) => None,
        }
        .filter(|target| target.is_none_or(|ix| ix < stops.len()));
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        match target.and_then(|target| self.focus_stop(target.map(|ix| stops[ix]), window, cx)) {
            Some(content) if !content.is_empty() => {
                editor.update(cx, |editor, cx| editor.announce(content, cx))
            }
            Some(_) => {}
            None => {
                self.focused_part = None;
                let handle = editor.read(cx).focus_handle.clone();
                window.focus(&handle, cx);
            }
        }
    }
}

/// What asked for a hover (Monaco `HoverStartSource`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HoverSource {
    Mouse,
    Click,
    Keyboard,
}

/// Hover provider requests, with their providers for verbosity requests.
type ProviderHoverTasks = Vec<(Arc<dyn HoverProvider>, Task<anyhow::Result<Option<Hover>>>)>;

/// Monaco `isValidHover`: some non-empty contents.
fn is_valid_hover(hover: &Hover) -> bool {
    hover.contents.iter().any(|c| !c.value.trim().is_empty())
}

/// A pending color picker hover part.
type PickerTask = Option<Task<Option<Entity<crate::ColorPickerWidget>>>>;

/// What a content hover shows (Monaco `ContentHoverResult.hoverParts`), by
/// participant.
#[derive(Default)]
struct HoverContents {
    markers: Vec<crate::Marker>,
    parts: Vec<MarkdownHoverPart>,
    color_picker: Option<Entity<crate::ColorPickerWidget>>,
    unicode: Vec<crate::unicode_highlighter::UnicodeHover>,
}

impl HoverContents {
    fn is_empty(&self) -> bool {
        self.markers.is_empty()
            && self.parts.is_empty()
            && self.color_picker.is_none()
            && self.unicode.is_empty()
    }
}

/// What a content hover operation waits for (Monaco `HoverOperation`'s
/// schedulers and its async iterable).
enum HoverEvent {
    /// The hover time: compute the sync parts and show.
    Sync,
    /// Three times the hover time: show "Loading..." if still waiting.
    Loading,
    /// A provider's answer, with its index (the part's ordinal).
    Provider(usize, Arc<dyn HoverProvider>, Option<Hover>),
    /// The color picker hover part.
    ColorPicker(Option<Entity<crate::ColorPickerWidget>>),
}

/// A content hover being computed (Monaco `HoverOperation` with the
/// `ContentHoverComputer`): its result so far.
#[derive(Default)]
struct ContentHoverOperation {
    position: PointUtf16,
    /// The word at the position (or the empty range there).
    word: Range<PointUtf16>,
    /// Whether the hover time passed and the sync parts are in.
    synced: bool,
    /// The providers and color picker still to answer.
    pending: usize,
    /// Whether "Loading..." shows (`WaitingForAsyncShowingLoading`).
    loading: bool,
    markers: Vec<crate::Marker>,
    sync_parts: Vec<MarkdownHoverPart>,
    provider_parts: Vec<MarkdownHoverPart>,
    /// The first provider's range, by ordinal.
    provider_range: Option<(usize, Range<PointUtf16>)>,
    color_picker: Option<Entity<crate::ColorPickerWidget>>,
    unicode: Vec<crate::unicode_highlighter::UnicodeHover>,
    toolbar: Option<Entity<crate::inline_completions::toolbar::InlineToolbar>>,
}

impl ContentHoverOperation {
    /// Monaco's `Idle` state after the hover time: all answers are in.
    fn is_complete(&self) -> bool {
        self.synced && self.pending == 0
    }

    /// Monaco `MarkdownHoverParticipant._getMarkdownHovers`: provider
    /// `ordinal`'s part.
    fn add_provider_hover(
        &mut self,
        ordinal: usize,
        provider: Arc<dyn HoverProvider>,
        mut hover: Hover,
    ) {
        // Monaco's standalone `registerHoverProvider`: a hover without a
        // range is about the word at the position, else the empty range
        // there.
        let range = hover.range.get_or_insert_with(|| self.word.clone()).clone();
        if self
            .provider_range
            .as_ref()
            .is_none_or(|(first, _)| ordinal < *first)
        {
            self.provider_range = Some((ordinal, range.clone()));
        }
        self.provider_parts.push(MarkdownHoverPart {
            contents: hover.contents.clone(),
            range: Some(range),
            source: Some(HoverPartSource {
                provider,
                hover,
                position: self.position,
            }),
            long_line_warning: false,
            ordinal,
        });
    }
}

/// A marker's range on `row`, as Monaco's marker hover part has it
/// (`MarkerHoverParticipant.computeSync`): clipped to the line.
fn range_on_row(range: &Range<PointUtf16>, row: u32, line_len: u32) -> Range<PointUtf16> {
    let start = if range.start.row == row {
        range.start
    } else {
        PointUtf16::new(row, 0)
    };
    let end = if range.end.row == row {
        range.end
    } else {
        PointUtf16::new(row, line_len)
    };
    start..end
}

impl MarkdownHoverPart {
    /// The part's range as Monaco's `MarkdownHover` has it for a hover on
    /// `row`: a decoration's is clipped to the line
    /// (`MarkdownHoverParticipant.computeSync`), a provider's isn't.
    fn range_on_row(&self, row: u32, line_len: u32) -> Option<Range<PointUtf16>> {
        let range = self.range.as_ref()?;
        Some(match self.source {
            Some(_) => range.clone(),
            None => range_on_row(range, row, line_len),
        })
    }
}

impl Render for HoverWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.markers.is_empty()
            && let Some(editor) = self.editor.upgrade()
        {
            let editor = editor.read(cx);
            let label = |action: &dyn gpui::Action| {
                crate::keybinding_label::editor_command_label(action, None, editor, window, cx)
            };
            self.status_keys = StatusKeys {
                view_problem: label(&crate::actions::GoToNextMarker),
                quick_fix: label(&crate::actions::QuickFix),
            };
        }
        let colors = self.theme.colors.clone();
        let parts = self.rendered_parts();
        // Monaco's focus listeners on each row (`FOCUS_IN` / `FOCUS_OUT`).
        self.focused_part = self.focused_part_in(window);
        // Monaco's `.hover-row`s: the parts in participant order, then the
        // status bar.
        let mut rows = Vec::with_capacity(parts.len());
        for (index, part) in parts.iter().enumerate() {
            let focused = self.focused_part == Some(index);
            let row = match *part {
                RenderedPart::Marker(ix) => {
                    let marker = &self.markers[ix];
                    let copy = marker_accessible_content(marker, self.uri.as_deref());
                    let row = self.render_marker(ix, marker);
                    self.copy_row(("marker-row", ix), index, row, copy, focused, window, cx)
                }
                RenderedPart::ColorPicker => match &self.color_picker {
                    Some(picker) => picker.clone().into_any_element(),
                    None => continue,
                },
                RenderedPart::Unicode(ix) => {
                    let row = self.render_unicode(ix, cx);
                    let copy = self.unicode[ix].reason.to_string();
                    self.copy_row(("unicode-row", ix), index, row, copy, focused, window, cx)
                }
                RenderedPart::Markdown(ix) => {
                    let row = self.render_markdown_part(ix, cx);
                    let copy = markdown_accessible_content(&self.parts[ix]);
                    self.copy_row(("markdown-row", ix), index, row, copy, focused, window, cx)
                }
                RenderedPart::InlineToolbar => match &self.inline_toolbar {
                    Some(toolbar) => div()
                        .px(px(8.))
                        .py(px(4.))
                        .child(toolbar.clone())
                        .into_any_element(),
                    None => continue,
                },
                RenderedPart::Command => match self.render_command() {
                    Some(row) => row,
                    None => continue,
                },
                RenderedPart::StatusBar => match self.render_status_bar(window, cx) {
                    Some(row) => row,
                    None => continue,
                },
            };
            let label = self.part_accessible_content(*part);
            let focus = self.part_focus(index, cx);
            // The outline is the row's own, not its buttons'.
            let row_focused = focus.is_focused(window);
            // Monaco `.hover-row:not(:first-child):not(:empty)`: a top
            // border across the card's full width, in the hover border at
            // half alpha (`hoverContribution.ts`). A focused row has the
            // focus outline (`[tabindex="0"]:focus`, inset by its width).
            rows.push(
                div()
                    .id(("hover-part", index))
                    .debug_selector(move || format!("hover-part-{index}"))
                    .track_focus(&focus)
                    .role(gpui::Role::Group)
                    .aria_label(gpui::SharedString::from(label))
                    .relative()
                    .when(index > 0, |this| {
                        this.border_t_1()
                            .border_color(colors.hover_border.opacity(0.5))
                    })
                    .child(row)
                    .when(row_focused, |this| {
                        this.child(
                            div()
                                .absolute()
                                .inset_0()
                                .border_1()
                                .border_color(colors.focus_border),
                        )
                    }),
            );
        }
        let bounds = self.bounds.clone();
        let line = self.line_height;
        let resize = if self.glyph {
            Vec::new()
        } else {
            self.render_resize_sashes(cx)
        };
        let extent = crate::scrollbars::extent(&self.scroll);
        let scroll = self.scroll.clone();
        let view = cx.entity().downgrade();
        // Monaco `ResizableContentWidget`: the card is resizable; once
        // resized it keeps that size, else it fits its content up to the
        // maximum.
        // Monaco `_findMaximumRenderingHeight`: no taller than the room
        // above or below the line in the window.
        let card = match self.size {
            Some(size) => div().w(size.width).h(size.height),
            None => div()
                .max_w(self.room.cap_width(self.max_size.width, px(100.)))
                .max_h(self.room.cap_height(self.max_size.height, line * 2.)),
        };
        let colors = &self.theme.colors;
        let card = card
            .id("hover-card")
            .track_scroll(&self.scroll)
            .overflow_scroll()
            .flex()
            .flex_col()
            // `.monaco-hover`: `border-radius: var(--vscode-cornerRadius-large)`
            // and `box-shadow: var(--vscode-shadow-lg)`.
            .rounded(card::CORNER_RADIUS)
            .shadow(card::shadow_lg())
            .bg(colors.hover_background)
            .text_color(colors.hover_foreground)
            .border_1()
            .border_color(if self.focus_handle.is_focused(window) {
                colors.focus_border
            } else {
                colors.hover_border
            })
            .children(rows);
        // Monaco's hover is a `DomScrollableElement`: 10px scrollbars, inside
        // the border.
        let scrollbars = crate::scrollbars::scrollbars(
            "hover",
            HOVER_SCROLL_GROUP.into(),
            &self.scroll,
            px(10.),
            px(1.),
            colors,
        );
        // Monaco's `.monaco-hover` is a `role="tooltip"` whose contents
        // screen readers read.
        let label = self.accessible_content();
        div()
            .id("hover")
            .group(HOVER_SCROLL_GROUP)
            .role(gpui::Role::Tooltip)
            .aria_label(gpui::SharedString::from(label))
            // With the editor's keys, for its preconditions.
            .key_context(crate::context_keys::widget_key_context(
                HOVER_KEY_CONTEXT,
                &crate::context_keys::editor_keys_for_widgets(&self.editor, cx),
            ))
            .track_focus(&self.focus_handle)
            .relative()
            .text_size(self.style.font_size)
            // The editor's line height (Monaco sets `line-height` to it over
            // the font size), not GPUI's default.
            .line_height(gpui::relative(self.style.line_height))
            .on_action(cx.listener(move |this, _: &ScrollUpHover, _, cx| {
                this.scroll_by(point(px(0.), -line), cx)
            }))
            .on_action(cx.listener(move |this, _: &ScrollDownHover, _, cx| {
                this.scroll_by(point(px(0.), line), cx)
            }))
            .on_action(cx.listener(move |this, _: &ScrollLeftHover, _, cx| {
                this.scroll_by(point(-line, px(0.)), cx)
            }))
            .on_action(cx.listener(move |this, _: &ScrollRightHover, _, cx| {
                this.scroll_by(point(line, px(0.)), cx)
            }))
            .on_action(cx.listener(|this, _: &PageUpHover, _, cx| {
                let page = this.page();
                this.scroll_by(point(px(0.), -page), cx)
            }))
            .on_action(cx.listener(|this, _: &PageDownHover, _, cx| {
                let page = this.page();
                this.scroll_by(point(px(0.), page), cx)
            }))
            .on_action(cx.listener(|this, _: &GoToTopHover, _, cx| {
                this.scroll_by(point(px(0.), px(-1e9)), cx)
            }))
            .on_action(cx.listener(|this, _: &GoToBottomHover, _, cx| {
                this.scroll_by(point(px(0.), px(1e9)), cx)
            }))
            // Tab and Shift+Tab (the editor's `tab` and `outdent` need
            // `editorTextFocus`, so the browser's apply).
            .on_action(cx.listener(|this, _: &FocusNextElement, window, cx| {
                this.move_part_focus(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusPreviousElement, window, cx| {
                this.move_part_focus(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &HideHover, window, cx| {
                // Monaco: Escape in the focused hover closes it and returns
                // focus to the editor.
                if let Some(editor) = this.editor.upgrade() {
                    editor.update(cx, |editor, cx| {
                        editor.hide_hover(cx);
                        window.focus(&editor.focus_handle, cx);
                    });
                }
            }))
            .child(
                canvas(
                    move |b, _, _| bounds.set(Some(b)),
                    move |_, _, window, cx| {
                        crate::scrollbars::follow_extent(&scroll, extent, view, window, cx)
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(card)
            .children(scrollbars)
            .children(resize)
    }
}

impl HoverWidget {
    /// Monaco `HoverCopyButton`: a hover row with a copy button that shows
    /// while the mouse is over the row or the focus is in it. The button
    /// (row `index`'s) is a `SimpleButton`: a tab stop that a click, Enter
    /// or Space presses.
    #[allow(clippy::too_many_arguments)]
    fn copy_row(
        &mut self,
        id: (&'static str, usize),
        index: usize,
        row: gpui::AnyElement,
        text: String,
        focused: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let focus = lazy_focus(&mut self.copy_focus, index, cx);
        let button_focused = focus.is_focused(window);
        let colors = &self.theme.colors;
        let group: gpui::SharedString = format!("{}-{}", id.0, id.1).into();
        let key_text = text.clone();
        div()
            .group(group.clone())
            .relative()
            .pr(px(20.))
            .child(row)
            .child(
                div()
                    .id(id)
                    .absolute()
                    // `.hover-copy-button { top: 4px; right: 4px }`.
                    .top(px(4.))
                    .right(px(4.))
                    .px(px(4.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .cursor_pointer()
                    .opacity(if focused { 1. } else { 0. })
                    .group_hover(group.clone(), |style| style.opacity(1.))
                    .hover(|style| colors.toolbar_hover(style))
                    .tooltip(crate::tooltip::tooltip("Copy", colors))
                    .debug_selector({
                        let group = group.clone();
                        move || format!("hover-copy-{group}")
                    })
                    .track_focus(&focus)
                    .role(gpui::Role::Button)
                    .aria_label(COPY_LABEL)
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.copy_content(&text, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        if presses_button(&event.keystroke) {
                            cx.stop_propagation();
                            this.copy_content(&key_text, cx);
                        }
                    }))
                    // `.hover-copy-button`: a 16px `copy` codicon.
                    .child(codicon(Codicon::Copy))
                    // `.hover-copy-button:focus`: a 1px `focusBorder`
                    // outline, inset by its width.
                    .when(button_focused, |this| {
                        this.child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(3.))
                                .border_1()
                                .border_color(colors.focus_border),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Monaco `HoverCopyButton._copyContent`: copies `text` and tells
    /// screen readers.
    fn copy_content(&self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        // Monaco `_copyContent`'s `status`.
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| {
                editor.announce_status("Copied to clipboard", cx)
            });
        }
    }

    /// Monaco `UnicodeHighlighterHoverParticipant`: a markdown hover, the
    /// reason then the "Adjust settings" link after it.
    fn render_unicode(&self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let hover = &self.unicode[ix];
        let editor = self.editor.clone();
        let part = hover.clone();
        let markdown = MarkdownString::new(format!(
            "{} [Adjust settings]({ADJUST_UNICODE_SETTINGS})",
            hover.reason
        ));
        let on_link: crate::markdown::LinkHandler = Rc::new(move |url, window, cx| {
            if url != ADJUST_UNICODE_SETTINGS {
                return false;
            }
            if let Some(editor) = editor.upgrade() {
                let part = part.clone();
                editor.update(cx, |editor, cx| {
                    editor.hide_hover(cx);
                    editor.show_unicode_exclude_options(part, window, cx);
                });
            }
            true
        });
        div()
            .id(("unicode-part", ix))
            .px(px(8.))
            .py(px(4.))
            .child(crate::markdown::render_markdown_with_links(
                &markdown,
                &self.style,
                &self.theme,
                Some(on_link),
                cx,
            ))
            .into_any_element()
    }

    /// An inlay hint label part's command: Monaco's "Execute Command" link
    /// with the keys that run it.
    fn render_command(&self) -> Option<gpui::AnyElement> {
        let (command, keys) = self.command.as_ref()?;
        let editor = self.editor.clone();
        let run = command.clone();
        Some(
            div()
                .flex()
                .px(px(8.))
                .py(px(4.))
                .child(
                    div()
                        .id("inlay-command")
                        .cursor_pointer()
                        .text_color(self.style.link)
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.hide_hover(cx);
                                    editor.run_inlay_hint_command(&run, cx);
                                });
                            }
                        })
                        .child("Execute Command"),
                )
                .child(keys.clone())
                .into_any_element(),
        )
    }

    /// Monaco `MarkdownHoverParticipant`: the part's markdown, with
    /// verbosity controls when its provider supports them and the long
    /// line warning's "Don't Show Again".
    fn render_markdown_part(&self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = &self.theme.colors;
        let part = &self.parts[ix];
        let link = self.style.link;
        // Each content is a `div.hover-contents` with `padding: 4px 8px`.
        let mut contents = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(8.))
            .py(px(4.));
        for content in &part.contents {
            contents = contents.child(render_markdown(content, &self.style, &self.theme, cx));
        }
        if part.long_line_warning {
            let editor = self.editor.clone();
            contents = contents.child(
                div()
                    .id(("long-line-warning", ix))
                    .cursor_pointer()
                    .text_color(link)
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        if let Some(editor) = editor.upgrade() {
                            editor.update(cx, |editor, cx| editor.hide_long_line_warning_hover(cx));
                        }
                    })
                    .child("Don't Show Again"),
            );
        }
        // Monaco `_renderHoverExpansionAction`: + and - beside the part.
        let verbosity = part
            .source
            .as_ref()
            .filter(|source| source.supports(true) || source.supports(false))
            .map(|source| {
                let action = |increase: bool| {
                    let enabled = source.supports(increase);
                    let editor = self.editor.clone();
                    let label = if increase {
                        "Increase Hover Verbosity"
                    } else {
                        "Decrease Hover Verbosity"
                    };
                    div()
                        .id((
                            if increase {
                                "verbosity-up"
                            } else {
                                "verbosity-down"
                            },
                            ix,
                        ))
                        .when(enabled, |this| this.cursor_pointer().text_color(link))
                        .when(!enabled, |this| this.opacity(0.6))
                        .tooltip(crate::tooltip::tooltip(label, colors))
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if !enabled {
                                return;
                            }
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.update_hover_verbosity_level(increase, Some(ix), cx)
                                });
                            }
                        })
                        // `add` and `remove` codicons at 11px.
                        .child(
                            codicon(if increase {
                                Codicon::Add
                            } else {
                                Codicon::Remove
                            })
                            .size(px(11.)),
                        )
                };
                div()
                    .flex_none()
                    .w(px(22.))
                    .px(px(5.))
                    .border_r_1()
                    .border_color(colors.hover_border)
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(action(true))
                    .child(action(false))
            });
        div()
            .id(("markdown-part", ix))
            .flex()
            .children(verbosity)
            .child(contents)
            .into_any_element()
    }

    /// Monaco `ResizableContentWidget`'s sashes: the full east edge, and the
    /// full edge away from the line (north above it, south below it), with
    /// the corner between them moving both.
    fn render_resize_sashes(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let above = self.shown_above(cx);
        let sash_size = px(SASH_SIZE);
        let offset = -sash_size / 2.;
        let sash = |id: &'static str, edges: ResizeEdges| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .absolute()
                .cursor(edges.cursor(above))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if let Some(bounds) = this.bounds.get() {
                            this.resizing = Some((event.position, bounds.size, above, edges));
                            cx.notify();
                        }
                    }),
                )
        };
        let east = sash(
            "hover-resize-east",
            ResizeEdges {
                east: true,
                vertical: false,
            },
        )
        .top_0()
        .bottom_0()
        .right(offset)
        .w(sash_size);
        let vertical = sash(
            if above {
                "hover-resize-north"
            } else {
                "hover-resize-south"
            },
            ResizeEdges {
                east: false,
                vertical: true,
            },
        )
        .left_0()
        .right_0()
        .h(sash_size)
        .when(above, |this| this.top(offset))
        .when(!above, |this| this.bottom(offset));
        let corner = sash(
            "hover-resize",
            ResizeEdges {
                east: true,
                vertical: true,
            },
        )
        .right(-sash_size)
        .size(sash_size * 2.)
        .when(above, |this| this.top(-sash_size))
        .when(!above, |this| this.bottom(-sash_size));
        let mut sashes = vec![
            east.into_any_element(),
            vertical.into_any_element(),
            corner.into_any_element(),
        ];
        // While dragging: follow the mouse anywhere in the window, with the
        // sash's pointer.
        if let Some((start, start_size, above, edges)) = self.resizing {
            let min_height = self.line_height + px(8.);
            let editor = self.editor.clone();
            sashes.push(
                crate::pointer_capture::capture(
                    cx.entity().downgrade(),
                    self.captured_cursor(),
                    move |this: &mut Self, event, cx| {
                        let editor_size = editor
                            .upgrade()
                            .and_then(|editor| editor.read(cx).layout_size());
                        let delta = event.position - start;
                        let dx = if edges.east { delta.x } else { px(0.) };
                        let dy = match (edges.vertical, above) {
                            (false, _) => px(0.),
                            (true, true) => -delta.y,
                            (true, false) => delta.y,
                        };
                        let mut width = (start_size.width + dx).max(px(MIN_HOVER_WIDTH));
                        let mut height = (start_size.height + dy).max(min_height);
                        if let Some(max) = editor_size {
                            width = width.min(max.width.max(px(MIN_HOVER_WIDTH)));
                            height = height.min(max.height.max(min_height));
                        }
                        let new_size = size(width, height);
                        this.size = Some(new_size);
                        this.last_size.set(Some(new_size));
                        cx.notify();
                    },
                    |this, cx| {
                        this.resizing = None;
                        cx.notify();
                    },
                )
                .into_any_element(),
            );
        }
        sashes
    }

    /// The pointer over the whole window while a sash is dragged (Monaco's
    /// pointer capture on the sash).
    pub(crate) fn captured_cursor(&self) -> Option<gpui::CursorStyle> {
        self.resizing
            .map(|(_, _, above, edges)| edges.cursor(above))
    }

    /// Whether the card was laid out above its anchor's line.
    fn shown_above(&self, cx: &gpui::App) -> bool {
        let (Some(bounds), Some(editor)) = (self.bounds.get(), self.editor.upgrade()) else {
            return false;
        };
        editor
            .read(cx)
            .scrolled_visible_position(self.anchor)
            .is_some_and(|line| bounds.bottom() <= line.y + px(1.))
    }

    /// Monaco `MarkerHoverParticipant._renderMarkerHover`, in the editor's
    /// font: the message (`white-space: pre-wrap`), its source and code
    /// dimmed (the code a link with a target), and each related
    /// information's `file(line, col): ` as a link before its message.
    fn render_marker(&self, ix: usize, marker: &crate::Marker) -> gpui::AnyElement {
        let link = self.style.link;
        let details = (marker.source.is_some() || marker.code.is_some()).then(|| {
            div()
                .flex_none()
                .flex()
                .pl(px(6.))
                .opacity(0.6)
                .children(marker.source.clone())
                .children(marker.code.as_ref().map(|code| {
                    crate::markers::code_element(
                        code,
                        marker.code_target.as_ref(),
                        link,
                        self.editor.clone(),
                    )
                }))
        });
        let mut column = div()
            .id(("marker", ix))
            .flex()
            .flex_col()
            .px(px(8.))
            .py(px(4.))
            .font_family(self.style.code_font.clone())
            .child(
                div()
                    .flex()
                    .child(div().min_w_0().child(marker.message.clone()))
                    .children(details),
            );
        for (ix, related) in marker.related_information.iter().enumerate() {
            let editor = self.editor.clone();
            let location = related.location();
            column = column.child(
                div()
                    .mt(px(8.))
                    .flex()
                    .child(
                        div()
                            .id(("related", ix))
                            .debug_selector(move || format!("hover-related-{ix}"))
                            .role(gpui::Role::Link)
                            .flex_none()
                            .cursor_pointer()
                            .text_color(link)
                            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                if let Some(editor) = editor.upgrade() {
                                    editor.update(cx, |editor, cx| {
                                        editor.hide_hover(cx);
                                        editor.open_location(&location, true, cx);
                                    });
                                }
                            })
                            .child(related.location_label(self.uri.as_deref())),
                    )
                    .child(div().min_w_0().child(related.message.clone())),
            );
        }
        column.into_any_element()
    }

    /// The status bar's entries (Monaco `_renderMarkerStatusbar` for the
    /// most severe marker): "View Problem" for errors, warnings and infos,
    /// then the quick fix entry.
    fn status_bar_entries(&self) -> Vec<StatusBarEntry> {
        let Some(marker) = most_severe(&self.markers) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        if marker.severity >= crate::MarkerSeverity::Info {
            entries.push(StatusBarEntry::Action {
                label: "View Problem".into(),
                keys: self.status_keys.view_problem.clone(),
                icon: None,
                run: StatusBarRun::ViewProblem(marker.clone()),
            });
        }
        match &self.quick_fix {
            QuickFixStatus::Hidden => {}
            // Monaco's `&nbsp;` placeholder.
            QuickFixStatus::Pending => entries.push(StatusBarEntry::Text("\u{a0}")),
            QuickFixStatus::Checking => {
                entries.push(StatusBarEntry::Text("Checking for quick fixes..."))
            }
            QuickFixStatus::Unavailable => {
                entries.push(StatusBarEntry::Text("No quick fixes available"))
            }
            QuickFixStatus::Available(set) => {
                entries.push(StatusBarEntry::Action {
                    label: "Quick Fix...".into(),
                    keys: self.status_keys.quick_fix.clone(),
                    icon: None,
                    run: StatusBarRun::QuickFix(set.clone(), marker.range.start),
                });
                if let Some(item) = set.valid_ai() {
                    entries.push(StatusBarEntry::Action {
                        label: item.action.title.clone(),
                        keys: None,
                        icon: Some(Codicon::Sparkle),
                        run: StatusBarRun::Apply(Box::new(item.clone())),
                    });
                }
            }
        }
        entries
    }

    /// Monaco `EditorHoverStatusBar`: `div.hover-row.status-bar` on
    /// `editorHoverWidget.statusBarBackground`, 12px text on 22px rows,
    /// each action a link `label (keys)`: a tab stop (Monaco `HoverAction`,
    /// `tabIndex` 0) that a click, Enter or Space runs.
    fn render_status_bar(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let entries = self.status_bar_entries();
        if entries.is_empty() {
            return None;
        }
        let focus: Vec<FocusHandle> = (0..entries.len())
            .map(|ix| lazy_focus(&mut self.status_focus, ix, cx))
            .collect();
        let colors = &self.theme.colors;
        let link = self.style.link;
        let actions = entries.into_iter().enumerate().map(|(ix, entry)| {
            let StatusBarEntry::Action {
                label,
                keys,
                icon,
                run,
            } = entry
            else {
                let StatusBarEntry::Text(text) = entry else {
                    unreachable!()
                };
                return div().mr(px(16.)).child(text).into_any_element();
            };
            let editor = self.editor.clone();
            let run = Rc::new(run);
            let links = self.status_links.clone();
            let painted = self.status_links.clone();
            // Monaco's `run(target)`: the action's element.
            let run_action = move |cx: &mut gpui::App| {
                let target = links.borrow().get(ix).copied().flatten();
                if let Some(editor) = editor.upgrade() {
                    let run = run.clone();
                    editor.update(cx, |editor, cx| run.run(target, editor, cx));
                }
            };
            let on_key = run_action.clone();
            let focused = focus[ix].is_focused(window);
            div()
                .id(("hover-status-action", ix))
                .debug_selector(move || format!("hover-status-action-{ix}"))
                .track_focus(&focus[ix])
                .role(gpui::Role::Button)
                .relative()
                .flex()
                .items_center()
                .mr(px(16.))
                .cursor_pointer()
                .text_color(link)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    run_action(cx);
                })
                .on_key_down(move |event: &gpui::KeyDownEvent, _, cx| {
                    if presses_button(&event.keystroke) {
                        cx.stop_propagation();
                        on_key(cx);
                    }
                })
                .when(focused, |this| {
                    this.child(
                        div()
                            .absolute()
                            .inset_0()
                            .border_1()
                            .border_color(colors.focus_border),
                    )
                })
                .child(
                    canvas(
                        move |bounds, _, _| {
                            let mut links = painted.borrow_mut();
                            if links.len() <= ix {
                                links.resize(ix + 1, None);
                            }
                            links[ix] = Some(bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .children(icon.map(|icon| div().pr(px(4.)).child(codicon(icon))))
                .child(status_action_text(&label, &keys))
                .into_any_element()
        });
        let radius = card::CORNER_RADIUS - px(1.);
        Some(
            div()
                .id("hover-status-bar")
                .debug_selector(|| "hover-status-bar".into())
                .flex()
                .px(px(8.))
                .bg(colors.hover_status_bar_background)
                .rounded_bl(radius)
                .rounded_br(radius)
                .text_size(px(12.))
                .line_height(px(22.))
                .whitespace_nowrap()
                .children(actions)
                .into_any_element(),
        )
    }

    /// Monaco `RenderedContentHoverParts.getAccessibleContent`: each row's
    /// accessible content.
    fn accessible_content(&self) -> String {
        let mut content: Vec<String> = self
            .rendered_parts()
            .into_iter()
            .map(|part| self.part_accessible_content(part))
            .collect();
        content.retain(|c| !c.is_empty());
        content.join("\n\n")
    }

    /// Monaco `getAccessibleHoverContentAtIndex`: a part's participant's
    /// accessible content, or the status bar's description.
    fn part_accessible_content(&self, part: RenderedPart) -> String {
        match part {
            RenderedPart::Marker(ix) => {
                marker_accessible_content(&self.markers[ix], self.uri.as_deref())
            }
            // `HoverColorPickerParticipant.getAccessibleContent`.
            RenderedPart::ColorPicker => "There is a color picker here.".into(),
            RenderedPart::Unicode(ix) => self.unicode[ix].reason.to_string(),
            RenderedPart::Markdown(ix) => markdown_accessible_content(&self.parts[ix]),
            // `InlineCompletionsHoverParticipant.getAccessibleContent`.
            RenderedPart::InlineToolbar => "There are inline completions here".into(),
            RenderedPart::Command => match &self.command {
                Some((_, keys)) => format!("Execute Command {keys}").trim_end().to_string(),
                None => String::new(),
            },
            RenderedPart::StatusBar => {
                let mut description = vec!["This is a hover status bar.".to_string()];
                for entry in self.status_bar_entries() {
                    match entry {
                        StatusBarEntry::Action {
                            label,
                            keys: Some(keys),
                            ..
                        } => description.push(format!(
                            "It has an action with label {label} and keybinding {keys}."
                        )),
                        StatusBarEntry::Action { label, .. } => {
                            description.push(format!("It has an action with label {label}."))
                        }
                        StatusBarEntry::Text(_) => {}
                    }
                }
                description.join("\n")
            }
        }
    }
}

/// An entry of the hover's status bar.
// Built per render and dropped right away; boxing would cost more.
#[allow(clippy::large_enum_variant)]
enum StatusBarEntry {
    /// Monaco `HoverAction`: a link, with its keybinding and icon.
    Action {
        label: gpui::SharedString,
        keys: Option<gpui::SharedString>,
        icon: Option<Codicon>,
        run: StatusBarRun,
    },
    /// Monaco's quick fix placeholder text.
    Text(&'static str),
}

/// What a status bar action does.
enum StatusBarRun {
    /// "View Problem": the marker zone at it.
    ViewProblem(crate::Marker),
    /// "Quick Fix...": the marker's quick fixes in the code action menu.
    QuickFix(Rc<crate::code_action::CodeActionSet>, PointUtf16),
    /// An AI quick fix, applied right away.
    Apply(Box<crate::code_action::Item>),
}

impl StatusBarRun {
    /// Monaco: hides the hover, then runs; `target` is where the action's
    /// link was painted.
    fn run(
        &self,
        target: Option<Bounds<Pixels>>,
        editor: &mut CodeEditor,
        cx: &mut Context<CodeEditor>,
    ) {
        use crate::code_action::MenuAnchor;
        editor.hide_hover(cx);
        match self {
            StatusBarRun::ViewProblem(marker) => {
                editor.show_marker(marker, cx);
                // Monaco: `this._editor.focus()`.
                editor.refocus = true;
            }
            // Monaco: at the clicked link (`getDomNodePagePosition(target)`),
            // else at the marker.
            StatusBarRun::QuickFix(set, position) => {
                let at = match target {
                    Some(bounds) => MenuAnchor::Window {
                        bounds,
                        position: *position,
                    },
                    None => MenuAnchor::Text(*position),
                };
                editor.show_marker_quick_fixes(set.clone(), at, cx)
            }
            StatusBarRun::Apply(item) => {
                editor.apply_code_action((**item).clone(), false, false, cx)
            }
        }
    }
}

/// The most severe of `markers`, the first of equals (Monaco sorts the
/// marker parts by `MarkerSeverity.compare` for the status bar).
fn most_severe(markers: &[crate::Marker]) -> Option<&crate::Marker> {
    markers.iter().fold(None, |best, marker| match best {
        Some(best) if best.severity >= marker.severity => Some(best),
        _ => Some(marker),
    })
}

/// Monaco `MarkdownHoverParticipant.getAccessibleContent`: the contents, a
/// line each; also what the copy button copies.
fn markdown_accessible_content(part: &MarkdownHoverPart) -> String {
    part.contents
        .iter()
        .map(|c| c.value.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Monaco `MarkerHoverParticipant.getAccessibleContent`: the message, then
/// each related information as `file(line, col): message`; also what the
/// copy button copies.
fn marker_accessible_content(marker: &crate::Marker, uri: Option<&str>) -> String {
    let mut lines = vec![marker.message.to_string()];
    lines.extend(
        marker
            .related_information
            .iter()
            .map(|related| format!("{}{}", related.location_label(uri), related.message)),
    );
    lines.join("\n")
}

use gpui::prelude::FluentBuilder as _;

impl CodeEditor {
    /// The markdown style of the editor's widgets; code blocks without a
    /// language are in the model's (Monaco renders their markdown with the
    /// editor as context).
    pub(crate) fn markdown_style(&self, cx: &gpui::App) -> MarkdownStyle {
        let colors = &self.theme.colors;
        let font_size = self.options.font_size;
        MarkdownStyle {
            foreground: colors.hover_foreground,
            link: colors.text_link_foreground,
            code_background: colors.text_code_block_background,
            // Monaco's hover CSS doesn't round inline code.
            code_corner_radius: px(0.),
            code_font: self.options.font_family.clone(),
            font_size,
            // Monaco `ContentHoverWidget._updateFont` and
            // `ParameterHintsWidget.updateFont`: the editor's line height
            // over its font size.
            line_height: f32::from(self.line_height()) / f32::from(font_size),
            separator: colors.text_separator_foreground,
            quote_background: colors.text_block_quote_background,
            quote_border: colors.text_block_quote_border,
            code_language: self.model().read(cx).language_id().cloned(),
        }
    }

    /// The unicode highlighter's hover parts at `position`.
    fn unicode_hovers_at(
        &self,
        position: PointUtf16,
        cx: &gpui::App,
    ) -> Vec<crate::unicode_highlighter::UnicodeHover> {
        let offset = self.snapshot.point_utf16_to_offset(position);
        self.unicode_hovers(offset, cx)
    }

    /// The unicode hover parts' reasons in the visible hover card.
    #[doc(hidden)]
    pub fn hover_unicode_reasons_for_tests(&self, cx: &gpui::App) -> Vec<gpui::SharedString> {
        self.hover
            .widget
            .as_ref()
            .map(|(widget, _)| {
                widget
                    .read(cx)
                    .unicode
                    .iter()
                    .map(|u| u.reason.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The diagnostics in the visible hover card.
    #[doc(hidden)]
    pub fn hover_markers_for_tests(&self, cx: &gpui::App) -> Vec<crate::Marker> {
        self.hover
            .widget
            .as_ref()
            .map(|(widget, _)| widget.read(cx).markers.clone())
            .unwrap_or_default()
    }

    /// The markdown parts' contents in the visible hover card.
    #[doc(hidden)]
    pub fn hover_parts_for_tests(&self, cx: &gpui::App) -> Vec<Vec<gpui::SharedString>> {
        self.hover
            .widget
            .as_ref()
            .map(|(widget, _)| {
                widget
                    .read(cx)
                    .parts
                    .iter()
                    .map(|part| part.contents.iter().map(|c| c.value.clone()).collect())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Monaco `MarkdownHoverParticipant.computeSync`'s long line warnings:
    /// past `stop_rendering_line_after` on a line that long, else on a line
    /// too long to tokenize.
    fn long_line_warnings(&self, position: PointUtf16, cx: &gpui::App) -> Vec<MarkdownHoverPart> {
        if !self.options.hover.show_long_line_warning {
            return Vec::new();
        }
        let rope = self.snapshot.rope();
        if position.row >= rope.line_count() {
            return Vec::new();
        }
        let length = rope.line_len_utf16(position.row);
        let column = position.column + 1;
        let message = match self.options.stop_rendering_line_after {
            Some(stop) if length > stop && column >= stop => STOPPED_RENDERING,
            _ if length >= self.model().read(cx).options().max_tokenization_line_length => {
                TOO_MANY_CHARACTERS
            }
            _ => return Vec::new(),
        };
        vec![MarkdownHoverPart {
            contents: vec![MarkdownString::new(message)],
            source: None,
            long_line_warning: true,
            range: Some(position..position),
            ordinal: 0,
        }]
    }

    /// Monaco `editor.action.hideLongLineWarningHover` (the warning's
    /// "Don't Show Again"): turns off `hover.show_long_line_warning`.
    pub fn hide_long_line_warning_hover(&mut self, cx: &mut Context<Self>) {
        self.hide_hover(cx);
        let mut options = self.options.clone();
        options.hover.show_long_line_warning = false;
        self.update_options(options, cx);
    }

    /// Monaco `editor.action.increaseHoverVerbosityLevel` and
    /// `decreaseHoverVerbosityLevel`: asks the provider of the hover's row
    /// `index` (else the focused row, else the first markdown part that
    /// supports it) for more or less detail and replaces the part with the
    /// answer; rows that aren't a provider's markdown are left alone.
    /// Repeated requests before an answer add up.
    pub fn update_hover_verbosity_level(
        &mut self,
        increase: bool,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some((widget, _)) = &self.hover.widget else {
            return;
        };
        let widget = widget.clone();
        // Monaco `_normalizedIndexToMarkdownHoverIndexRange`.
        let markdown = match index.or(widget.read(cx).focused_part) {
            Some(row) => match widget.read(cx).rendered_parts().get(row) {
                Some(RenderedPart::Markdown(ix)) => Some(*ix),
                _ => return,
            },
            None => None,
        };
        self.update_markdown_hover_verbosity(&widget, increase, markdown, cx);
    }

    /// Asks the provider of markdown part `index` (else the first that
    /// supports it) for more or less detail.
    fn update_markdown_hover_verbosity(
        &mut self,
        widget: &Entity<HoverWidget>,
        increase: bool,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let widget = widget.clone();
        let (index, source, delta) = {
            let hover = widget.read(cx);
            let index = index.or_else(|| {
                hover.parts.iter().position(|part| {
                    part.source
                        .as_ref()
                        .is_some_and(|source| source.supports(increase))
                })
            });
            let Some(index) = index else {
                return;
            };
            let Some(source) = hover
                .parts
                .get(index)
                .and_then(|part| part.source.clone())
                .filter(|source| source.supports(increase))
            else {
                return;
            };
            let mut delta = if increase { 1 } else { -1 };
            if let Some((pending, _)) = hover.verbosity_requests.get(&index) {
                delta += pending;
            }
            (index, source, delta)
        };
        let context = self.model().read(cx).provider_context();
        let hover_context = HoverContext {
            verbosity_request: Some(HoverVerbosityRequest {
                verbosity_delta: delta,
                previous_hover: source.hover.clone(),
            }),
        };
        let request = source.provider.provide_hover_with_context(
            &context,
            source.position,
            &hover_context,
            cx,
        );
        let weak = widget.downgrade();
        let task = cx.spawn(async move |_, cx| {
            let hover = request.await;
            weak.update(cx, |widget, cx| {
                widget.verbosity_requests.remove(&index);
                if let Ok(Some(hover)) = hover
                    && let Some(part) = widget.parts.get_mut(index)
                {
                    part.contents = hover.contents.clone();
                    part.source = Some(HoverPartSource {
                        provider: source.provider,
                        hover,
                        position: source.position,
                    });
                    cx.notify();
                }
            })
            .ok();
        });
        widget.update(cx, |widget, _| {
            widget.verbosity_requests.insert(index, (delta, task));
        });
    }

    /// Monaco `InlineCompletionsHoverParticipant.computeSync`: with
    /// `show_toolbar: OnHover`, the toolbar when `position` is where ghost
    /// text is inserted; it asks explicitly for all completions.
    fn inline_toolbar_hover(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Option<Entity<crate::inline_completions::toolbar::InlineToolbar>> {
        use crate::options::InlineSuggestShowToolbar;
        if self.options.inline_suggest.show_toolbar != InlineSuggestShowToolbar::OnHover
            || !self.inline_ghost_text_at(position, cx)
        {
            return None;
        }
        self.trigger_inline_suggest(cx);
        let editor = cx.entity();
        let theme = self.theme.clone();
        Some(cx.new(|cx| {
            crate::inline_completions::toolbar::InlineToolbar::new(&editor, theme, false, cx)
        }))
    }

    fn set_hover_inline_toolbar(
        &mut self,
        toolbar: Option<Entity<crate::inline_completions::toolbar::InlineToolbar>>,
        cx: &mut Context<Self>,
    ) {
        if let Some((widget, _)) = &self.hover.widget {
            widget.update(cx, |widget, cx| {
                widget.inline_toolbar = toolbar;
                cx.notify();
            });
        }
    }

    /// Whether the visible hover has the inline suggestion toolbar.
    #[doc(hidden)]
    pub fn hover_has_inline_toolbar_for_tests(&self, cx: &gpui::App) -> bool {
        self.hover
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).inline_toolbar.is_some())
    }

    /// Whether the hover card is being resized (Monaco `isResizing`).
    /// The pointer the hover shows over the whole window during a resize.
    pub(crate) fn hover_captured_cursor(&self, cx: &gpui::App) -> Option<gpui::CursorStyle> {
        let (widget, _) = self.hover.widget.as_ref()?;
        widget.read(cx).captured_cursor()
    }

    fn is_hover_resizing(&self, cx: &gpui::App) -> bool {
        self.hover
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).resizing.is_some())
    }

    /// Whether a hover card is showing.
    pub fn is_hover_visible(&self) -> bool {
        self.hover.widget.is_some()
    }

    /// Hides the hover and cancels a pending one.
    pub fn hide_hover(&mut self, cx: &mut Context<Self>) {
        self.hover.task = None;
        self.hover.hiding = None;
        self.hover.range = None;
        self.hover.glyph = None;
        self.hover.from_keyboard = false;
        self.hover.inlay = None;
        self.hide_hover_card(cx);
    }

    /// Removes the card and its highlight, leaving what it's about and a
    /// pending hover (Monaco `ContentHoverWidgetWrapper._hideHover`).
    fn hide_hover_card(&mut self, cx: &mut Context<Self>) {
        self.hover.partial = false;
        if let Some(mut highlight) = self.hover.highlight.take() {
            highlight.clear(cx);
        }
        if let Some((_, id)) = self.hover.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Monaco `ContentHoverController.hideContentHover`: hides the hover
    /// for mouse moves, clicks, scrolls and keys, unless the inline
    /// suggestion toolbar's "⋯" menu shows (`dropDownVisible`).
    pub(crate) fn hide_content_hover(&mut self, cx: &mut Context<Self>) {
        if self.inline_toolbar_menu_visible() {
            return;
        }
        self.hide_hover(cx);
    }

    /// Whether a hover shows or is on its way.
    fn hover_active(&self) -> bool {
        self.hover.widget.is_some() || self.hover.task.is_some()
    }

    /// Whether the mouse at `position` keeps the hover (Monaco
    /// `_shouldKeepHoverWidgetVisible`): it's on the card, or the card is
    /// being resized or its color picker dragged (Monaco
    /// `isMouseOnColorPickerOrChoosingColor`).
    fn mouse_keeps_hover(&self, position: Option<gpui::Point<Pixels>>, cx: &gpui::App) -> bool {
        self.is_hover_resizing(cx)
            || self
                .hover_color_picker(cx)
                .is_some_and(|picker| picker.read(cx).captured_cursor().is_some())
            || position.is_some_and(|position| {
                self.hover_card_bounds(cx)
                    .is_some_and(|bounds| bounds.contains(&position))
            })
    }

    /// Monaco `ContentHoverController._onEditorMouseDown`, and
    /// `_onEditorMouseMove` while the button is down (a drag-select, during
    /// which no hover shows): the mouse at `position` off the card hides the
    /// hover.
    pub(crate) fn hover_mouse_down(
        &mut self,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.ignores_hover_mouse() {
            return;
        }
        if self.hover_active() && !self.mouse_keeps_hover(Some(position), cx) {
            self.hide_content_hover(cx);
        }
    }

    /// Monaco `ContentHoverController._ignoreMouseEvents`: while a context
    /// menu shows (from `onDidShowContextMenu`, which hid the hover, to
    /// `onDidHideContextMenu`), the mouse and scrolling don't touch the
    /// hover.
    fn ignores_hover_mouse(&self) -> bool {
        self.is_context_menu_visible()
    }

    /// Monaco `_onEditorMouseLeave`: the mouse left the editor at
    /// `position`. The hiding delay's grace period ends; the hover, and a
    /// pending one, go unless the mouse went onto the card.
    pub(crate) fn hover_mouse_leave(
        &mut self,
        position: Option<gpui::Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        if self.ignores_hover_mouse() {
            return;
        }
        self.hover.hiding = None;
        // A drag (the card's scrollbar, say) has the pointer captured, which
        // Monaco's editor doesn't see leave.
        if self.hover_active() && !cx.has_active_drag() && !self.mouse_keeps_hover(position, cx) {
            self.hide_content_hover(cx);
        }
    }

    /// Monaco `_onEditorScrollChanged`: scrolling hides the hover. Called
    /// when the scroll position changed, maybe while painting, so it hides
    /// after.
    pub(crate) fn hover_scrolled(&mut self, cx: &mut Context<Self>) {
        if self.hover_active() && !self.ignores_hover_mouse() {
            let editor = cx.weak_entity();
            cx.defer(move |cx| {
                editor
                    .update(cx, |this, cx| this.hide_content_hover(cx))
                    .ok();
            });
        }
    }

    /// Monaco `ContentHoverController._onKeyDown`, before the key's command
    /// runs (which it doesn't stop): any key but a modifier, the first of a
    /// chord, or a hover command while the hover shows (Show or Focus
    /// Hover, Increase or Decrease Hover Verbosity) hides the hover.
    pub(crate) fn hover_key_down(
        &mut self,
        event: &gpui::KeystrokeEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !self.hover_active() {
            return;
        }
        // Monaco `isModifierKey`: GPUI reports a lone modifier press as a
        // keystroke named after it.
        if matches!(
            event.keystroke.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        ) {
            return;
        }
        // Monaco `_isPotentialKeyboardShortcut`, from the keybindings the key
        // resolves to.
        let mut input = window
            .pending_input_keystrokes()
            .map(<[gpui::Keystroke]>::to_vec)
            .unwrap_or_default();
        input.push(event.keystroke.clone());
        let (bindings, more_chords_needed) = cx
            .key_bindings()
            .borrow()
            .bindings_for_input(&input, &event.context_stack);
        if more_chords_needed {
            return;
        }
        let is_hover_action = |action: &dyn gpui::Action| {
            action.as_any().is::<crate::actions::ShowHover>()
                || action
                    .as_any()
                    .is::<crate::actions::IncreaseHoverVerbosityLevel>()
                || action
                    .as_any()
                    .is::<crate::actions::DecreaseHoverVerbosityLevel>()
        };
        if self.is_hover_visible()
            && bindings
                .first()
                .is_some_and(|binding| is_hover_action(binding.action()))
        {
            return;
        }
        self.hide_content_hover(cx);
    }

    /// Monaco `editor.action.showHover` ("Show or Focus Hover", Cmd/Ctrl+K
    /// Cmd/Ctrl+I): focuses a visible hover, or shows one at the cursor.
    pub fn show_or_focus_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_hover(window, cx) {
            return;
        }
        self.show_hover_at_cursor(cx);
    }

    /// Focuses the visible hover and has screen readers read it; returns
    /// whether one shows. A lone row takes the focus itself (Monaco
    /// `ContentHoverWidgetWrapper.focus`).
    pub(crate) fn focus_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((widget, _)) = self.hover.widget.clone() else {
            return false;
        };
        let content = widget.update(cx, |widget, cx| {
            let lone = (widget.rendered_parts().len() == 1).then_some(0);
            widget.focus_part(lone, window, cx)
        });
        // Monaco: the focused hover's contents are its accessible name, so
        // screen readers read them on focus (`ContentHoverWidget.show`).
        if let Some(content) = content.filter(|c| !c.is_empty()) {
            self.announce(content, cx);
        }
        true
    }

    /// Focuses row `index` of the visible hover, its parts then its status
    /// bar (Monaco `ContentHoverController.focusHoverPartWithIndex`), and
    /// has screen readers read it; returns whether there is one.
    pub fn focus_hover_part(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((widget, _)) = self.hover.widget.clone() else {
            return false;
        };
        let Some(content) =
            widget.update(cx, |widget, cx| widget.focus_part(Some(index), window, cx))
        else {
            return false;
        };
        if !content.is_empty() {
            self.announce(content, cx);
        }
        true
    }

    /// The visible hover's focused row, if one has the focus (Monaco
    /// `focusedHoverPartIndex`).
    pub fn focused_hover_part_index(&self, window: &Window, cx: &gpui::App) -> Option<usize> {
        let (widget, _) = self.hover.widget.as_ref()?;
        widget.read(cx).focused_part_in(window)
    }

    /// The visible hover's rows: its parts, then its status bar (Monaco
    /// `hoverPartsCount`).
    pub fn hover_part_count(&self, cx: &gpui::App) -> usize {
        self.hover
            .widget
            .as_ref()
            .map_or(0, |(widget, _)| widget.read(cx).rendered_parts().len())
    }

    /// What screen readers read for the visible hover's row `index` (Monaco
    /// `getAccessibleWidgetContentAtIndex`).
    pub fn hover_part_accessible_content(&self, index: usize, cx: &gpui::App) -> Option<String> {
        let (widget, _) = self.hover.widget.as_ref()?;
        let widget = widget.read(cx);
        let part = *widget.rendered_parts().get(index)?;
        Some(widget.part_accessible_content(part))
    }

    /// The visible hover's accessible content (Monaco
    /// `ContentHoverController.getAccessibleWidgetContent`), as screen
    /// readers get it when it takes focus.
    pub fn hover_accessible_content(&self, cx: &gpui::App) -> Option<String> {
        let (widget, _) = self.hover.widget.as_ref()?;
        Some(widget.read(cx).accessible_content())
    }

    /// Where the hover card was last painted.
    pub(crate) fn hover_card_bounds(&self, cx: &gpui::App) -> Option<Bounds<Pixels>> {
        let (widget, _) = self.hover.widget.as_ref()?;
        widget.read(cx).bounds.get()
    }

    /// Where the hover card was last painted, for tests.
    #[doc(hidden)]
    pub fn hover_card_bounds_for_tests(&self, cx: &gpui::App) -> Option<Bounds<Pixels>> {
        self.hover_card_bounds(cx)
    }

    /// The hover card's scroll offset, for tests.
    #[doc(hidden)]
    pub fn hover_scroll_offset_for_tests(&self, cx: &gpui::App) -> Option<gpui::Point<Pixels>> {
        let (widget, _) = self.hover.widget.as_ref()?;
        Some(widget.read(cx).scroll.offset())
    }

    /// Whether the visible hover, or one of its rows, has keyboard focus
    /// (Monaco `hoverFocused`, from a focus tracker on the whole widget).
    pub fn is_hover_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.hover.widget.as_ref().is_some_and(|(widget, _)| {
            let widget = widget.read(cx);
            widget.focus_handle.is_focused(window)
                || widget
                    .part_focus
                    .iter()
                    .chain(&widget.copy_focus)
                    .chain(&widget.status_focus)
                    .any(|h| h.is_focused(window))
        })
    }

    /// Shows the hover for the cursor position right away (Monaco
    /// `editor.action.showHover`, Cmd/Ctrl+K Cmd/Ctrl+I).
    pub fn show_hover_at_cursor(&mut self, cx: &mut Context<Self>) {
        // Widget inputs are plain inputs in Monaco.
        if self.widget_input {
            return;
        }
        let position = self.cursors.primary().position;
        self.hide_hover(cx);
        self.hover.from_keyboard = true;
        self.start_content_hover(position, None, HoverSource::Keyboard, cx);
    }

    /// Monaco `ContentHoverController._onEditorMouseMove`: on mouse moves
    /// over the editor (not over other widgets) with what's under the
    /// mouse, whether it's over the hover card, and whether the card has
    /// focus.
    pub(crate) fn hover_mouse_move(
        &mut self,
        target: HoverTarget,
        over_hover: bool,
        hover_focused: bool,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if self.ignores_hover_mouse() {
            return;
        }
        self.hover.last_mouse = Some((target, modifiers));
        // Off an inlay hint's part.
        if self.hover.inlay.is_some() && !over_hover && !hover_focused {
            self.hide_hover(cx);
        }
        let options = self.options.hover;
        let visible = self.hover.widget.is_some();
        // Monaco `_shouldKeepCurrentHover`.
        let keep = visible
            && (hover_focused
                || self.is_hover_resizing(cx)
                || (options.sticky && self.hover.from_keyboard)
                || (options.sticky && over_hover)
                || (over_hover && self.hover_color_picker(cx).is_some()));
        if keep {
            self.hover.hiding = None;
            return;
        }
        // Monaco `_shouldRescheduleHoverComputation`: a grace period for
        // the mouse to reach a sticky hover.
        // Monaco's glyph hover has none (`GlyphHoverController`).
        if visible
            && options.sticky
            && !options.hiding_delay.is_zero()
            && self.hover.glyph.is_none()
            && !self.mouse_in_hover(target)
        {
            if self.hover.hiding.is_none() {
                let delay = options.hiding_delay;
                self.hover.hiding = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(delay).await;
                    this.update(cx, |this, cx| {
                        this.hover.hiding = None;
                        if let Some((target, modifiers)) = this.hover.last_mouse {
                            this.react_to_hover_mouse(target, over_hover, modifiers, cx);
                        }
                    })
                    .ok();
                }));
            }
            return;
        }
        self.hover.hiding = None;
        self.react_to_hover_mouse(target, over_hover, modifiers, cx);
    }

    /// Whether `target` is in what the hover is about, so it stays.
    fn mouse_in_hover(&self, target: HoverTarget) -> bool {
        match target {
            HoverTarget::Text(position) => self
                .hover
                .range
                .as_ref()
                .is_some_and(|range| range.start <= position && position <= range.end),
            HoverTarget::Gutter(row, lane) => self.hover.glyph == Some((row, lane)),
            HoverTarget::None => false,
        }
    }

    /// Monaco `shouldShowHover`: `hover.enabled`, with `onKeyboardModifier`
    /// the modifier opposite the multi-cursor one.
    fn hover_enabled_for(&self, modifiers: &Modifiers) -> bool {
        match self.options.hover.enabled {
            HoverEnabled::On => true,
            HoverEnabled::Off => false,
            HoverEnabled::OnKeyboardModifier => match self.options.multi_cursor_modifier {
                MultiCursorModifier::Alt => modifiers.control || modifiers.platform,
                MultiCursorModifier::CtrlCmd => modifiers.alt,
            },
        }
    }

    /// Monaco `_onKeyDown` for `onKeyboardModifier`: pressing the modifier
    /// shows the hover under the mouse.
    pub(crate) fn hover_modifiers_changed(
        &mut self,
        modifiers: &Modifiers,
        cx: &mut Context<Self>,
    ) {
        if self.options.hover.enabled != HoverEnabled::OnKeyboardModifier
            || self.hover.widget.is_some()
            || !self.hover_enabled_for(modifiers)
        {
            return;
        }
        if let Some((target, _)) = self.hover.last_mouse {
            self.react_to_hover_mouse(target, false, *modifiers, cx);
        }
    }

    /// Monaco `_reactToEditorMouseMove`: keeps the hover if the mouse is
    /// still on its range, starts a new one after `hover.delay`, or hides.
    fn react_to_hover_mouse(
        &mut self,
        target: HoverTarget,
        over_hover: bool,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if over_hover && self.hover.widget.is_some() && self.options.hover.sticky {
            return;
        }
        if !self.hover_enabled_for(&modifiers) {
            self.hide_content_hover(cx);
            return;
        }
        if self.mouse_in_hover(target) {
            return;
        }
        let delay = self.options.hover.delay;
        if let HoverTarget::Text(position) = target
            && self.adopt_visible_hover(position, cx)
        {
            self.schedule_text_hover(position, delay, cx);
            return;
        }
        self.hide_content_hover(cx);
        match target {
            HoverTarget::None => {}
            HoverTarget::Gutter(row, lane) => self.schedule_gutter_hover(row, lane, delay, cx),
            HoverTarget::Text(position) => self.schedule_text_hover(position, delay, cx),
        }
    }

    fn schedule_text_hover(
        &mut self,
        position: PointUtf16,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        self.start_content_hover(position, Some(delay), HoverSource::Mouse, cx);
    }

    /// Monaco `HoverOperation` with the `ContentHoverComputer`, started
    /// after `delay` for the mouse (`HoverStartMode.Delayed`) or at once
    /// (`None`, `Immediate`): the providers and the color picker are asked
    /// at half the delay (`computeAsync`), the markers, decoration messages
    /// and other parts are computed at the delay (`computeSync`) and shown,
    /// with each provider's part as it arrives. A delayed hover still
    /// waiting at three times the delay shows "Loading..." until done.
    fn start_content_hover(
        &mut self,
        position: PointUtf16,
        delay: Option<Duration>,
        source: HoverSource,
        cx: &mut Context<Self>,
    ) {
        let config = self.cursor_config(cx);
        let word =
            cursor::word_at(&config, self.snapshot.rope(), position).unwrap_or(position..position);
        self.hover.range = Some(word.clone());
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let mut events: FuturesUnordered<LocalBoxFuture<'static, HoverEvent>> =
                FuturesUnordered::new();
            if let Some(delay) = delay {
                // Monaco `_firstWaitTime`, then `_secondWaitTime` and
                // `_loadingMessageTime` from the start.
                let first = delay / 2;
                executor.timer(first).await;
                events.push(
                    executor
                        .timer(delay - first)
                        .map(|_| HoverEvent::Sync)
                        .boxed_local(),
                );
                events.push(
                    executor
                        .timer(delay * 3 - first)
                        .map(|_| HoverEvent::Loading)
                        .boxed_local(),
                );
            }
            let Ok(mut operation) = this.update(cx, |this, cx| {
                let tasks = this.hover_provider_requests(position, cx);
                let picker = this.color_picker_request(position, source, cx);
                let mut operation = ContentHoverOperation {
                    position,
                    word: word.clone(),
                    pending: tasks.len() + usize::from(picker.is_some()),
                    ..Default::default()
                };
                for (ordinal, (provider, task)) in tasks.into_iter().enumerate() {
                    events.push(
                        async move {
                            HoverEvent::Provider(ordinal, provider, task.await.ok().flatten())
                        }
                        .boxed_local(),
                    );
                }
                if let Some(picker) = picker {
                    events.push(picker.map(HoverEvent::ColorPicker).boxed_local());
                }
                if delay.is_none() {
                    this.compute_sync_hover(&mut operation, cx);
                    this.show_content_hover_result(&operation, cx);
                }
                operation
            }) else {
                return;
            };
            while !operation.is_complete() {
                let Some(event) = events.next().await else {
                    return;
                };
                let updated = this.update(cx, |this, cx| {
                    match event {
                        HoverEvent::Sync => this.compute_sync_hover(&mut operation, cx),
                        // Monaco `_triggerLoadingMessage`: only while waiting
                        // for the providers after the hover time.
                        HoverEvent::Loading => {
                            if !operation.synced {
                                return;
                            }
                            operation.loading = true;
                        }
                        HoverEvent::Provider(ordinal, provider, hover) => {
                            operation.pending -= 1;
                            if let Some(hover) = hover.filter(is_valid_hover) {
                                operation.add_provider_hover(ordinal, provider, hover);
                            }
                        }
                        HoverEvent::ColorPicker(picker) => {
                            operation.pending -= 1;
                            operation.color_picker = picker;
                        }
                    }
                    // Monaco `_fireResult`: nothing before the hover time.
                    if operation.synced {
                        this.show_content_hover_result(&operation, cx);
                    }
                });
                if updated.is_err() {
                    return;
                }
            }
        }));
    }

    /// Monaco `ContentHoverComputer.computeSync` at the hover time: the
    /// markers, long line warnings and decoration hover messages, unicode
    /// parts and the inline suggestion toolbar at the operation's position.
    fn compute_sync_hover(
        &mut self,
        operation: &mut ContentHoverOperation,
        cx: &mut Context<Self>,
    ) {
        let position = operation.position;
        let (markers, parts) = self.hover_sync_parts(position, cx);
        operation.markers = markers;
        operation.sync_parts = parts;
        operation.unicode = self.unicode_hovers_at(position, cx);
        operation.toolbar = self.inline_toolbar_hover(position, cx);
        operation.synced = true;
    }

    /// Monaco `ContentHoverWidgetWrapper._withResult` and
    /// `_setCurrentResult`: shows the operation's parts so far, unless a
    /// complete hover shows and this result isn't complete yet (an adopted
    /// hover stays until the new one is done). An empty result hides the
    /// card; the range stays, so moving inside it doesn't ask again.
    fn show_content_hover_result(
        &mut self,
        operation: &ContentHoverOperation,
        cx: &mut Context<Self>,
    ) {
        let complete = operation.is_complete();
        let previous_complete = self.hover.widget.is_some() && !self.hover.partial;
        if previous_complete && !complete {
            return;
        }
        let mut parts: Vec<MarkdownHoverPart> = operation
            .provider_parts
            .iter()
            .chain(&operation.sync_parts)
            .cloned()
            .collect();
        // Monaco `_addLoadingMessage`: the markdown participant's
        // `createLoadingMessage`, about the hovered character.
        if operation.loading && !complete {
            let position = operation.position;
            let rope = self.snapshot.rope();
            let line_len = rope.line_len_utf16(position.row.min(rope.max_point().row));
            let end = (position.column + 1).min(line_len).max(position.column);
            parts.push(MarkdownHoverPart {
                range: Some(position..PointUtf16::new(position.row, end)),
                ordinal: LOADING_PART_ORDINAL,
                ..MarkdownHoverPart::new(vec![LOADING_MESSAGE.into()])
            });
        }
        // Monaco `MarkdownRenderedHoverParts._renderHoverParts`.
        parts.sort_by_key(|part| part.ordinal);
        let contents = HoverContents {
            markers: operation.markers.clone(),
            parts,
            color_picker: operation.color_picker.clone(),
            unicode: operation.unicode.clone(),
        };
        if contents.is_empty() && operation.toolbar.is_none() {
            self.hide_hover_card(cx);
            return;
        }
        let range = operation
            .provider_range
            .as_ref()
            .map_or_else(|| operation.word.clone(), |(_, range)| range.clone());
        self.show_hover(operation.position, range, contents, cx);
        self.set_hover_inline_toolbar(operation.toolbar.clone(), cx);
        self.hover.partial = !complete;
    }

    /// Monaco `ContentHoverWidgetWrapper._startShowingOrUpdateHover` for the
    /// mouse at `position`, off what the visible hover is about: if it's on
    /// the hover's line (`HoverRangeAnchor.canAdoptVisibleHover`), the hover
    /// stays with the parts still valid at `position`
    /// (`ContentHoverResult.filter`, each part's `isValidForHoverAnchor`)
    /// while the hover there is computed, so moving along a long squiggle
    /// doesn't make it blink. Returns whether it stayed.
    fn adopt_visible_hover(&mut self, position: PointUtf16, cx: &mut Context<Self>) -> bool {
        // The glyph and inlay hint hovers are other anchors.
        if self.hover.glyph.is_some() || self.hover.inlay.is_some() {
            return false;
        }
        let Some((widget, _)) = self.hover.widget.clone() else {
            return false;
        };
        let row = position.row;
        let widget = widget.read(cx);
        if widget.anchor.row != row {
            return false;
        }
        let rope = self.snapshot.rope();
        if row >= rope.line_count() {
            return false;
        }
        let line_len = rope.line_len_utf16(row);
        // The anchor is the character under the mouse.
        let end = (position.column + 1).min(line_len).max(position.column);
        let valid = |range: &Range<PointUtf16>| {
            range.start.column <= position.column && range.end.column >= end
        };
        let markers: Vec<crate::Marker> = widget
            .markers
            .iter()
            .filter(|marker| valid(&range_on_row(&marker.range, row, line_len)))
            .cloned()
            .collect();
        let parts: Vec<MarkdownHoverPart> = widget
            .parts
            .iter()
            .filter(|part| part.range_on_row(row, line_len).is_some_and(|r| valid(&r)))
            .cloned()
            .collect();
        // The color picker, unicode and inline suggestion parts are about
        // the character the hover was asked for, which the mouse left.
        let unchanged = markers.len() == widget.markers.len()
            && parts.len() == widget.parts.len()
            && widget.color_picker.is_none()
            && widget.unicode.is_empty()
            && widget.inline_toolbar.is_none()
            && widget.command.is_none();
        if unchanged {
            return true;
        }
        if markers.is_empty() && parts.is_empty() {
            return false;
        }
        // The filtered hover keeps the visible one's anchor.
        let anchor = widget.anchor;
        let Some(range) = self.hover.range.clone() else {
            return false;
        };
        let contents = HoverContents {
            markers,
            parts,
            ..Default::default()
        };
        // Monaco `ContentHoverResult.filter` keeps `isComplete`.
        let partial = self.hover.partial;
        self.show_hover(anchor, range, contents, cx);
        self.hover.partial = partial;
        true
    }

    /// Monaco `GlyphHoverWidget._onModelDecorationsChanged`: a visible glyph
    /// hover is computed again for its line and lane, after the hover delay
    /// (staying meanwhile), and goes if nothing is left to show.
    pub(crate) fn hover_decorations_changed(&mut self, cx: &mut Context<Self>) {
        if self.hover.widget.is_none() {
            return;
        }
        let Some((row, target)) = self.hover.glyph else {
            return;
        };
        let delay = self.options.hover.delay;
        self.schedule_gutter_hover(row, target, delay, cx);
    }

    /// Monaco `GlyphHoverWidget._startShowingAt`: after `delay`, the hover
    /// messages for `row`'s glyph margin lane or line number, beside the
    /// gutter.
    fn schedule_gutter_hover(
        &mut self,
        row: u32,
        target: LaneOrLineNumber,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        self.hover.glyph = Some((row, target));
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                let messages = this.glyph_hover_messages(row, target, cx);
                // Monaco `GlyphHoverWidget._withResult`: nothing to show
                // hides a visible card.
                if messages.is_empty() {
                    if this.hover.widget.is_some() {
                        this.hide_hover(cx);
                    }
                    return;
                }
                let start = PointUtf16::new(row, 0);
                let contents = HoverContents {
                    parts: messages,
                    ..Default::default()
                };
                let beside = BesideGutter {
                    row,
                    after_line_numbers: target == LaneOrLineNumber::LineNumber,
                };
                this.show_hover_card(start, start..start, contents, Some(beside), cx);
                this.hover.range = None;
                this.hover.glyph = Some((row, target));
            })
            .ok();
        }));
    }

    /// Monaco `GlyphHoverComputer.computeSync`: of the decorations on
    /// `row` (`getLineDecorations`, any that cover it), the
    /// `glyph_margin_hover_message`s of those in the hovered lane, or the
    /// `line_number_hover_message`s, a hover row per message.
    fn glyph_hover_messages(
        &self,
        row: u32,
        target: LaneOrLineNumber,
        cx: &mut Context<Self>,
    ) -> Vec<MarkdownHoverPart> {
        let rope = self.snapshot.rope();
        if row >= rope.line_count() {
            return Vec::new();
        }
        let start = rope.point_to_offset(text::Point::new(row, 0));
        let end = rope.point_to_offset(text::Point::new(row, rope.line_len(row)));
        let editor_id = Some(cx.entity_id());
        let validation = self.shows_validation_decorations();
        self.model().update(cx, |model, _| {
            model
                .decorations_in_range(start..end, editor_id)
                .into_iter()
                .filter(|d| validation || !d.options.validation)
                .flat_map(|d| {
                    let messages = match target {
                        LaneOrLineNumber::Lane(lane) => {
                            let own = d.options.glyph_margin.map(|g| g.position);
                            if own.unwrap_or_default() != lane {
                                return Vec::new();
                            }
                            &d.options.glyph_margin_hover_message
                        }
                        LaneOrLineNumber::LineNumber => &d.options.line_number_hover_message,
                    };
                    if is_empty_markdown(messages) {
                        return Vec::new();
                    }
                    messages.clone()
                })
                .map(|message| MarkdownHoverPart::new(vec![message]))
                .collect()
        })
    }

    /// The mouse is over an inlay hint's label `part`: after the hover
    /// delay, shows the hint's and part's tooltips, resolving the hint
    /// first (Monaco `InlayHintsHover`, which replaces the other hover
    /// parts there).
    pub(crate) fn hover_inlay_hint_part(
        &mut self,
        part: crate::inlay_hints::InlayHintPart,
        cx: &mut Context<Self>,
    ) {
        if self.hover.inlay == Some(part) || self.ignores_hover_mouse() {
            return;
        }
        self.hide_hover(cx);
        self.hover.inlay = Some(part);
        let delay = self.options.hover.delay;
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(contents) = this.update(cx, |this, cx| this.inlay_hint_hover_contents(part, cx))
            else {
                return;
            };
            let Some(crate::inlay_hints::InlayHintHover {
                position,
                contents,
                command,
            }) = contents.await
            else {
                return;
            };
            if contents.is_empty() && command.is_none() {
                return;
            }
            this.update(cx, |this, cx| {
                let parts = contents
                    .into_iter()
                    .map(|content| MarkdownHoverPart::new(vec![content]))
                    .collect();
                let contents = HoverContents {
                    parts,
                    ..Default::default()
                };
                this.show_hover(position, position..position, contents, cx);
                this.hover.inlay = Some(part);
                if let Some((widget, _)) = &this.hover.widget {
                    widget.update(cx, |widget, cx| {
                        widget.command = command;
                        cx.notify();
                    });
                }
            })
            .ok();
        }));
    }

    /// The markdown contents of the visible hover card, for tests.
    #[doc(hidden)]
    pub fn hover_contents_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        self.hover
            .widget
            .as_ref()
            .map(|(widget, _)| {
                let widget = widget.read(cx);
                widget
                    .parts
                    .iter()
                    .flat_map(|part| part.contents.iter().map(|c| c.value.to_string()))
                    .chain(
                        widget
                            .command
                            .iter()
                            .map(|(_, keys)| format!("Execute Command{keys}")),
                    )
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Monaco `computeSync` of the marker and markdown participants: the
    /// markers at `position`, then its long line warnings and decoration
    /// hover messages, ordered after the providers' parts.
    fn hover_sync_parts(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> (Vec<crate::Marker>, Vec<MarkdownHoverPart>) {
        let offset = self.snapshot.point_utf16_to_offset(position);
        // Monaco's marker hover part comes first.
        let markers = if self.shows_validation_decorations() {
            self.model().read(cx).markers_at(offset)
        } else {
            Vec::new()
        };
        // Monaco `MarkdownHoverParticipant.computeSync`: long line warnings,
        // then decoration hover messages.
        let mut parts = self.long_line_warnings(position, cx);
        let decorations = self.line_decoration_hovers(position, cx);
        parts.extend(decorations);
        for (index, part) in parts.iter_mut().enumerate() {
            part.ordinal = SYNC_PART_ORDINAL + index;
        }
        (markers, parts)
    }

    /// Monaco `MarkdownHoverParticipant.computeAsync`: asks each hover
    /// provider about `position`, in registry order.
    fn hover_provider_requests(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> ProviderHoverTasks {
        let model = self.model().read(cx);
        let context = model.provider_context();
        let providers = hover_providers(cx, self.provider_target(cx));
        providers
            .into_iter()
            .map(|provider| {
                let task = provider.provide_hover(&context, position, cx);
                (provider, task)
            })
            .collect()
    }

    /// Monaco `ContentHoverComputer._getLineDecorations` and
    /// `MarkdownHoverParticipant.computeSync`: the hover messages of the
    /// decorations on `position`'s line that the mouse is in, or anywhere
    /// on the line for a whole-line decoration; a `show_if_collapsed` one
    /// takes a character more each side. Each message's range is the
    /// decoration's on the line.
    fn line_decoration_hovers(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Vec<MarkdownHoverPart> {
        let row = position.row;
        let rope = self.snapshot.rope();
        if row >= rope.line_count() {
            return Vec::new();
        }
        let line_len = rope.line_len_utf16(row);
        let start = rope.point_to_offset(text::Point::new(row, 0));
        let end = rope.point_to_offset(text::Point::new(row, rope.line_len(row)));
        let editor_id = Some(cx.entity_id());
        let validation = self.shows_validation_decorations();
        let snapshot = &self.snapshot;
        let column = position.column;
        self.model().update(cx, |model, _| {
            model
                .decorations_in_range(start..end, editor_id)
                .into_iter()
                .filter(|d| validation || !d.options.validation)
                .filter_map(|d| {
                    if is_empty_markdown(&d.options.hover_message) {
                        return None;
                    }
                    let range = snapshot.offset_to_point_utf16(d.range.start)
                        ..snapshot.offset_to_point_utf16(d.range.end);
                    let start = if range.start.row == row {
                        range.start.column
                    } else {
                        0
                    };
                    let end = if range.end.row == row {
                        range.end.column
                    } else {
                        line_len
                    };
                    if !d.options.is_whole_line {
                        let outside = if d.options.show_if_collapsed {
                            start > column + 1 || column.saturating_sub(1) > end
                        } else {
                            start > column || column > end
                        };
                        if outside {
                            return None;
                        }
                    }
                    Some(MarkdownHoverPart {
                        range: Some(PointUtf16::new(row, start)..PointUtf16::new(row, end)),
                        ..MarkdownHoverPart::new(d.options.hover_message.clone())
                    })
                })
                .collect()
        })
    }

    /// Monaco `HoverColorPickerParticipant`: a picker for the color at
    /// `position`, if `source` may open one.
    pub(crate) fn color_picker_request(
        &mut self,
        position: PointUtf16,
        source: HoverSource,
        cx: &mut Context<Self>,
    ) -> PickerTask {
        use crate::options::ColorDecoratorsActivatedOn as On;
        let activated = self.options.color_decorators_activated_on;
        let valid = match source {
            HoverSource::Mouse => activated != On::Click,
            HoverSource::Click => activated != On::Hover,
            HoverSource::Keyboard => true,
        };
        if !valid {
            return None;
        }
        let data = self.color_data_at(position, cx)?;
        Some(self.create_color_picker(data, None, cx))
    }

    /// Shows the color picker hover for a click on the swatch at
    /// `position` (Monaco `HoverColorPickerContribution`).
    pub(crate) fn show_color_picker_hover(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        let Some(picker) = self.color_picker_request(position, HoverSource::Click, cx) else {
            return;
        };
        self.hide_hover(cx);
        self.hover.range = Some(position..position);
        self.hover.task = Some(cx.spawn(async move |this, cx| {
            let Some(picker) = picker.await else {
                return;
            };
            this.update(cx, |this, cx| {
                let contents = HoverContents {
                    color_picker: Some(picker),
                    ..Default::default()
                };
                this.show_hover(position, position..position, contents, cx);
            })
            .ok();
        }));
    }

    /// The color picker in the hover, if any.
    pub(crate) fn hover_color_picker(
        &self,
        cx: &gpui::App,
    ) -> Option<Entity<crate::ColorPickerWidget>> {
        let (widget, _) = self.hover.widget.as_ref()?;
        widget.read(cx).color_picker.clone()
    }

    /// Shows `contents` in a new card, replacing the visible one: anchored
    /// at `anchor` (Monaco `showAtPosition`, where the hover was asked for)
    /// and about `range` (the word or the provider's range, which the mouse
    /// may move in without asking again).
    fn show_hover(
        &mut self,
        anchor: PointUtf16,
        range: Range<PointUtf16>,
        contents: HoverContents,
        cx: &mut Context<Self>,
    ) {
        self.show_hover_card(anchor, range, contents, None, cx);
    }

    /// [`Self::show_hover`], or with `beside`, Monaco's glyph hover
    /// (`GlyphHoverWidget`): an overlay beside the gutter, centered on its
    /// row, not resizable and without a highlight.
    fn show_hover_card(
        &mut self,
        anchor: PointUtf16,
        range: Range<PointUtf16>,
        contents: HoverContents,
        beside: Option<BesideGutter>,
        cx: &mut Context<Self>,
    ) {
        let HoverContents {
            markers,
            parts,
            color_picker,
            unicode,
        } = contents;
        if let Some((_, id)) = self.hover.widget.take() {
            self.remove_widget(id, cx);
        }
        // Monaco `RenderedContentHover.computeHoverPositions`: the color
        // picker shows at the color (`forceShowAtRange`); otherwise the card
        // is anchored where the hover was asked for, touching the start of
        // the parts on that line (`showAtSecondaryPosition`).
        let (range, anchor, secondary) = match &color_picker {
            Some(picker) => {
                let range = picker.read(cx).range().clone();
                (range.clone(), range.start, range.start)
            }
            None => {
                let secondary = self.hover_secondary_position(anchor, &markers, &parts);
                (range, anchor, secondary)
            }
        };
        let highlight = self
            .hover_highlight_range(anchor.row, &markers, &parts, &color_picker, cx)
            .filter(|_| beside.is_none());
        // Monaco `.monaco-editor .monaco-hover hr`: the hover border at half
        // alpha (`hoverContribution.ts`).
        let style = MarkdownStyle {
            separator: self.theme.colors.hover_border.opacity(0.5),
            ..self.markdown_style(cx)
        };
        let theme = self.theme.clone();
        let status_marker = most_severe(&markers).cloned();
        // Monaco forgets whether another marker had quick fixes.
        if self
            .hover
            .recent_marker_code_actions
            .as_ref()
            .is_some_and(|(marker, _)| Some(marker) != status_marker.as_ref())
        {
            self.hover.recent_marker_code_actions = None;
        }
        let known_unavailable = matches!(&self.hover.recent_marker_code_actions, Some((_, false)));
        // Monaco: no quick fixes in a read-only editor.
        let quick_fix = match &status_marker {
            Some(_) if !self.options.read_only => {
                if known_unavailable {
                    QuickFixStatus::Unavailable
                } else {
                    QuickFixStatus::Pending
                }
            }
            _ => QuickFixStatus::Hidden,
        };
        let editor = cx.entity().downgrade();
        let line_height = self.line_height();
        // Monaco `ContentHoverWidget._updateMaxDimensions`.
        let editor_size = self.layout_size().unwrap_or_default();
        let last = self.hover.last_size.get().unwrap_or_default();
        let mut max_size = size(
            (editor_size.width * 0.66).max(px(750.)).max(last.width),
            (editor_size.height / 4.).max(px(250.)).max(last.height),
        );
        if beside.is_some() {
            max_size.width = px(GLYPH_HOVER_MAX_WIDTH);
        }
        let uri = self.model().read(cx).uri().cloned();
        let request_quick_fixes = !matches!(quick_fix, QuickFixStatus::Hidden);
        let widget = cx.new(|cx| HoverWidget {
            markers,
            quick_fix,
            quick_fix_tasks: Vec::new(),
            uri,
            editor,
            parts,
            focused_part: None,
            part_focus: Vec::new(),
            copy_focus: Vec::new(),
            status_focus: Vec::new(),
            verbosity_requests: HashMap::new(),
            anchor,
            glyph: beside.is_some(),
            size: None,
            max_size,
            resizing: None,
            last_size: self.hover.last_size.clone(),
            color_picker,
            unicode,
            command: None,
            inline_toolbar: None,
            style,
            theme,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            line_height,
            bounds: Default::default(),
            room: Default::default(),
            status_keys: StatusKeys::default(),
            status_links: Default::default(),
        });
        if let Some(marker) = status_marker.filter(|_| request_quick_fixes) {
            self.request_hover_quick_fixes(&widget, marker, !known_unavailable, cx);
        }
        let id = match beside {
            // Monaco's glyph hover is an overlay widget that places itself
            // (`allowEditorOverflow`).
            Some(beside) => {
                let id = self.add_overlay_widget(
                    crate::widgets::OverlayWidget {
                        allow_editor_overflow: true,
                        ..crate::widgets::OverlayWidget::new(
                            crate::widgets::OverlayPosition::Custom(gpui::Point::default()),
                            widget.clone().into(),
                        )
                    },
                    cx,
                );
                if let Some(overlay) = self.widgets.overlays.iter_mut().find(|w| w.id == id) {
                    overlay.beside_gutter = Some(beside);
                }
                id
            }
            None => {
                // Monaco `hover.above`: the preferred side first.
                let preference = if self.options.hover.above {
                    vec![
                        ContentWidgetPreference::Above,
                        ContentWidgetPreference::Below,
                    ]
                } else {
                    vec![
                        ContentWidgetPreference::Below,
                        ContentWidgetPreference::Above,
                    ]
                };
                self.add_content_widget(
                    crate::widgets::ContentWidget {
                        secondary_position: Some(secondary),
                        allow_editor_overflow: true,
                        ..crate::widgets::ContentWidget::new(
                            anchor,
                            preference,
                            widget.clone().into(),
                        )
                    },
                    cx,
                )
            }
        };
        self.widgets.set_layer(id, crate::widgets::layer::HOVER);
        let room = self.widgets.room(id);
        widget.update(cx, |widget, _| widget.room = room);
        if let Some(mut old) = self.hover.highlight.take() {
            old.clear(cx);
        }
        // Monaco's `hoverHighlight` (`editor.hoverHighlightBackground`).
        self.hover.highlight = highlight.map(|highlight| {
            let options = crate::DecorationOptions {
                background: Some(self.theme.colors.hover_highlight_background),
                ..Default::default()
            };
            self.create_decorations_collection(vec![(highlight, options)], cx)
        });
        self.hover.range = Some(range);
        self.hover.widget = Some((widget, id));
        self.hover.partial = false;
    }

    /// Monaco `computeHoverPositions`' secondary position: the start of the
    /// parts' ranges that lie on `anchor`'s line, if before `anchor`, but
    /// not before the start of its view line (a wrapped line's segment).
    fn hover_secondary_position(
        &self,
        anchor: PointUtf16,
        markers: &[crate::Marker],
        parts: &[MarkdownHoverPart],
    ) -> PointUtf16 {
        let row = anchor.row;
        let rope = self.snapshot.rope();
        if row >= rope.line_count() {
            return anchor;
        }
        let line_len = rope.line_len_utf16(row);
        let (view_line, _) = self.view_lines.to_view(anchor, false);
        let boundary = self.view_lines.to_model(view_line, 0);
        let boundary = if boundary.row == row {
            boundary.column
        } else {
            0
        };
        let ranges = markers
            .iter()
            .map(|marker| range_on_row(&marker.range, row, line_len))
            .chain(
                parts
                    .iter()
                    .filter_map(|part| part.range_on_row(row, line_len)),
            );
        let mut column = anchor.column;
        for range in ranges {
            if range.start.row == row && range.end.row == row {
                column = column.min(range.start.column).max(boundary);
            }
        }
        PointUtf16::new(row, column)
    }

    /// Monaco `RenderedContentHoverParts._createEditorDecorations`: the
    /// union of the parts' ranges, a marker's clipped to the hovered line
    /// `row`; `None` when no part is about text or the union is empty.
    fn hover_highlight_range(
        &self,
        row: u32,
        markers: &[crate::Marker],
        parts: &[MarkdownHoverPart],
        color_picker: &Option<Entity<crate::ColorPickerWidget>>,
        cx: &gpui::App,
    ) -> Option<Range<PointUtf16>> {
        let rope = self.snapshot.rope();
        let line_len = rope.line_len_utf16(row.min(rope.max_point().row));
        // Monaco `MarkerHoverParticipant.computeSync`.
        let marker_ranges = markers
            .iter()
            .map(|marker| range_on_row(&marker.range, row, line_len));
        let picker_range = color_picker
            .as_ref()
            .map(|picker| picker.read(cx).range().clone());
        marker_ranges
            .chain(parts.iter().filter_map(|part| part.range.clone()))
            .chain(picker_range)
            .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end))
            .filter(|range| range.start < range.end)
    }

    /// Monaco `_renderMarkerStatusbar`'s quick fixes: asks for `marker`'s
    /// code actions, showing "Checking for quick fixes..." after 200ms
    /// (unless `show_checking` is off: `marker` is known to have none),
    /// then "Quick Fix..." or "No quick fixes available".
    fn request_hover_quick_fixes(
        &mut self,
        widget: &Entity<HoverWidget>,
        marker: crate::Marker,
        show_checking: bool,
        cx: &mut Context<Self>,
    ) {
        let mut tasks = Vec::new();
        if show_checking {
            let weak = widget.downgrade();
            tasks.push(cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(CHECKING_QUICK_FIXES_DELAY)
                    .await;
                weak.update(cx, |widget, cx| {
                    if matches!(widget.quick_fix, QuickFixStatus::Pending) {
                        widget.quick_fix = QuickFixStatus::Checking;
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        let request = self.marker_quick_fixes(&marker, cx);
        let weak = widget.downgrade();
        tasks.push(cx.spawn(async move |this, cx| {
            let set = request.await;
            let has_actions = set.has_valid();
            this.update(cx, |this, _| {
                this.hover.recent_marker_code_actions = Some((marker, has_actions));
            })
            .ok();
            weak.update(cx, |widget, cx| {
                widget.quick_fix = if has_actions {
                    QuickFixStatus::Available(Rc::new(set))
                } else {
                    QuickFixStatus::Unavailable
                };
                cx.notify();
            })
            .ok();
        }));
        widget.update(cx, |widget, _| widget.quick_fix_tasks = tasks);
    }

    /// The visible hover's status bar, as `label (keys)` links and plain
    /// texts.
    #[doc(hidden)]
    pub fn hover_status_bar_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        let Some((widget, _)) = &self.hover.widget else {
            return Vec::new();
        };
        widget
            .read(cx)
            .status_bar_entries()
            .into_iter()
            .map(|entry| match entry {
                StatusBarEntry::Action {
                    label,
                    keys: Some(keys),
                    ..
                } => format!("{label} ({keys})"),
                StatusBarEntry::Action { label, .. } => label.to_string(),
                StatusBarEntry::Text(text) => text.to_string(),
            })
            .collect()
    }

    /// Runs the visible hover's status bar action whose label starts with
    /// `label`; returns whether there was one.
    #[doc(hidden)]
    pub fn run_hover_status_action_for_tests(
        &mut self,
        label: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((widget, _)) = &self.hover.widget else {
            return false;
        };
        let widget = widget.read(cx);
        let run = widget
            .status_bar_entries()
            .into_iter()
            .enumerate()
            .find_map(|(ix, entry)| match entry {
                StatusBarEntry::Action { label: l, run, .. } if l.starts_with(label) => {
                    Some((ix, run))
                }
                _ => None,
            });
        // As if its link was clicked where it was last painted.
        let links = widget.status_links.clone();
        match run {
            Some((ix, run)) => {
                let target = links.borrow().get(ix).copied().flatten();
                run.run(target, self, cx);
                true
            }
            None => false,
        }
    }

    /// The range of the visible hover's `hoverHighlight` decoration.
    #[doc(hidden)]
    pub fn hover_highlight_for_tests(&self, cx: &gpui::App) -> Option<Range<PointUtf16>> {
        self.hover.highlight.as_ref()?.range(0, cx)
    }
}
