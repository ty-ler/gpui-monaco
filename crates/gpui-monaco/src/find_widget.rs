//! The find/replace widget (Monaco's `findWidget`): an overlay widget whose
//! inputs are single-line [`CodeEditor`]s that grow with their lines
//! (Monaco's `flexibleHeight` inputs: Ctrl+Enter and pasting insert line
//! breaks).
//!
//! The layout follows `findWidget.css`: the replace toggle is a full-height
//! column at the left, the case, word and regex toggles sit inside the find
//! input (`FindInput`'s `.controls`) and preserve case inside the replace
//! input, and the close button is at the top right. Narrow editors get
//! Monaco's reduced, narrow and collapsed layouts (`_tryUpdateWidgetWidth`).

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::actions::{
    CloseFindWidget, FindHistoryNext, FindHistoryPrevious, FindInputFocusEditor, FindInputNewline,
    Newline, NextMatch, Outdent, Paste, PreviousMatch, RemoveSecondaryCursors, ReplaceAll,
    ReplaceOne, Tab, ToggleCaseSensitive, ToggleFindInSelection, TogglePreserveCase, ToggleRegex,
    ToggleWholeWord,
};
use crate::codicons::{Codicon, icon as codicon};
use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::find::FindHistory;
use crate::find::FindOptions;
use crate::model::TextModelEvent;
use crate::theme::EditorColors;

/// Monaco `FIND_WIDGET_INITIAL_WIDTH`.
const FIND_WIDGET_INITIAL_WIDTH: f32 = 419.;
/// The `narrow-find-widget` and `collapsed-find-widget` maximum widths.
const NARROW_WIDTH: f32 = 257.;
const COLLAPSED_WIDTH: f32 = 170.;
/// The matches count's `min-width` (Monaco `MAX_MATCHES_COUNT_WIDTH`'s
/// start).
const MATCHES_COUNT_MIN_WIDTH: f32 = 69.;
/// Monaco `FIND_INPUT_AREA_HEIGHT`: the widget's height without replace.
pub(crate) const FIND_INPUT_AREA_HEIGHT: f32 = 33.;
/// The widget's `line-height`, which its inputs' lines take.
const INPUT_LINE_HEIGHT: f32 = 19.;
/// Monaco's `flexibleMaxHeight` for the find and replace inputs; taller
/// text scrolls.
const INPUT_MAX_HEIGHT: f32 = 118.;
/// `padding: 0 4px 0 9px`.
const PADDING_LEFT: f32 = 9.;
const PADDING_RIGHT: f32 = 4.;
/// `margin-top: 4px`, below the editor's top.
const MARGIN_TOP: f32 = 4.;
/// The slide-in starts `translateY(calc(-100% - 10px))` above: the
/// widget's height and its shadow.
const REVEAL_SHADOW: f32 = 10.;
/// The find and replace parts' `margin: 3px 25px 0 17px`.
const PART_MARGIN_TOP: f32 = 3.;
const PART_MARGIN_RIGHT: f32 = 25.;
const PART_MARGIN_LEFT: f32 = 17.;
/// A `.button`: a 16px codicon with 3px padding, and its 3px left margin.
const BUTTON_SIZE: f32 = 22.;
const BUTTON_MARGIN: f32 = 3.;
/// A `Toggle` (`.monaco-custom-toggle`): 20px with a 2px left margin; the
/// find in selection one is 22px (`.codicon-find-selection`).
const TOGGLE_SIZE: f32 = 20.;
const TOGGLE_MARGIN: f32 = 2.;
/// The input box's text padding (`.monaco-inputbox .input`), less the
/// input editor's own 2px line decorations.
const INPUT_PADDING_LEFT: f32 = 4.;
/// `--vscode-cornerRadius-large` (the widget) and `-small` (the replace
/// toggle), and the input box's and buttons' radii.
const CORNER_RADIUS_LARGE: f32 = 8.;
const CORNER_RADIUS_SMALL: f32 = 4.;
const INPUT_RADIUS: f32 = 2.;
const BUTTON_RADIUS: f32 = 5.;
const TOGGLE_RADIUS: f32 = 3.;
/// The sash's pointer (Monaco's vertical `Sash`: `ew-resize`).
const SASH_CURSOR: gpui::CursorStyle = gpui::CursorStyle::ResizeLeftRight;
/// The sash: Monaco's vertical `Sash` (`size: 2`), and a wider grab area.
const SASH_SIZE: f32 = 2.;
const SASH_HIT_SIZE: f32 = 4.;
/// The slide-in (`transition: transform 200ms linear`).
const REVEAL_DURATION: Duration = Duration::from_millis(200);
/// Monaco's delay before a typed search string joins the history.
const HISTORY_DELAY: Duration = Duration::from_millis(500);
/// `NLS_MATCHES_COUNT_LIMIT_TITLE`.
const MATCHES_LIMIT_TITLE: &str = "Only the first 19999 results are highlighted, but all find operations work on the entire text.";

/// The find widget's buttons and toggles, which take the focus (Monaco's
/// are tab stops), in their DOM order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    ToggleReplace,
    MatchCase,
    WholeWord,
    Regex,
    Previous,
    Next,
    InSelection,
    Close,
    PreserveCase,
    Replace,
    ReplaceAll,
}

const CONTROLS: [Control; 11] = [
    Control::ToggleReplace,
    Control::MatchCase,
    Control::WholeWord,
    Control::Regex,
    Control::Previous,
    Control::Next,
    Control::InSelection,
    Control::Close,
    Control::PreserveCase,
    Control::Replace,
    Control::ReplaceAll,
];

/// A place Tab moves through: an input or a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    FindInput,
    ReplaceInput,
    Control(Control),
}

/// The widget's tab stops, in Monaco's DOM order.
const STOPS: [Stop; 13] = [
    Stop::Control(Control::ToggleReplace),
    Stop::FindInput,
    Stop::Control(Control::MatchCase),
    Stop::Control(Control::WholeWord),
    Stop::Control(Control::Regex),
    Stop::Control(Control::Previous),
    Stop::Control(Control::Next),
    Stop::Control(Control::InSelection),
    Stop::Control(Control::Close),
    Stop::ReplaceInput,
    Stop::Control(Control::PreserveCase),
    Stop::Control(Control::Replace),
    Stop::Control(Control::ReplaceAll),
];

impl Control {
    fn index(self) -> usize {
        CONTROLS.iter().position(|c| *c == self).unwrap_or_default()
    }

    fn id(self) -> &'static str {
        match self {
            Control::ToggleReplace => "toggle-replace",
            Control::MatchCase => "match-case",
            Control::WholeWord => "whole-word",
            Control::Regex => "regex",
            Control::Previous => "previous",
            Control::Next => "next",
            Control::InSelection => "in-selection",
            Control::Close => "close",
            Control::PreserveCase => "preserve-case",
            Control::Replace => "replace",
            Control::ReplaceAll => "replace-all",
        }
    }

    /// Monaco's title, before the keybinding.
    fn title(self) -> &'static str {
        match self {
            Control::ToggleReplace => "Toggle Replace",
            Control::MatchCase => "Match Case",
            Control::WholeWord => "Match Whole Word",
            Control::Regex => "Use Regular Expression",
            Control::Previous => "Previous Match",
            Control::Next => "Next Match",
            Control::InSelection => "Find in Selection",
            Control::Close => "Close",
            Control::PreserveCase => "Preserve Case",
            Control::Replace => "Replace",
            Control::ReplaceAll => "Replace All",
        }
    }

    /// The command whose keybinding the title shows (Monaco
    /// `_keybindingLabelFor`).
    fn action(self) -> Option<Box<dyn gpui::Action>> {
        Some(match self {
            Control::ToggleReplace => return None,
            Control::MatchCase => Box::new(ToggleCaseSensitive),
            Control::WholeWord => Box::new(ToggleWholeWord),
            Control::Regex => Box::new(ToggleRegex),
            Control::Previous => Box::new(PreviousMatch),
            Control::Next => Box::new(NextMatch),
            Control::InSelection => Box::new(ToggleFindInSelection),
            Control::Close => Box::new(CloseFindWidget),
            Control::PreserveCase => Box::new(TogglePreserveCase),
            Control::Replace => Box::new(ReplaceOne),
            Control::ReplaceAll => Box::new(ReplaceAll),
        })
    }
}

/// Monaco's widths for the editor's size (`_tryUpdateWidgetWidth`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FindWidgetLayout {
    /// The widget's width.
    pub width: Pixels,
    /// The find and replace inputs' width.
    pub input_width: Pixels,
    /// `reduced-find-widget`: no matches count.
    pub reduced: bool,
    /// `narrow-find-widget`: at most 257px.
    pub narrow: bool,
    /// `collapsed-find-widget`: at most 170px, without the previous, next
    /// and replace buttons or the find input's toggles.
    pub collapsed: bool,
    /// The editor has no room for text (Monaco's `hiddenEditor`).
    pub hidden: bool,
}

impl FindWidgetLayout {
    /// Monaco `_tryUpdateWidgetWidth` for an editor `editor_width` wide
    /// with a `minimap_width` minimap, the widget `width` wide after a
    /// resize (`resized`), and the matches count `matches_width` wide.
    /// Unlike Monaco, the widget also never gets wider than the room left
    /// of it, so it stays inside the editor.
    fn compute(
        editor_width: Pixels,
        content_width: Pixels,
        minimap_width: Pixels,
        width: Pixels,
        resized: bool,
        matches_width: Pixels,
    ) -> Self {
        if content_width <= px(0.) {
            return Self {
                hidden: true,
                ..Default::default()
            };
        }
        let initial = px(FIND_WIDGET_INITIAL_WIDTH);
        let max_width = editor_width - px(28.) - minimap_width - px(15.);
        let reduced = initial + px(28.) + minimap_width >= editor_width;
        let narrow =
            initial + px(28.) + minimap_width - px(MATCHES_COUNT_MIN_WIDTH) >= editor_width;
        let collapsed = initial + px(28.) + minimap_width - px(MATCHES_COUNT_MIN_WIDTH)
            >= editor_width + px(50.);
        let mut layout = Self {
            reduced,
            narrow,
            collapsed,
            ..Default::default()
        };
        let mut width = if resized { width } else { initial };
        if resized && width > initial {
            // Resized by the user: only the editor's width limits it.
            layout.reduced = false;
            layout.narrow = false;
            layout.collapsed = false;
            width = width.min(max_width);
        } else if collapsed {
            width = width.min(px(COLLAPSED_WIDTH));
        } else if narrow {
            width = width.min(px(NARROW_WIDTH));
        } else {
            width = width.min(max_width);
        }
        // Never past the editor's left edge.
        width = width
            .min(editor_width - px(28.) - minimap_width)
            .max(px(0.));
        layout.width = width;
        let part =
            width - px(2. + PADDING_LEFT + PADDING_RIGHT + PART_MARGIN_LEFT + PART_MARGIN_RIGHT);
        layout.input_width = (part - layout.find_actions_width(matches_width)).max(px(0.));
        layout
    }

    /// The find part's `.find-actions` width: the matches count (unless
    /// reduced), previous and next (unless collapsed), and find in
    /// selection.
    fn find_actions_width(&self, matches_width: Pixels) -> Pixels {
        let mut width = px(TOGGLE_MARGIN + BUTTON_SIZE);
        if !self.reduced {
            // `margin: 0 0 0 3px; padding: 2px 0 0 2px`.
            width += matches_width + px(3.);
        }
        if !self.collapsed {
            width += px(2. * (BUTTON_SIZE + BUTTON_MARGIN));
        }
        width
    }
}

/// An input's box height (Monaco `inputBox.height` plus its border): a row
/// per line of its text in its own line height, up to Monaco's
/// `flexibleMaxHeight`, plus the box's padding and border.
fn input_box_height(input: &CodeEditor, cx: &gpui::App) -> Pixels {
    let lines = input.model().read(cx).buffer().max_point_utf16().row + 1;
    let line_height = input.options().resolved_line_height();
    let inner = (line_height * lines as f32 + px(4.)).min(px(INPUT_MAX_HEIGHT));
    inner.max(line_height + px(4.)) + px(2.)
}

/// The widget's height (Monaco `_getHeight`): the find input, and the
/// replace input when it shows, with 4px around each.
pub(crate) fn widget_height(
    find: &CodeEditor,
    replace: Option<&CodeEditor>,
    cx: &gpui::App,
) -> Pixels {
    let mut height = px(4.) + input_box_height(find, cx);
    if let Some(replace) = replace {
        height += px(4.) + input_box_height(replace, cx);
    }
    height + px(4.)
}

/// Where `text` inserted at `start` ends.
fn end_after(start: text::PointUtf16, text: &str) -> text::PointUtf16 {
    match text.rsplit_once('\n') {
        Some((before, last)) => text::PointUtf16::new(
            start.row + before.matches('\n').count() as u32 + 1,
            last.encode_utf16().count() as u32,
        ),
        None => text::PointUtf16::new(start.row, start.column + text.encode_utf16().count() as u32),
    }
}

/// The find widget's key context; its inputs' are inside it. Each carries
/// the editor's context keys, for its preconditions.
pub(crate) const FIND_WIDGET_KEY_CONTEXT: &str = "FindWidget";
/// The find input's key context, around its input editor.
pub(crate) const FIND_INPUT_KEY_CONTEXT: &str = "FindInput";
/// The replace input's key context, around its input editor.
pub(crate) const REPLACE_INPUT_KEY_CONTEXT: &str = "ReplaceInput";

/// The find and replace widget, shown as an overlay at the top right of the
/// editor by the find commands.
pub struct FindWidget {
    editor: WeakEntity<CodeEditor>,
    find_input: Entity<CodeEditor>,
    replace_input: Entity<CodeEditor>,
    replace_visible: bool,
    focus_handle: FocusHandle,
    /// Each [`Control`]'s focus handle.
    controls: Vec<FocusHandle>,
    /// Where Up and Down are in each input's history.
    find_history_index: Option<usize>,
    replace_history_index: Option<usize>,
    /// Set while history navigation writes an input.
    navigating_history: bool,
    /// Monaco's `_updateHistoryDelayer`.
    history_task: Option<gpui::Task<()>>,
    /// The widget's width after dragging its left edge (Monaco's sash),
    /// and whether it was (Monaco `_resized`).
    width: Pixels,
    resized: bool,
    /// The drag: where it started and the width then.
    resize: Option<(Pixels, Pixels)>,
    /// Monaco's `MAX_MATCHES_COUNT_WIDTH`: grows to fit the widest count.
    matches_width: Pixels,
    /// The height last measured and the height it was laid out for.
    measured: Rc<Cell<Option<(Pixels, Pixels)>>>,
    /// The layout of the last frame, for tests.
    last_layout: FindWidgetLayout,
    /// When the widget was shown, on the executor's clock (Monaco adds
    /// `visible`, starting the slide-in).
    revealed_at: Instant,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for FindWidget {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// A find or replace input: Monaco's 13px UI font in the widget's 19px
/// lines, with a placeholder.
fn input(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<FindWidget>,
    theme: Option<std::sync::Arc<crate::Theme>>,
) -> Entity<CodeEditor> {
    cx.new(|cx| {
        let theme = theme.unwrap_or_else(|| {
            crate::theme::global_theme(cx)
                .unwrap_or_else(|| std::sync::Arc::new(crate::Theme::vs_dark()))
        });
        let mut input = CodeEditor::new_widget_input("", theme, window, cx);
        let mut options = input.options().clone();
        options.line_height = Some(px(INPUT_LINE_HEIGHT));
        options.placeholder = Some(placeholder.into());
        input.update_options(options, cx);
        input
    })
}

impl FindWidget {
    /// `theme` is the main editor's; the widget is created while the editor
    /// is being updated, so it can't read it.
    pub fn new(
        editor: WeakEntity<CodeEditor>,
        theme: std::sync::Arc<crate::Theme>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = Some(theme);
        // Monaco `NLS_FIND_INPUT_PLACEHOLDER` and
        // `NLS_REPLACE_INPUT_PLACEHOLDER`.
        let find_input = input("Find", window, cx, theme.clone());
        let replace_input = input("Replace", window, cx, theme);
        let subscriptions = [&find_input, &replace_input]
            .into_iter()
            .map(|input| {
                let model = input.read(cx).model().clone();
                cx.subscribe(&model, |this, _, event: &TextModelEvent, cx| {
                    if matches!(event, TextModelEvent::Edited { .. }) {
                        this.push_options(cx);
                    }
                })
            })
            .collect();
        Self {
            editor,
            find_input,
            replace_input,
            replace_visible: false,
            focus_handle: cx.focus_handle(),
            controls: CONTROLS.iter().map(|_| cx.focus_handle()).collect(),
            find_history_index: None,
            replace_history_index: None,
            navigating_history: false,
            history_task: None,
            width: px(FIND_WIDGET_INITIAL_WIDTH),
            resized: false,
            resize: None,
            matches_width: px(MATCHES_COUNT_MIN_WIDTH),
            measured: Rc::new(Cell::new(None)),
            last_layout: FindWidgetLayout::default(),
            revealed_at: cx.background_executor().now(),
            _subscriptions: subscriptions,
        }
    }

    /// How far the slide-in is, from 0 to 1: on the executor's clock, so
    /// tests step it, and done at once when motion is reduced (Monaco's
    /// `.monaco-reduce-motion` 0ms transition).
    fn reveal_progress(&self, cx: &gpui::App) -> f32 {
        if cx.reduce_motion() {
            return 1.;
        }
        let elapsed = cx
            .background_executor()
            .now()
            .saturating_duration_since(self.revealed_at);
        (elapsed.as_secs_f32() / REVEAL_DURATION.as_secs_f32()).min(1.)
    }

    /// The find and replace inputs.
    pub(crate) fn inputs(&self) -> (Entity<CodeEditor>, Entity<CodeEditor>) {
        (self.find_input.clone(), self.replace_input.clone())
    }

    fn input_text(input: &Entity<CodeEditor>, cx: &gpui::App) -> String {
        input.read(cx).model().read(cx).text()
    }

    /// Sends the inputs' text to the editor's find options, and adds it to
    /// the history after a pause in typing.
    fn push_options(&mut self, cx: &mut Context<Self>) {
        let query = Self::input_text(&self.find_input, cx);
        let replace = Self::input_text(&self.replace_input, cx);
        if !self.navigating_history {
            self.find_history_index = None;
            self.replace_history_index = None;
            self.history_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(HISTORY_DELAY).await;
                this.update(cx, |this, cx| this.update_history(cx)).ok();
            }));
        }
        // Monaco `find.findOnType`: when off, Enter sends the search string.
        let find_on_type = self.find_on_type(cx);
        self.update_options(cx, |options| {
            if find_on_type {
                options.query = query;
            }
            options.replace = replace;
        });
        // The inputs grow with their lines (Monaco `onDidHeightChange`).
        self.with_editor(cx, |editor, cx| editor.update_find_view_zone(cx));
    }

    fn find_on_type(&self, cx: &gpui::App) -> bool {
        self.editor
            .upgrade()
            .is_none_or(|editor| editor.read(cx).options().find.find_on_type)
    }

    fn update_options(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut FindOptions)) {
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| {
                let mut options = editor.find_options().clone();
                f(&mut options);
                if options != *editor.find_options() {
                    editor.set_find_options(options, cx);
                }
            });
        }
        cx.notify();
    }

    /// Monaco `_updateHistory`: adds the inputs' text to the history.
    fn update_history(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let options = editor.read(cx).options().find.clone();
        let query = Self::input_text(&self.find_input, cx);
        let replace = Self::input_text(&self.replace_input, cx);
        let history = cx.default_global::<FindHistory>();
        if options.history {
            FindHistory::add(&mut history.find, &query);
        }
        if options.replace_history && !replace.is_empty() {
            FindHistory::add(&mut history.replace, &replace);
        }
    }

    /// Monaco `HistoryInputBox.showPreviousValue`/`showNextValue` for the
    /// focused input.
    fn navigate_history(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let options = editor.read(cx).options().find.clone();
        let replace = self.replace_input.focus_handle(cx).is_focused(window);
        if (replace && !options.replace_history) || (!replace && !options.history) {
            return;
        }
        let input = if replace {
            self.replace_input.clone()
        } else {
            self.find_input.clone()
        };
        let value = Self::input_text(&input, cx);
        let history = cx.default_global::<FindHistory>();
        let list = if replace {
            &mut history.replace
        } else {
            &mut history.find
        };
        // The current text joins the history first.
        if !value.is_empty() && !list.contains(&value) {
            list.push(value.clone());
        }
        let list = list.clone();
        let index = if replace {
            &mut self.replace_history_index
        } else {
            &mut self.find_history_index
        };
        let current = index.or_else(|| list.iter().position(|v| *v == value));
        let next = if back {
            match current {
                Some(0) => return,
                Some(ix) => Some(ix - 1),
                None => list.len().checked_sub(1),
            }
        } else {
            match current {
                Some(ix) if ix + 1 < list.len() => Some(ix + 1),
                // Past the newest: an empty input.
                _ => None,
            }
        };
        if next.is_none() && back {
            return;
        }
        *index = next;
        let text = next.map_or(String::new(), |ix| list[ix].clone());
        self.navigating_history = true;
        input.update(cx, |input, cx| {
            input
                .model()
                .update(cx, |model, cx| model.set_value(&text, cx));
            let end = input.model().read(cx).buffer().max_point_utf16();
            input.set_selections(&[Selection::collapsed(end)], cx);
        });
        self.navigating_history = false;
        cx.notify();
    }

    /// Shows `options` in the inputs (when opened with a seeded query).
    pub fn sync_from_editor(
        &mut self,
        options: &FindOptions,
        replace_visible: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_inputs(options, replace_visible, cx);
    }

    /// Whether the find input, the replace input and the widget have the
    /// focus.
    pub(crate) fn focus_state(&self, window: &Window, cx: &gpui::App) -> (bool, bool, bool) {
        (
            self.find_input.focus_handle(cx).is_focused(window),
            self.replace_input.focus_handle(cx).is_focused(window),
            self.focus_handle.contains_focused(window, cx),
        )
    }

    /// Selects the find input's text without focusing it (Monaco
    /// `_findInput.select()`).
    pub(crate) fn select_find_input(&mut self, cx: &mut Context<Self>) {
        self.find_input.update(cx, |input, cx| {
            let end = input.model().read(cx).buffer().max_point_utf16();
            input.set_selections(
                &[Selection {
                    anchor: text::PointUtf16::zero(),
                    head: end,
                }],
                cx,
            );
        });
    }

    /// Shows `options` and the replace input's visibility.
    pub(crate) fn set_inputs(
        &mut self,
        options: &FindOptions,
        replace_visible: bool,
        cx: &mut Context<Self>,
    ) {
        self.replace_visible = replace_visible;
        for (input, value) in [
            (&self.find_input, &options.query),
            (&self.replace_input, &options.replace),
        ] {
            let model = input.read(cx).model().clone();
            if model.read(cx).text() != *value {
                model.update(cx, |model, cx| model.set_value(value, cx));
            }
        }
        cx.notify();
    }

    /// Focuses the find input and selects its text.
    pub fn focus_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        Self::focus_input(&self.find_input, window, cx);
    }

    /// Focuses the replace input and selects its text.
    pub fn focus_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        Self::focus_input(&self.replace_input, window, cx);
    }

    /// Whether the find input has focus.
    pub fn is_find_input_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.find_input.focus_handle(cx).is_focused(window)
    }

    fn focus_input(input: &Entity<CodeEditor>, window: &mut Window, cx: &mut Context<Self>) {
        input.update(cx, |input, cx| {
            let end = input.model().read(cx).buffer().max_point_utf16();
            input.set_selections(
                &[Selection {
                    anchor: text::PointUtf16::zero(),
                    head: end,
                }],
                cx,
            );
            window.focus(&input.focus_handle(cx), cx);
        });
    }

    fn with_editor(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut CodeEditor, &mut Context<CodeEditor>),
    ) {
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, f);
        }
    }

    fn on_history_previous(
        &mut self,
        _: &FindHistoryPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.move_in_multiline_input(true, window, cx) {
            self.navigate_history(true, window, cx);
        }
    }

    fn on_history_next(
        &mut self,
        _: &FindHistoryNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.move_in_multiline_input(false, window, cx) {
            self.navigate_history(false, window, cx);
        }
    }

    /// The focused input, if one is.
    fn focused_input(&self, window: &Window, cx: &gpui::App) -> Option<Entity<CodeEditor>> {
        [&self.find_input, &self.replace_input]
            .into_iter()
            .find(|input| input.focus_handle(cx).is_focused(window))
            .cloned()
    }

    /// Monaco `stopPropagationForMultiLineUpwards`/`Downwards`: in an input
    /// with several lines, Up (unless at its very start) and Down (unless
    /// at its very end) move the cursor, as in a textarea, instead of
    /// walking the history. Returns whether it moved.
    fn move_in_multiline_input(
        &mut self,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(input) = self.focused_input(window, cx) else {
            return false;
        };
        input.update(cx, |input, cx| {
            let rope = input.model().read(cx).buffer().rope().clone();
            let max = rope.max_point_utf16();
            if max.row == 0 {
                return false;
            }
            let selection = input.selections()[0];
            let target = if up {
                let start = selection.start();
                if start == text::PointUtf16::zero() {
                    return false;
                }
                if start.row == 0 {
                    text::PointUtf16::zero()
                } else {
                    let column = start.column.min(rope.line_len_utf16(start.row - 1));
                    text::PointUtf16::new(start.row - 1, column)
                }
            } else {
                let end = selection.end();
                if end == max {
                    return false;
                }
                if end.row == max.row {
                    max
                } else {
                    let column = end.column.min(rope.line_len_utf16(end.row + 1));
                    text::PointUtf16::new(end.row + 1, column)
                }
            };
            input.set_selections(&[Selection::collapsed(target)], cx);
            true
        })
    }

    /// Monaco `InputBox.insertAtCursor`: replaces the input's selection
    /// with `text`, the cursor after it.
    fn insert_in_input(input: &Entity<CodeEditor>, text: &str, cx: &mut Context<Self>) {
        input.update(cx, |input, cx| {
            let selection = input.selections()[0];
            let start = selection.start();
            let end = end_after(start, text);
            input.execute_edits(
                vec![(start..selection.end(), text.to_string())],
                Some(vec![Selection::collapsed(end)]),
                cx,
            );
        });
    }

    /// Ctrl+Enter in an input inserts a line break (Monaco
    /// `_onFindInputKeyDown`/`_onReplaceInputKeyDown`).
    fn on_input_newline(
        &mut self,
        _: &FindInputNewline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = self.focused_input(window, cx) {
            Self::insert_in_input(&input, "\n", cx);
        }
    }

    /// Cmd/Ctrl+Down in an input focuses the editor, keeping the widget
    /// (Monaco `_onFindInputKeyDown`/`_onReplaceInputKeyDown`).
    fn on_focus_editor(
        &mut self,
        _: &FindInputFocusEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.editor.upgrade() {
            window.focus(&editor.focus_handle(cx), cx);
        }
    }

    /// Pasting text with line breaks keeps them, as Monaco's textarea
    /// inputs do (single-line inputs turn them into spaces).
    fn on_paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.focused_input(window, cx) else {
            return;
        };
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if !text.contains(['\n', '\r']) {
            return;
        }
        cx.stop_propagation();
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        Self::insert_in_input(&input, &text, cx);
    }

    /// Monaco's multicursor commands have `editorFocus`, which the find
    /// widget's inputs have: Cmd/Ctrl+D, Cmd/Ctrl+Shift+L and Cmd/Ctrl+K
    /// Cmd/Ctrl+D act on the editor, searching for the widget's string.
    fn capture_multi_cursor(
        &mut self,
        action: &dyn gpui::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focused_input(window, cx).is_none() {
            return;
        }
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        if editor.update(cx, |editor, cx| {
            editor.multi_cursor_from_find_widget(action, cx)
        }) {
            cx.stop_propagation();
        }
    }

    fn on_enter(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(control) = self.focused_control(window) {
            self.activate(control, window, cx);
            return;
        }
        if self.replace_input.focus_handle(cx).is_focused(window) {
            self.with_editor(cx, |editor, cx| editor.replace_one(cx));
        } else {
            if !self.find_on_type(cx) {
                let query = Self::input_text(&self.find_input, cx);
                self.update_options(cx, |options| options.query = query);
            }
            self.with_editor(cx, |editor, cx| editor.find_next(cx));
        }
    }

    /// Space (and Enter) on a focused button or toggle presses it.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() || !matches!(keystroke.key.as_str(), "space" | "enter") {
            return;
        }
        if let Some(control) = self.focused_control(window) {
            cx.stop_propagation();
            self.activate(control, window, cx);
        }
    }

    fn on_escape(
        &mut self,
        _: &RemoveSecondaryCursors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| editor.close_find(window, cx));
        }
    }

    fn focused_control(&self, window: &Window) -> Option<Control> {
        CONTROLS
            .iter()
            .copied()
            .find(|control| self.controls[control.index()].is_focused(window))
    }

    fn focused_stop(&self, window: &Window, cx: &gpui::App) -> Option<Stop> {
        if self.find_input.focus_handle(cx).is_focused(window) {
            Some(Stop::FindInput)
        } else if self.replace_input.focus_handle(cx).is_focused(window) {
            Some(Stop::ReplaceInput)
        } else {
            self.focused_control(window).map(Stop::Control)
        }
    }

    /// The editor's state the controls depend on.
    fn control_state(&self, cx: &gpui::App) -> Option<ControlState> {
        let editor = self.editor.upgrade()?;
        let editor = editor.read(cx);
        let options = editor.find_options();
        let find = editor.find_state();
        let loop_ = editor.options().find.loop_;
        let has_query = !options.query.is_empty();
        let has_matches = find.matches_count > 0;
        let read_only = editor.options().read_only;
        Some(ControlState {
            read_only,
            can_previous: has_query && has_matches && find.can_navigate_back(loop_),
            can_next: has_query && has_matches && find.can_navigate_forward(loop_),
            can_replace: self.replace_visible && has_query,
            can_find_in_selection: options.in_selection
                || editor.selections().iter().any(|s| !s.is_empty()),
        })
    }

    /// Whether `stop` takes the focus now: shown and enabled (Monaco's
    /// disabled buttons leave the tab order).
    fn is_tab_stop(&self, stop: Stop, cx: &gpui::App) -> bool {
        let Some(state) = self.control_state(cx) else {
            return false;
        };
        let layout = self.last_layout;
        match stop {
            Stop::FindInput => true,
            Stop::ReplaceInput => self.replace_visible,
            Stop::Control(control) => match control {
                Control::ToggleReplace => !state.read_only,
                Control::MatchCase | Control::WholeWord | Control::Regex => !layout.collapsed,
                Control::Previous => !layout.collapsed && state.can_previous,
                Control::Next => !layout.collapsed && state.can_next,
                Control::InSelection => state.can_find_in_selection,
                Control::Close => true,
                Control::PreserveCase => self.replace_visible,
                Control::Replace | Control::ReplaceAll => {
                    self.replace_visible && !layout.collapsed && state.can_replace
                }
            },
        }
    }

    fn focus_stop(&mut self, stop: Stop, window: &mut Window, cx: &mut Context<Self>) {
        match stop {
            Stop::FindInput => Self::focus_input(&self.find_input, window, cx),
            Stop::ReplaceInput => Self::focus_input(&self.replace_input, window, cx),
            Stop::Control(control) => window.focus(&self.controls[control.index()], cx),
        }
        cx.notify();
    }

    /// Leaves the widget for the editor's text.
    fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            window.focus(&editor.focus_handle(cx), cx);
        }
    }

    /// Tab and Shift+Tab: Monaco's widget order, with its keydown
    /// handlers' jumps (find input → replace input or match case, replace
    /// input → match case, regex → preserve case, preserve case → the
    /// navigation buttons, close → replace, …). Past either end, the
    /// focus goes to the editor.
    fn move_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.focused_stop(window, cx) else {
            self.focus_stop(Stop::FindInput, window, cx);
            return;
        };
        let replace = self.replace_visible;
        let jump = match (current, forward) {
            (Stop::FindInput, true) if replace => Some(Stop::ReplaceInput),
            (Stop::FindInput, true) => Some(Stop::Control(Control::MatchCase)),
            (Stop::ReplaceInput, true) => Some(Stop::Control(Control::MatchCase)),
            (Stop::ReplaceInput, false) => Some(Stop::FindInput),
            (Stop::Control(Control::MatchCase), false) if replace => Some(Stop::ReplaceInput),
            (Stop::Control(Control::Regex), true) if replace => {
                Some(Stop::Control(Control::PreserveCase))
            }
            (Stop::Control(Control::PreserveCase), true) => [
                Control::Previous,
                Control::Next,
                Control::InSelection,
                Control::Close,
            ]
            .into_iter()
            .map(Stop::Control)
            .find(|stop| self.is_tab_stop(*stop, cx)),
            (Stop::Control(Control::Close), true) if replace => {
                let to = Stop::Control(Control::Replace);
                if !self.is_tab_stop(to, cx) {
                    self.focus_editor(window, cx);
                    return;
                }
                Some(to)
            }
            (Stop::Control(Control::Replace), false) => Some(Stop::Control(Control::Close)),
            _ => None,
        };
        if let Some(stop) = jump.filter(|stop| self.is_tab_stop(*stop, cx)) {
            self.focus_stop(stop, window, cx);
            return;
        }
        let ix = STOPS.iter().position(|s| *s == current).unwrap_or_default();
        let mut candidates: Box<dyn Iterator<Item = &Stop>> = if forward {
            Box::new(STOPS[ix + 1..].iter())
        } else {
            Box::new(STOPS[..ix].iter().rev())
        };
        match candidates.find(|stop| self.is_tab_stop(**stop, cx)) {
            Some(stop) => self.focus_stop(*stop, window, cx),
            None => self.focus_editor(window, cx),
        }
    }

    fn on_tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(true, window, cx);
    }

    fn on_shift_tab(&mut self, _: &Outdent, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(false, window, cx);
    }

    /// Presses `control` (a click, or Space or Enter while it's focused).
    fn activate(&mut self, control: Control, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.control_state(cx) else {
            return;
        };
        match control {
            Control::ToggleReplace => {
                if state.read_only {
                    return;
                }
                self.replace_visible = !self.replace_visible;
                let visible = self.replace_visible;
                self.with_editor(cx, |editor, cx| {
                    editor.set_find_replace_visible(visible, cx)
                });
                cx.notify();
            }
            Control::MatchCase => self.update_options(cx, |o| o.match_case = !o.match_case),
            Control::WholeWord => self.update_options(cx, |o| o.whole_word = !o.whole_word),
            Control::Regex => self.update_options(cx, |o| o.is_regex = !o.is_regex),
            Control::PreserveCase => {
                self.update_options(cx, |o| o.preserve_case = !o.preserve_case)
            }
            Control::InSelection => {
                if state.can_find_in_selection {
                    self.update_options(cx, |o| o.in_selection = !o.in_selection);
                }
            }
            Control::Previous => {
                if state.can_previous {
                    self.with_editor(cx, |editor, cx| editor.find_previous(cx));
                }
            }
            Control::Next => {
                if state.can_next {
                    self.with_editor(cx, |editor, cx| editor.find_next(cx));
                }
            }
            Control::Replace => {
                if state.can_replace {
                    self.with_editor(cx, |editor, cx| editor.replace_one(cx));
                }
            }
            Control::ReplaceAll => {
                if state.can_replace {
                    self.with_editor(cx, |editor, cx| editor.replace_all(cx));
                }
            }
            Control::Close => {
                if let Some(editor) = self.editor.upgrade() {
                    editor.update(cx, |editor, cx| editor.close_find(window, cx));
                }
            }
        }
    }

    /// The pointer over the whole window while the sash is dragged
    /// (Monaco's pointer capture on the sash).
    pub(crate) fn captured_cursor(&self) -> Option<gpui::CursorStyle> {
        self.resize.map(|_| SASH_CURSOR)
    }

    /// The widget's layout in the last frame, for tests: (width, input
    /// width, reduced, narrow, collapsed).
    pub(crate) fn layout_for_tests(&self) -> (Pixels, Pixels, bool, bool, bool) {
        let l = self.last_layout;
        (l.width, l.input_width, l.reduced, l.narrow, l.collapsed)
    }

    /// The title (tooltip) of the control with `id`, as rendered, for
    /// tests.
    pub(crate) fn title_for_tests(
        &self,
        id: &str,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<String> {
        let control = CONTROLS.iter().find(|c| c.id() == id)?;
        let focus = self.editor.upgrade()?.focus_handle(cx);
        Some(Self::title(*control, &focus, window))
    }

    /// Whether the control with `id` is enabled and shown (a tab stop),
    /// for tests.
    pub(crate) fn is_enabled_for_tests(&self, id: &str, cx: &gpui::App) -> bool {
        CONTROLS
            .iter()
            .find(|c| c.id() == id)
            .is_some_and(|c| self.is_tab_stop(Stop::Control(*c), cx))
    }

    /// The focused control's id, for tests.
    pub(crate) fn focused_control_for_tests(&self, window: &Window) -> Option<&'static str> {
        self.focused_control(window).map(Control::id)
    }

    /// Monaco's title: the label and the command's keybinding.
    fn title(control: Control, focus: &FocusHandle, window: &Window) -> String {
        let label = control.title().to_string();
        let keys = control.action().and_then(|action| {
            crate::keybinding_label::command_label(action.as_ref(), None, focus, window)
        });
        match keys {
            Some(keys) => format!("{label} ({keys})"),
            None => label,
        }
    }
}

/// What the controls' enabled states depend on (Monaco `_updateButtons`).
#[derive(Clone, Copy)]
struct ControlState {
    read_only: bool,
    can_previous: bool,
    can_next: bool,
    can_replace: bool,
    can_find_in_selection: bool,
}

/// A rendered control's look.
struct ControlLook<'a> {
    colors: &'a EditorColors,
    focused: bool,
    title: String,
    focus: FocusHandle,
}

/// A find option toggle (Monaco's `Toggle`: a 16px codicon in a 20px
/// box, `inputOption.*` colors when checked).
fn toggle(
    control: Control,
    icon: Codicon,
    checked: bool,
    enabled: bool,
    size: f32,
    look: ControlLook,
    cx: &mut Context<FindWidget>,
) -> impl IntoElement {
    let colors = look.colors;
    let tooltip = crate::tooltip::tooltip(look.title, colors);
    div()
        .id(control.id())
        .track_focus(&look.focus)
        .size(px(size))
        .ml(px(TOGGLE_MARGIN))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(TOGGLE_RADIUS))
        .border_1()
        .border_color(if look.focused {
            colors.focus_border
        } else if checked {
            colors.input_option_active_border
        } else {
            gpui::transparent_black()
        })
        .when(checked, |this| {
            this.bg(colors.input_option_active_background)
                .text_color(colors.input_option_active_foreground)
        })
        .when(!enabled, |this| this.text_color(colors.disabled_foreground))
        .when(enabled && !checked, |this| {
            let hover = colors.input_option_hover_background;
            this.hover(move |style| style.bg(hover)).cursor_pointer()
        })
        .child(codicon(icon))
        .tooltip(tooltip)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| this.activate(control, window, cx)))
}

/// A find widget button (Monaco's `SimpleButton`: a 16px codicon in a
/// 22px box; `disabledForeground` and no hover when disabled).
fn button(
    control: Control,
    icon: Codicon,
    enabled: bool,
    look: ControlLook,
    cx: &mut Context<FindWidget>,
) -> gpui::Stateful<gpui::Div> {
    let colors = look.colors;
    let tooltip = crate::tooltip::tooltip(look.title, colors);
    let hover = colors.clone();
    div()
        .id(control.id())
        .track_focus(&look.focus)
        .size(px(BUTTON_SIZE))
        .ml(px(BUTTON_MARGIN))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(BUTTON_RADIUS))
        .border_1()
        .border_color(if look.focused {
            colors.focus_border
        } else {
            gpui::transparent_black()
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| hover.toolbar_hover(style))
        })
        .when(!enabled, |this| this.text_color(colors.disabled_foreground))
        .child(codicon(icon))
        .tooltip(tooltip)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| this.activate(control, window, cx)))
}

impl Render for FindWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div().into_any_element();
        };
        let Some(state) = self.control_state(cx) else {
            return div().into_any_element();
        };
        let (colors, options, status, matches_count, invalid, layout_info, editor_focus) = {
            let editor = editor.read(cx);
            (
                editor.colors().clone(),
                editor.find_options().clone(),
                editor.find_status(),
                editor.find_matches_count(),
                editor.is_find_query_invalid(),
                editor.layout_info(),
                editor.focus_handle(cx),
            )
        };
        let replace_visible = self.replace_visible && !state.read_only;
        let reveal = self.reveal_progress(cx);
        if reveal < 1. {
            window.request_animation_frame();
        }
        // Monaco `MAX_MATCHES_COUNT_WIDTH`: the count's box grows to fit.
        let status_label: SharedString = status.clone().unwrap_or_default().into();
        if !status_label.is_empty() {
            let text_width = window
                .text_system()
                .layout_line(
                    &status_label,
                    px(12.),
                    &[gpui::TextRun {
                        len: status_label.len(),
                        font: window.text_style().font(),
                        color: colors.widget_foreground,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
                .width;
            // `padding-left: 2px`.
            self.matches_width = self.matches_width.max(text_width + px(2.));
        }
        let (editor_width, content_width, minimap_width) = layout_info
            .map_or((px(800.), px(800.), px(0.)), |l| {
                (l.width, l.content_width, l.minimap.minimap_width)
            });
        let layout = FindWidgetLayout::compute(
            editor_width,
            content_width,
            minimap_width,
            self.width,
            self.resized,
            self.matches_width,
        );
        self.last_layout = layout;
        if layout.hidden {
            return div().into_any_element();
        }
        let editor_keys = crate::context_keys::editor_keys_for_widgets(&self.editor, cx);
        let find_box = input_box_height(self.find_input.read(cx), cx);
        let replace_box = input_box_height(self.replace_input.read(cx), cx);
        let height = widget_height(
            self.find_input.read(cx),
            replace_visible.then(|| self.replace_input.read(cx)),
            cx,
        );
        let find_focused = self.find_input.focus_handle(cx).is_focused(window);
        let replace_focused = self.replace_input.focus_handle(cx).is_focused(window);
        // Monaco shows "No results" in `errorForeground`.
        let no_results = !options.query.is_empty() && matches_count == 0;
        let regex_error = (options.is_regex && invalid)
            .then(|| crate::search::regex_error(&options.query))
            .flatten();
        let focused = self.focused_control(window);
        let titles: Vec<String> = CONTROLS
            .iter()
            .map(|control| Self::title(*control, &editor_focus, window))
            .collect();
        let handles = self.controls.clone();
        let look = |control: Control| ControlLook {
            colors: &colors,
            focused: focused == Some(control),
            title: titles[control.index()].clone(),
            focus: handles[control.index()].clone(),
        };

        // Monaco's `InputBox`: `input.background`, `input.border` (or
        // `focusBorder` while focused, `inputValidation.errorBorder` when
        // the regex doesn't parse), the text padded right of the toggles.
        let input_box = |input: &Entity<CodeEditor>,
                         context: &'static str,
                         height: Pixels,
                         focused: bool,
                         error: bool,
                         controls_width: f32| {
            div()
                .relative()
                .flex_none()
                .w(layout.input_width)
                .h(height)
                .rounded(px(INPUT_RADIUS))
                .bg(colors.input_background)
                .border_1()
                .border_color(if error {
                    colors.input_validation_error_border
                } else if focused {
                    colors.focus_border
                } else {
                    colors.input_border.unwrap_or(gpui::transparent_black())
                })
                .child(
                    div()
                        .key_context(crate::context_keys::widget_key_context(
                            context,
                            &editor_keys,
                        ))
                        .absolute()
                        .top(px(2.))
                        .bottom(px(2.))
                        .left(px(INPUT_PADDING_LEFT))
                        .right(px(controls_width + 2.))
                        .child(input.clone()),
                )
        };
        // `.controls`: `position: absolute; top: 3px; right: 2px`.
        let controls = || div().absolute().top(px(2.)).right(px(2.)).flex();

        let find_controls_width = if layout.collapsed {
            0.
        } else {
            3. * (TOGGLE_SIZE + TOGGLE_MARGIN)
        };
        let find_part = div()
            .flex()
            .flex_none()
            .child(
                input_box(
                    &self.find_input,
                    FIND_INPUT_KEY_CONTEXT,
                    find_box,
                    find_focused,
                    regex_error.is_some(),
                    find_controls_width,
                )
                .when(!layout.collapsed, |this| {
                    this.child(
                        controls()
                            .child(toggle(
                                Control::MatchCase,
                                Codicon::CaseSensitive,
                                options.match_case,
                                true,
                                TOGGLE_SIZE,
                                look(Control::MatchCase),
                                cx,
                            ))
                            .child(toggle(
                                Control::WholeWord,
                                Codicon::WholeWord,
                                options.whole_word,
                                true,
                                TOGGLE_SIZE,
                                look(Control::WholeWord),
                                cx,
                            ))
                            .child(toggle(
                                Control::Regex,
                                Codicon::Regex,
                                options.is_regex,
                                true,
                                TOGGLE_SIZE,
                                look(Control::Regex),
                                cx,
                            )),
                    )
                }),
            )
            .child(
                // `.find-actions`.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(25.))
                    .when(!layout.reduced, |this| {
                        let limit = matches_count >= crate::MATCHES_LIMIT;
                        this.child(
                            div()
                                .id("matches-count")
                                .flex()
                                .flex_none()
                                .justify_center()
                                .ml(px(3.))
                                .pl(px(2.))
                                .w(self.matches_width)
                                .whitespace_nowrap()
                                .when(no_results, |this| this.text_color(colors.error_foreground))
                                .when(limit, |this| {
                                    this.tooltip(crate::tooltip::tooltip(
                                        MATCHES_LIMIT_TITLE,
                                        &colors,
                                    ))
                                })
                                .child(if options.query.is_empty() {
                                    // Monaco shows "No results" before typing.
                                    SharedString::from("No results")
                                } else {
                                    status_label.clone()
                                }),
                        )
                    })
                    .when(!layout.collapsed, |this| {
                        this.child(button(
                            Control::Previous,
                            Codicon::ArrowUp,
                            state.can_previous,
                            look(Control::Previous),
                            cx,
                        ))
                        .child(button(
                            Control::Next,
                            Codicon::ArrowDown,
                            state.can_next,
                            look(Control::Next),
                            cx,
                        ))
                    })
                    .child(toggle(
                        Control::InSelection,
                        Codicon::Selection,
                        options.in_selection,
                        state.can_find_in_selection,
                        BUTTON_SIZE,
                        look(Control::InSelection),
                        cx,
                    )),
            );

        let replace_part = replace_visible.then(|| {
            div()
                .flex()
                .flex_none()
                .mt(px(PART_MARGIN_TOP))
                .child(
                    input_box(
                        &self.replace_input,
                        REPLACE_INPUT_KEY_CONTEXT,
                        replace_box,
                        replace_focused,
                        false,
                        TOGGLE_SIZE + TOGGLE_MARGIN,
                    )
                    .child(controls().child(toggle(
                        Control::PreserveCase,
                        Codicon::PreserveCase,
                        options.preserve_case,
                        true,
                        TOGGLE_SIZE,
                        look(Control::PreserveCase),
                        cx,
                    ))),
                )
                .child(
                    // `.replace-actions`.
                    div().flex().flex_none().items_center().h(px(25.)).when(
                        !layout.collapsed,
                        |this| {
                            this.child(button(
                                Control::Replace,
                                Codicon::Replace,
                                state.can_replace,
                                look(Control::Replace),
                                cx,
                            ))
                            .child(button(
                                Control::ReplaceAll,
                                Codicon::ReplaceAll,
                                state.can_replace,
                                look(Control::ReplaceAll),
                                cx,
                            ))
                        },
                    ),
                )
        });

        // The replace toggle: a full-height column at the left (`.button.toggle.left`),
        // hidden when the editor is read-only.
        let toggle_replace = (!state.read_only).then(|| {
            let hover = colors.clone();
            let control = Control::ToggleReplace;
            let look = look(control);
            div()
                .id(control.id())
                .track_focus(&look.focus)
                .absolute()
                .left(px(0.))
                .top(px(0.))
                .bottom(px(0.))
                .w(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(CORNER_RADIUS_SMALL))
                .border_1()
                .border_color(if look.focused {
                    colors.focus_border
                } else {
                    gpui::transparent_black()
                })
                .cursor_pointer()
                .hover(move |style| hover.toolbar_hover(style))
                .child(codicon(if replace_visible {
                    // `find-expanded`.
                    Codicon::ChevronDown
                } else {
                    // `find-collapsed`.
                    Codicon::ChevronRight
                }))
                .tooltip(crate::tooltip::tooltip(look.title, &colors))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.activate(control, window, cx)),
                )
        });

        // The close button: `position: absolute; top: 5px; right: 4px`.
        let close = button(
            Control::Close,
            Codicon::Close,
            true,
            look(Control::Close),
            cx,
        )
        .absolute()
        .top(px(4.))
        .right(px(PADDING_RIGHT - 1.));

        // Monaco's sash: dragging the left edge resizes the widget; a
        // double click switches between the initial and widest widths
        // (`onDidReset`). Drawn in `editorWidget.resizeBorder`.
        let resizing = self.resize;
        let max_width = editor_width - px(28.) - minimap_width - px(15.);
        let sash = div()
            .id("find-sash")
            .debug_selector(|| "find-sash".into())
            .absolute()
            .left(px(0.))
            .top(px(0.))
            .bottom(px(0.))
            .w(px(SASH_HIT_SIZE))
            .cursor(SASH_CURSOR)
            .child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .bottom(px(0.))
                    .w(px(SASH_SIZE))
                    .bg(colors.widget_resize_border.unwrap_or(colors.widget_border)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let current = this.last_layout.width;
                    if event.click_count == 2 {
                        let initial = px(FIND_WIDGET_INITIAL_WIDTH);
                        if current < initial {
                            // Narrow editors: the layout decides.
                            return;
                        }
                        if !this.resized || current == initial {
                            this.width = max_width;
                            this.resized = true;
                        } else {
                            this.width = initial;
                        }
                    } else {
                        this.resize = Some((event.position.x, current));
                    }
                    cx.notify();
                }),
            )
            // While dragging: follow the mouse anywhere in the window, with
            // the sash's pointer.
            .children(resizing.map(|(start_x, start_width)| {
                crate::pointer_capture::capture(
                    cx.entity().downgrade(),
                    self.captured_cursor(),
                    move |this: &mut Self, event, cx| {
                        // Monaco `onDidChange`: narrower than the initial
                        // width or wider than the maximum is ignored.
                        let width = start_width + (start_x - event.position.x);
                        if width >= px(FIND_WIDGET_INITIAL_WIDTH) && width <= max_width {
                            this.width = width;
                            this.resized = true;
                            cx.notify();
                        }
                    },
                    |this, cx| {
                        this.resize = None;
                        cx.notify();
                    },
                )
            }));

        // Measures the painted height for the editor's view zone.
        let measured = self.measured.clone();
        let weak_editor = self.editor.clone();
        let measure = gpui::canvas(
            move |bounds, _, cx| {
                // The box, without its margin.
                let report = (height, bounds.size.height - px(MARGIN_TOP));
                if measured.get() != Some(report) {
                    measured.set(Some(report));
                    cx.defer(move |cx| {
                        if let Some(editor) = weak_editor.upgrade() {
                            editor.update(cx, |editor, cx| {
                                editor.find_widget_measured(report.0, report.1, cx)
                            });
                        }
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let widget = div()
            .id("find-widget")
            .debug_selector(|| "find-widget".into())
            .relative()
            .flex_none()
            .mt(px(MARGIN_TOP))
            .w(layout.width)
            .h(height)
            .overflow_hidden()
            .pl(px(PADDING_LEFT))
            .pr(px(PADDING_RIGHT))
            .bg(colors.widget_background)
            .text_color(colors.widget_foreground)
            .text_size(px(12.))
            .line_height(px(INPUT_LINE_HEIGHT))
            .border_1()
            .border_color(colors.widget_outline.unwrap_or(gpui::transparent_black()))
            .rounded(px(CORNER_RADIUS_LARGE))
            .when_some(colors.widget_shadow, |this, shadow| {
                this.shadow(vec![gpui::BoxShadow {
                    color: shadow,
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(8.),
                    spread_radius: px(2.),
                    inset: false,
                }])
            })
            .children(toggle_replace)
            .child(
                div()
                    .mt(px(PART_MARGIN_TOP))
                    .ml(px(PART_MARGIN_LEFT))
                    .mr(px(PART_MARGIN_RIGHT))
                    .flex()
                    .flex_col()
                    .child(find_part)
                    .children(replace_part),
            )
            .child(close)
            .child(sash);

        // The regex error under the find input (Monaco's `InputBox`
        // message, shown while the input has focus), outside the widget's
        // clip.
        let message = regex_error.filter(|_| find_focused).map(|message| {
            div()
                .absolute()
                .top(px(MARGIN_TOP + 1. + PART_MARGIN_TOP) + find_box - px(1.))
                .left(px(1. + PADDING_LEFT + PART_MARGIN_LEFT))
                .w(layout.input_width)
                .p(px(12. * 0.4))
                .text_size(px(12.))
                .line_height(px(17.))
                .bg(colors.input_validation_error_background)
                .border_1()
                .border_color(colors.input_validation_error_border)
                .when_some(colors.input_validation_error_foreground, |this, color| {
                    this.text_color(color)
                })
                .child(message)
        });

        div()
            .key_context(crate::context_keys::widget_key_context(
                FIND_WIDGET_KEY_CONTEXT,
                &editor_keys,
            ))
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_input_newline))
            .on_action(cx.listener(Self::on_focus_editor))
            .capture_action(cx.listener(Self::on_paste))
            .capture_action(cx.listener(
                |this, action: &crate::actions::AddSelectionToNextFindMatch, window, cx| {
                    this.capture_multi_cursor(action, window, cx)
                },
            ))
            .capture_action(cx.listener(
                |this, action: &crate::actions::AddSelectionToPreviousFindMatch, window, cx| {
                    this.capture_multi_cursor(action, window, cx)
                },
            ))
            .capture_action(cx.listener(
                |this, action: &crate::actions::MoveSelectionToNextFindMatch, window, cx| {
                    this.capture_multi_cursor(action, window, cx)
                },
            ))
            .capture_action(cx.listener(
                |this, action: &crate::actions::MoveSelectionToPreviousFindMatch, window, cx| {
                    this.capture_multi_cursor(action, window, cx)
                },
            ))
            .capture_action(cx.listener(
                |this, action: &crate::actions::SelectAllOccurrences, window, cx| {
                    this.capture_multi_cursor(action, window, cx)
                },
            ))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(Self::on_tab))
            .on_action(cx.listener(Self::on_shift_tab))
            .on_action(cx.listener(Self::on_history_previous))
            .on_action(cx.listener(Self::on_history_next))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .relative()
            .flex_none()
            // Monaco's slide-in: `translateY(calc(-100% - 10px))` to 0 in
            // 200ms, linearly. The editor places this root by its origin,
            // so a child moves.
            .child(
                // A flex box: the widget's margin stays inside it.
                div()
                    .relative()
                    .flex()
                    .flex_none()
                    .top(-(height + px(REVEAL_SHADOW)) * (1. - reveal))
                    .child(widget)
                    .child(measure)
                    .children(message),
            )
            .into_any_element()
    }
}
