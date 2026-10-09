//! The find/replace widget (Monaco's `findWidget`): an overlay widget whose
//! inputs are single-line [`CodeEditor`]s.

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, div, px,
};

use crate::actions::{Newline, Outdent, RemoveSecondaryCursors, Tab};
use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::find::FindOptions;
use crate::model::TextModelEvent;
use crate::options::EditorOptions;
use crate::theme::EditorColors;

pub struct FindWidget {
    editor: WeakEntity<CodeEditor>,
    find_input: Entity<CodeEditor>,
    replace_input: Entity<CodeEditor>,
    replace_visible: bool,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
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
        let mut editor = CodeEditor::with_text("", window, cx);
        editor.update_options(EditorOptions::single_line_input(), cx);
        if let Some(theme) = theme {
            editor.set_theme(theme, cx);
        }
        editor
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
            _subscriptions: subscriptions,
        }
    }

    fn input_text(input: &Entity<CodeEditor>, cx: &gpui::App) -> String {
        input.read(cx).model().read(cx).text()
    }

    /// Sends the inputs' text to the editor's find options.
    fn push_options(&mut self, cx: &mut Context<Self>) {
        let query = Self::input_text(&self.find_input, cx);
        let replace = Self::input_text(&self.replace_input, cx);
        self.update_options(cx, |options| {
            options.query = query;
            options.replace = replace;
        });
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

    /// Shows `options` in the inputs (when opened with a seeded query).
    pub fn sync_from_editor(
        &mut self,
        options: &FindOptions,
        replace_visible: bool,
        _window: &mut Window,
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

    fn on_enter(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.replace_input.focus_handle(cx).is_focused(window) {
            self.with_editor(cx, |editor, cx| editor.replace_one(cx));
        } else {
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
        })
        .cursor_pointer()
        .child(label)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| this.update_options(cx, |o| on_click(o))))
}

fn button(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<FindWidget>,
    on_click: impl Fn(&mut FindWidget, &mut Window, &mut Context<FindWidget>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(4.))
        .rounded(px(3.))
        .cursor_pointer()
        .hover(|style| style.bg(gpui::hsla(0., 0., 0.5, 0.2)))
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
        let (colors, options, status, line_height) = {
            let editor = editor.read(cx);
            (
                editor.colors().clone(),
                editor.find_options().clone(),
                editor.find_status(),
                editor.options().resolved_line_height(),
            )
        };
        let no_results = status.as_deref() == Some("No results");
        let input_box = |input: &Entity<CodeEditor>, error: bool| {
            div()
                .w(px(220.))
                .h(line_height + px(4.))
                .px(px(2.))
                .py(px(2.))
                .bg(colors.input_background)
                .border_1()
                .border_color(if error {
                    gpui::red().opacity(0.6)
                } else {
                    gpui::transparent_black()
                })
                .child(input.clone())
        };
        let status_label: SharedString = status.unwrap_or_default().into();

        div()
            .key_context("FindWidget")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(Self::on_tab))
            .on_action(cx.listener(Self::on_shift_tab))
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
                        cx,
                        |this, _, cx| {
                            this.replace_visible = !this.replace_visible;
                            cx.notify();
                        },
                    ))
                    .child(input_box(&self.find_input, no_results))
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
                    .child(div().min_w(px(70.)).child(status_label))
                    .child(button("previous", "↑", cx, |this, _, cx| {
                        this.with_editor(cx, |editor, cx| editor.find_previous(cx))
                    }))
                    .child(button("next", "↓", cx, |this, _, cx| {
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
                    .child(button("close", "×", cx, |this, window, cx| {
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
                        .child(input_box(&self.replace_input, false))
                        .child(button("replace", "Replace", cx, |this, _, cx| {
                            this.with_editor(cx, |editor, cx| editor.replace_one(cx))
                        }))
                        .child(button("replace-all", "All", cx, |this, _, cx| {
                            this.with_editor(cx, |editor, cx| editor.replace_all(cx))
                        })),
                )
            })
    }
}
