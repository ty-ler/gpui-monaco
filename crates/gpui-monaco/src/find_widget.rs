//! The find/replace widget (Monaco's `findWidget`): an overlay widget whose
//! inputs are single-line [`CodeEditor`]s that grow with their lines
//! (Monaco's `flexibleHeight` inputs: Ctrl+Enter and pasting insert line
//! breaks).

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, div, px,
};

use crate::actions::{
    FindHistoryNext, FindHistoryPrevious, FindInputNewline, Newline, Outdent, Paste,
    RemoveSecondaryCursors, Tab,
};
use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::find::FindHistory;
use crate::find::FindOptions;
use crate::model::TextModelEvent;
use crate::theme::EditorColors;

/// The find and replace widget, shown as an overlay at the top right of the
/// editor by the find commands.
pub struct FindWidget {
    editor: WeakEntity<CodeEditor>,
    find_input: Entity<CodeEditor>,
    replace_input: Entity<CodeEditor>,
    replace_visible: bool,
    focus_handle: FocusHandle,
    /// Where Up and Down are in each input's history.
    find_history_index: Option<usize>,
    replace_history_index: Option<usize>,
    /// Set while history navigation writes an input.
    navigating_history: bool,
    /// Monaco's `_updateHistoryDelayer`.
    history_task: Option<gpui::Task<()>>,
    /// The inputs' width, changed by dragging the left edge (Monaco's
    /// sash).
    input_width: gpui::Pixels,
    /// The drag: where it started and the width then.
    resize: Option<(gpui::Pixels, gpui::Pixels)>,
    _subscriptions: Vec<Subscription>,
}

/// The inputs' width before resizing (Monaco's 419px widget).
const INPUT_WIDTH: f32 = 220.;
/// Monaco's delay before a typed search string joins the history.
const HISTORY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
/// Monaco's `flexibleMaxHeight` for the find and replace inputs; taller
/// text scrolls.
const INPUT_MAX_HEIGHT: f32 = 118.;

/// An input's height for `text`: a row per line, up to Monaco's
/// `flexibleMaxHeight`, plus the box's padding.
pub(crate) fn input_height(text: &str, line_height: gpui::Pixels) -> gpui::Pixels {
    let lines = text.split('\n').count().max(1) as f32;
    (line_height * lines + px(4.)).min(px(INPUT_MAX_HEIGHT).max(line_height + px(4.)))
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

impl Focusable for FindWidget {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn input(
    window: &mut Window,
    cx: &mut Context<FindWidget>,
    theme: Option<std::sync::Arc<crate::Theme>>,
) -> Entity<CodeEditor> {
    cx.new(|cx| {
        let theme = theme.unwrap_or_else(|| {
            crate::theme::global_theme(cx)
                .unwrap_or_else(|| std::sync::Arc::new(crate::Theme::vs_dark()))
        });
        CodeEditor::new_widget_input("", theme, window, cx)
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
        let find_input = input(window, cx, theme.clone());
        let replace_input = input(window, cx, theme);
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
            find_history_index: None,
            replace_history_index: None,
            navigating_history: false,
            history_task: None,
            input_width: px(INPUT_WIDTH),
            resize: None,
            _subscriptions: subscriptions,
        }
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

    fn on_enter(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.replace_visible && self.find_input.focus_handle(cx).is_focused(window) {
            Self::focus_input(&self.replace_input, window, cx);
        } else {
            Self::focus_input(&self.find_input, window, cx);
        }
    }

    fn on_shift_tab(&mut self, _: &Outdent, window: &mut Window, cx: &mut Context<Self>) {
        Self::focus_input(&self.find_input, window, cx);
    }
}

fn toggle(
    id: &'static str,
    label: &'static str,
    active: bool,
    colors: &EditorColors,
    cx: &mut Context<FindWidget>,
    on_click: impl Fn(&mut FindOptions) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(3.))
        .rounded(px(3.))
        .border_1()
        .border_color(if active {
            colors.input_option_active_border
        } else {
            gpui::transparent_black()
        })
        .when(active, |this| {
            this.bg(colors.input_option_active_background)
                .text_color(colors.input_option_active_foreground)
        })
        .when(!active, |this| {
            let hover = colors.input_option_hover_background;
            this.hover(move |style| style.bg(hover))
        })
        .cursor_pointer()
        .child(label)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| this.update_options(cx, |o| on_click(o))))
}

fn button(
    id: &'static str,
    label: &'static str,
    colors: &EditorColors,
    cx: &mut Context<FindWidget>,
    on_click: impl Fn(&mut FindWidget, &mut Window, &mut Context<FindWidget>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(4.))
        .rounded(px(3.))
        .cursor_pointer()
        .hover(|style| colors.toolbar_hover(style))
        .child(label)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
}

use gpui::prelude::FluentBuilder as _;

impl Render for FindWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div();
        };
        let (colors, options, status, line_height, invalid, max_width) = {
            let editor = editor.read(cx);
            (
                editor.colors().clone(),
                editor.find_options().clone(),
                editor.find_status(),
                editor.options().resolved_line_height(),
                editor.is_find_query_invalid(),
                editor
                    .layout
                    .as_ref()
                    .map_or(px(800.), |l| l.text_bounds.size.width - px(260.)),
            )
        };
        let max_width = max_width.max(px(INPUT_WIDTH));
        // Monaco shows "No results" in `errorForeground`, and an invalid
        // regex as an error on the input.
        let no_results = status.as_deref() == Some("No results");
        let input_width = self.input_width.min(max_width);
        let find_height = input_height(&Self::input_text(&self.find_input, cx), line_height);
        let replace_height = input_height(&Self::input_text(&self.replace_input, cx), line_height);
        let input_box = |input: &Entity<CodeEditor>, height: gpui::Pixels, error: bool| {
            div()
                .w(input_width)
                .h(height)
                .px(px(2.))
                .py(px(2.))
                .bg(colors.input_background)
                .border_1()
                .border_color(if error {
                    colors.input_validation_error_border
                } else {
                    gpui::transparent_black()
                })
                .child(input.clone())
        };
        let status_label: SharedString = status.unwrap_or_default().into();
        // Dragging the left edge resizes the widget (Monaco's sash); a
        // double click switches between the initial and widest widths.
        let entity = cx.entity().downgrade();
        let resizing = self.resize;
        let sash = div()
            .id("find-sash")
            .absolute()
            .left(px(0.))
            .top(px(0.))
            .bottom(px(0.))
            .w(px(4.))
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        this.input_width = if this.input_width > px(INPUT_WIDTH) {
                            px(INPUT_WIDTH)
                        } else {
                            px(f32::MAX)
                        };
                    } else {
                        this.resize = Some((event.position.x, this.input_width));
                    }
                    cx.notify();
                }),
            )
            .child(gpui::canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let Some((start_x, start_width)) = resizing else {
                        return;
                    };
                    let moved = entity.clone();
                    window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                        if phase == gpui::DispatchPhase::Capture
                            && let Some(this) = moved.upgrade()
                        {
                            this.update(cx, |this, cx| {
                                let width =
                                    start_width.min(max_width) + (start_x - event.position.x);
                                this.input_width = width.clamp(px(INPUT_WIDTH), max_width);
                                cx.notify();
                            });
                        }
                    });
                    let released = entity.clone();
                    window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                        if phase == gpui::DispatchPhase::Capture
                            && let Some(this) = released.upgrade()
                        {
                            this.update(cx, |this, cx| {
                                this.resize = None;
                                cx.notify();
                            });
                        }
                    });
                },
            ));

        div()
            .key_context("FindWidget")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_input_newline))
            .capture_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(Self::on_tab))
            .on_action(cx.listener(Self::on_shift_tab))
            .on_action(cx.listener(Self::on_history_previous))
            .on_action(cx.listener(Self::on_history_next))
            .relative()
            .child(sash)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(4.))
            .mr(px(14.))
            .bg(colors.widget_background)
            .text_color(colors.widget_foreground)
            .text_size(px(12.))
            .border_1()
            .border_color(colors.widget_border)
            .shadow_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(button(
                        "toggle-replace",
                        if self.replace_visible { "▾" } else { "▸" },
                        &colors,
                        cx,
                        |this, _, cx| {
                            this.replace_visible = !this.replace_visible;
                            let visible = this.replace_visible;
                            this.with_editor(cx, |editor, cx| {
                                editor.set_find_replace_visible(visible, cx)
                            });
                            cx.notify();
                        },
                    ))
                    .child(input_box(&self.find_input, find_height, invalid))
                    .child(toggle(
                        "match-case",
                        "Aa",
                        options.match_case,
                        &colors,
                        cx,
                        |o| o.match_case = !o.match_case,
                    ))
                    .child(toggle(
                        "whole-word",
                        "ab",
                        options.whole_word,
                        &colors,
                        cx,
                        |o| o.whole_word = !o.whole_word,
                    ))
                    .child(toggle("regex", ".*", options.is_regex, &colors, cx, |o| {
                        o.is_regex = !o.is_regex
                    }))
                    .child(
                        div()
                            .min_w(px(70.))
                            .when(no_results && !options.query.is_empty(), |this| {
                                this.text_color(colors.error_foreground)
                            })
                            .child(status_label),
                    )
                    .child(button("previous", "↑", &colors, cx, |this, _, cx| {
                        this.with_editor(cx, |editor, cx| editor.find_previous(cx))
                    }))
                    .child(button("next", "↓", &colors, cx, |this, _, cx| {
                        this.with_editor(cx, |editor, cx| editor.find_next(cx))
                    }))
                    .child(toggle(
                        "in-selection",
                        "≡",
                        options.in_selection,
                        &colors,
                        cx,
                        |o| o.in_selection = !o.in_selection,
                    ))
                    .child(button("close", "×", &colors, cx, |this, window, cx| {
                        if let Some(editor) = this.editor.upgrade() {
                            editor.update(cx, |editor, cx| editor.close_find(window, cx));
                        }
                    })),
            )
            .when(self.replace_visible, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .pl(px(18.))
                        .child(input_box(&self.replace_input, replace_height, false))
                        .child(toggle(
                            "preserve-case",
                            "AB",
                            options.preserve_case,
                            &colors,
                            cx,
                            |o| o.preserve_case = !o.preserve_case,
                        ))
                        .child(button("replace", "Replace", &colors, cx, |this, _, cx| {
                            this.with_editor(cx, |editor, cx| editor.replace_one(cx))
                        }))
                        .child(button("replace-all", "All", &colors, cx, |this, _, cx| {
                            this.with_editor(cx, |editor, cx| editor.replace_all(cx))
                        })),
                )
            })
    }
}
