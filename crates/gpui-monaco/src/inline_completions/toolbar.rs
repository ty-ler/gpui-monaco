//! The inline suggestion toolbar (Monaco `InlineSuggestionHintsContentWidget`
//! and `InlineCompletionsHoverParticipant`): previous/next with the "1/3"
//! count, Accept and Accept Word, and a menu with Accept Line, Always Show
//! Toolbar and the provider's commands. It shows above the ghost text with
//! `show_toolbar: Always`, or in the hover over ghost text with `OnHover`.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window, div, px,
};

use crate::codicons::{Codicon, icon as codicon};
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
        }
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
        label: SharedString,
        checked: bool,
        run: ToolbarAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = &self.theme.colors;
        let editor = self.editor.clone();
        div()
            .id(id)
            .flex()
            .gap(px(6.))
            .px(px(6.))
            .py(px(2.))
            .cursor_pointer()
            .hover(|style| style.bg(colors.scrollbar_slider_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.menu_open = false;
                    cx.notify();
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div().into_any_element();
        };
        let (count, always, commands) = {
            let editor = editor.read(cx);
            (
                editor.inline_suggestion_index(cx),
                editor.options().inline_suggest.show_toolbar == InlineSuggestShowToolbar::Always,
                editor.inline.commands.clone(),
            )
        };
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
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.menu_open = !this.menu_open;
                        cx.notify();
                    }),
                )
                // `toolbar-more`.
                .child(codicon(Codicon::ToolBarMore)),
        );
        let menu = self.menu_open.then(|| {
            let mut menu = div()
                .occlude()
                .mt(px(2.))
                .py(px(4.))
                .min_w(px(180.))
                .flex()
                .flex_col()
                .bg(colors.widget_background)
                .border_1()
                .border_color(colors.widget_border)
                .shadow_md()
                .child(self.menu_item(
                    ("inline-menu", 0),
                    "Accept Line".into(),
                    false,
                    Arc::new(|editor, cx| {
                        editor.accept_next_inline_line(cx);
                    }),
                    cx,
                ))
                .child(self.menu_item(
                    ("inline-menu", 1),
                    "Always Show Toolbar".into(),
                    always,
                    Arc::new(|editor, cx| editor.toggle_always_show_inline_toolbar(cx)),
                    cx,
                ));
            if !commands.is_empty() {
                menu = menu.child(div().my(px(4.)).h(px(1.)).bg(colors.widget_border));
            }
            for (ix, command) in commands.into_iter().enumerate() {
                let title = command.title.clone();
                menu = menu.child(self.menu_item(
                    ("inline-command", ix),
                    title,
                    false,
                    Arc::new(move |_, cx| {
                        if let Err(err) = crate::commands::execute_command(cx, &command) {
                            log::warn!("{err:#}");
                        }
                    }),
                    cx,
                ));
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
            .child(bar)
            .children(menu)
            .into_any_element()
    }
}
