//! The inline suggestion toolbar (Monaco `InlineSuggestionHintsContentWidget`
//! and `InlineCompletionsHoverParticipant`): previous/next with the "1/3"
//! count, Accept and Accept Word, and a menu with Accept Line, Always Show
//! Toolbar and the provider's commands. It shows above the ghost text with
//! `show_toolbar: Always`, or in the hover over ghost text with `OnHover`.
//! A warning on the shown completion shows above it (Monaco's
//! `warningMessage`). The menu takes the keyboard while open, as Monaco's
//! context menu: Up, Down, Home and End move, Enter or Space runs, Escape
//! closes.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window,
    div, px,
};

use crate::actions::{
    ContextMenuFirst, ContextMenuHide, ContextMenuLast, ContextMenuNext, ContextMenuPrevious,
    ContextMenuSelect,
};
use crate::codicons::{Codicon, icon as codicon};
use crate::context_menu::CONTEXT_MENU_KEY_CONTEXT;
use crate::editor::CodeEditor;

/// The More Actions menu paints above the editor's overflowing widgets,
/// the hover included.
const MENU_PRIORITY: usize = 1000;
use crate::options::InlineSuggestShowToolbar;
use crate::theme::Theme;

/// What a toolbar button does to the editor.
type ToolbarAction = Arc<dyn Fn(&mut CodeEditor, &mut Context<CodeEditor>)>;

/// The inline suggestion toolbar.
pub struct InlineToolbar {
    editor: WeakEntity<CodeEditor>,
    theme: Arc<Theme>,
    /// Drawn as its own card (Monaco `withBorder`), not inside a hover.
    with_border: bool,
    /// The "⋯" menu is open.
    menu_open: bool,
    /// The menu's keyboard-selected row.
    active: Option<usize>,
    /// The menu's focus, while it's open.
    focus: FocusHandle,
    /// The menu had the keyboard at the last frame.
    focus_seen: bool,
}

/// A menu row: its label, whether it's checked, and what it runs.
type MenuItem = (SharedString, bool, ToolbarAction);

/// Monaco's menu navigation: the next (`delta` 1) or previous (-1) row,
/// wrapping, or the first or last for larger steps.
pub(crate) fn step_menu(active: Option<usize>, count: usize, delta: isize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let count = count as isize;
    let next = match (active, delta) {
        (Some(active), 1 | -1) => (active as isize + delta).rem_euclid(count),
        (None, 1) => 0,
        (None, -1) => count - 1,
        (_, delta) if delta > 0 => count - 1,
        _ => 0,
    };
    Some(next as usize)
}

impl InlineToolbar {
    pub(crate) fn new(
        editor: &Entity<CodeEditor>,
        theme: Arc<Theme>,
        with_border: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        // Monaco's autoruns: follow the count and commands.
        cx.observe(editor, |_, _, cx| cx.notify()).detach();
        Self {
            editor: editor.downgrade(),
            theme,
            with_border,
            menu_open: false,
            active: None,
            focus: cx.focus_handle(),
            focus_seen: false,
        }
    }

    /// The menu's rows: Accept Line, Always Show Toolbar, then the
    /// provider's commands.
    fn menu_items(&self, always: bool, commands: Vec<crate::providers::Command>) -> Vec<MenuItem> {
        let mut items: Vec<MenuItem> = vec![
            (
                "Accept Line".into(),
                false,
                Arc::new(|editor: &mut CodeEditor, cx: &mut Context<CodeEditor>| {
                    editor.accept_next_inline_line(cx);
                }),
            ),
            (
                "Always Show Toolbar".into(),
                always,
                Arc::new(|editor: &mut CodeEditor, cx: &mut Context<CodeEditor>| {
                    editor.toggle_always_show_inline_toolbar(cx)
                }),
            ),
        ];
        for command in commands {
            let title = command.title.clone();
            items.push((
                title,
                false,
                Arc::new(move |_: &mut CodeEditor, cx: &mut Context<CodeEditor>| {
                    if let Err(err) = crate::commands::execute_command(cx, &command) {
                        log::warn!("{err:#}");
                    }
                }),
            ));
        }
        items
    }

    fn current_menu_items(&self, cx: &gpui::App) -> Vec<MenuItem> {
        let Some(editor) = self.editor.upgrade() else {
            return Vec::new();
        };
        let editor = editor.read(cx);
        self.menu_items(
            editor.options().inline_suggest.show_toolbar == InlineSuggestShowToolbar::Always,
            editor.inline.commands.clone(),
        )
    }

    /// Opens or closes the menu; opened, it takes the keyboard.
    fn set_menu_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_open = open;
        self.active = None;
        if open {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Closes the menu and gives the editor the keyboard back.
    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_menu_open(false, window, cx);
        if let Some(editor) = self.editor.upgrade() {
            let focus = editor.read(cx).focus_handle.clone();
            window.focus(&focus, cx);
        }
    }

    fn move_active(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.current_menu_items(cx).len();
        self.active = step_menu(self.active, count, delta);
        cx.notify();
    }

    /// Runs the keyboard-selected row.
    fn run_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        let Some((_, _, run)) = self.current_menu_items(cx).into_iter().nth(active) else {
            return;
        };
        self.close_menu(window, cx);
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| run(editor, cx));
        }
    }

    /// Whether the menu is open, and its selected row, for tests.
    #[doc(hidden)]
    pub(crate) fn menu_for_tests(&self) -> Option<Option<usize>> {
        self.menu_open.then_some(self.active)
    }

    /// Opens the menu as clicking "⋯" does, for tests.
    #[doc(hidden)]
    pub(crate) fn open_menu_for_tests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_menu_open(true, window, cx);
    }

    fn button(
        &self,
        id: &'static str,
        label: impl IntoElement,
        keys: Option<&'static str>,
        enabled: bool,
        run: ToolbarAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = &self.theme.colors;
        let editor = self.editor.clone();
        div()
            .id(id)
            .flex()
            .items_center()
            .px(px(3.))
            .rounded(px(3.))
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(colors.scrollbar_slider_hover))
            })
            .when(!enabled, |this| this.opacity(0.4))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.menu_open = false;
                    cx.notify();
                    if !enabled {
                        return;
                    }
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| run(editor, cx));
                    }
                }),
            )
            .child(label)
            .children(keys.map(|keys| div().ml(px(4.)).opacity(0.6).child(keys)))
            .into_any_element()
    }

    fn menu_item(
        &self,
        id: (&'static str, usize),
        (label, checked, run): MenuItem,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = &self.theme.colors;
        let editor = self.editor.clone();
        let ix = id.1;
        let active = self.active == Some(ix);
        div()
            .id(id)
            .flex()
            .gap(px(6.))
            .px(px(6.))
            .py(px(2.))
            .cursor_pointer()
            .when(active, |this| {
                this.bg(colors.menu_selection_background)
                    .text_color(colors.menu_selection_foreground)
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                // Monaco's menus follow the mouse.
                if *hovered && this.active != Some(ix) {
                    this.active = Some(ix);
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_menu(window, cx);
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| run(editor, cx));
                    }
                }),
            )
            .child(div().w(px(16.)).when(checked, |this| {
                // `menu-selection`: the `check` codicon.
                this.child(codicon(Codicon::MenuSelection))
            }))
            .child(label)
            .into_any_element()
    }
}

impl Render for InlineToolbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div().into_any_element();
        };
        let (count, always, commands, warning, markdown_style) = {
            let editor = editor.read(cx);
            (
                editor.inline_suggestion_index(cx),
                editor.options().inline_suggest.show_toolbar == InlineSuggestShowToolbar::Always,
                editor.inline.commands.clone(),
                editor.inline_warning(cx),
                editor.markdown_style(),
            )
        };
        // The menu closes once the keyboard leaves it.
        if self.menu_open && !self.focus.contains_focused(window, cx) && self.focus_seen {
            self.menu_open = false;
            self.active = None;
        }
        self.focus_seen = self.menu_open && self.focus.contains_focused(window, cx);
        let mac = cfg!(target_os = "macos");
        let colors = &self.theme.colors;
        // Monaco enables previous/next with more than one suggestion.
        let several = count.is_some_and(|(_, count)| count > 1);
        let mut bar = div().flex().items_center().gap(px(2.));
        bar = bar.child(
            div()
                .id("inline-previous")
                .tooltip(crate::tooltip::tooltip(
                    if mac {
                        "Previous (⌥[)"
                    } else {
                        "Previous (Alt+[)"
                    },
                    colors,
                ))
                .child(self.button(
                    "inline-previous-button",
                    // `inline-suggestion-hints-previous` / `-next`:
                    // `chevron-left` and `chevron-right`.
                    codicon(Codicon::ChevronLeft),
                    None,
                    several,
                    Arc::new(|editor, cx| editor.show_next_inline_suggestion(false, cx)),
                    cx,
                )),
        );
        bar =
            bar.child(
                div().min_w(px(19.)).flex().justify_center().child(
                    count.map_or(String::new(), |(ix, count)| format!("{}/{count}", ix + 1)),
                ),
            );
        bar = bar.child(
            div()
                .id("inline-next")
                .tooltip(crate::tooltip::tooltip(
                    if mac { "Next (⌥])" } else { "Next (Alt+])" },
                    colors,
                ))
                .child(self.button(
                    "inline-next-button",
                    codicon(Codicon::ChevronRight),
                    None,
                    several,
                    Arc::new(|editor, cx| editor.show_next_inline_suggestion(true, cx)),
                    cx,
                )),
        );
        // Monaco's `MenuId.InlineSuggestionToolbar` primary group.
        bar = bar.child(self.button(
            "inline-accept",
            "Accept",
            Some("Tab"),
            true,
            Arc::new(|editor, cx| {
                editor.accept_inline_suggestion(cx);
            }),
            cx,
        ));
        bar = bar.child(self.button(
            "inline-accept-word",
            "Accept Word",
            Some(if mac { "⌘→" } else { "Ctrl+RightArrow" }),
            true,
            Arc::new(|editor, cx| {
                editor.accept_next_inline_word(cx);
            }),
            cx,
        ));
        // The secondary group, behind "⋯".
        bar = bar.child(
            div()
                .id("inline-more")
                .px(px(3.))
                .rounded(px(3.))
                .cursor_pointer()
                .hover(|style| style.bg(colors.scrollbar_slider_hover))
                .tooltip(crate::tooltip::tooltip("More Actions...", colors))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        if this.menu_open {
                            this.close_menu(window, cx);
                        } else {
                            this.set_menu_open(true, window, cx);
                        }
                    }),
                )
                // `toolbar-more`.
                .child(codicon(Codicon::ToolBarMore)),
        );
        let menu = self.menu_open.then(|| {
            let mut menu = div()
                .id("inline-menu")
                .track_focus(&self.focus)
                .key_context(CONTEXT_MENU_KEY_CONTEXT)
                .on_action(cx.listener(|this, _: &ContextMenuNext, _, cx| this.move_active(1, cx)))
                .on_action(
                    cx.listener(|this, _: &ContextMenuPrevious, _, cx| this.move_active(-1, cx)),
                )
                .on_action(cx.listener(|this, _: &ContextMenuFirst, _, cx| {
                    this.move_active(-(isize::MAX / 2), cx)
                }))
                .on_action(cx.listener(|this, _: &ContextMenuLast, _, cx| {
                    this.move_active(isize::MAX / 2, cx)
                }))
                .on_action(cx.listener(|this, _: &ContextMenuSelect, window, cx| {
                    this.run_active(window, cx)
                }))
                .on_action(
                    cx.listener(|this, _: &ContextMenuHide, window, cx| {
                        this.close_menu(window, cx)
                    }),
                )
                .occlude()
                .mt(px(2.))
                .py(px(4.))
                .min_w(px(180.))
                .flex()
                .flex_col()
                .bg(colors.widget_background)
                .border_1()
                .border_color(colors.widget_border)
                .shadow_md();
            let fixed = 2;
            for (ix, item) in self.menu_items(always, commands).into_iter().enumerate() {
                if ix == fixed {
                    menu = menu.child(div().my(px(4.)).h(px(1.)).bg(colors.widget_border));
                }
                menu = menu.child(self.menu_item(("inline-menu-item", ix), item, cx));
            }
            // Monaco's More Actions menu is a context menu outside the
            // widget: a popup above everything (the hover card around an
            // on-hover toolbar would clip it), taking the mouse.
            div().absolute().top_full().right(px(0.)).child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .snap_to_window()
                        .child(menu),
                )
                .with_priority(MENU_PRIORITY),
            )
        });
        div()
            .id("inline-toolbar")
            .relative()
            .p(px(4.))
            .text_color(colors.widget_foreground)
            .when(self.with_border, |this| {
                this.bg(colors.widget_background)
                    .border_1()
                    .border_color(colors.widget_border)
            })
            // Clicks stay out of the text behind it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // Monaco's `warningMessage`: above the toolbar, at most 400px
            // wide.
            .children(warning.map(|warning| {
                div()
                    .max_w(px(400.))
                    .m(px(4.))
                    .flex()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_color(colors.marker_color(crate::MarkerSeverity::Warning))
                            .child(codicon(warning.icon.unwrap_or(Codicon::Warning))),
                    )
                    .child(crate::markdown::render_documentation(
                        &warning.message,
                        &markdown_style,
                        &self.theme,
                        cx,
                    ))
            }))
            .child(bar)
            .children(menu)
            .into_any_element()
    }
}
