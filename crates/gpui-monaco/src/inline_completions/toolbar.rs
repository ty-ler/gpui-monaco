//! The inline suggestion toolbar (Monaco `InlineSuggestionHintsContentWidget`
//! and `InlineCompletionsHoverParticipant`): previous/next with the "1/3"
//! count, Accept and Accept Word with their keybindings, and "⋯", whose
//! menu (Accept Line, Always Show Toolbar and the provider's commands) is
//! the editor's context menu. It shows above the ghost text with
//! `show_toolbar: Always`, as a hover card of its own, or in the hover
//! over ghost text with `OnHover`. A warning on the shown completion shows
//! above it (Monaco's `warningMessage`).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Bounds, Context, Entity, InteractiveElement, IntoElement, KeyBinding, MouseButton,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, WeakEntity,
    Window, div, px,
};

use crate::codicons::{Codicon, icon as codicon};
use crate::editor::CodeEditor;
use crate::keybinding_label::{KeyLabel, KeycapStyle, command_binding, keycaps_styled};
use crate::theme::Theme;
use crate::widgets::card;

/// What a toolbar button does to the editor.
type ToolbarAction = Arc<dyn Fn(&mut CodeEditor, &mut Context<CodeEditor>)>;

/// The inline suggestion toolbar.
pub struct InlineToolbar {
    editor: WeakEntity<CodeEditor>,
    theme: Arc<Theme>,
    /// Drawn as its own hover card (Monaco `withBorder`), not inside a
    /// hover.
    with_border: bool,
    /// Where the "⋯" button was painted, in the window: its menu shows
    /// below it.
    more_bounds: Rc<Cell<Bounds<Pixels>>>,
}

/// The binding Monaco's `lookupKeybinding` shows for an inline suggestion
/// command: `action`'s ([`command_binding`]), else the key of `via`, the
/// editor action that runs the command for that key (Tab runs
/// `inlineSuggest.commit`, Escape `inlineSuggest.hide`).
pub(crate) fn inline_command_binding(
    action: &dyn gpui::Action,
    via: Option<&dyn gpui::Action>,
    focus: &gpui::FocusHandle,
    window: &Window,
) -> Option<KeyBinding> {
    command_binding(action, None, focus, window)
        .or_else(|| via.and_then(|via| command_binding(via, None, focus, window)))
}

/// The toolbar's keys (Monaco `keybindingService.lookupKeybinding`): as
/// bound now, rebinding included.
struct ToolbarKeys {
    previous: Option<KeyBinding>,
    next: Option<KeyBinding>,
    accept: Option<KeyBinding>,
    accept_word: Option<KeyBinding>,
}

impl ToolbarKeys {
    fn lookup(focus: &gpui::FocusHandle, window: &Window) -> Self {
        use crate::actions::*;
        let binding = |action: &dyn gpui::Action, via: Option<&dyn gpui::Action>| {
            inline_command_binding(action, via, focus, window)
        };
        Self {
            previous: binding(&ShowPreviousInlineSuggestion, None),
            next: binding(&ShowNextInlineSuggestion, None),
            accept: binding(&AcceptInlineSuggestion, Some(&Tab)),
            accept_word: binding(&AcceptNextInlineWord, None),
        }
    }
}

impl CodeEditor {
    /// The toolbar's Previous and Next tooltips and its Accept and Accept
    /// Word keys, as it shows them, for tests.
    #[doc(hidden)]
    pub fn inline_toolbar_labels_for_tests(&self, window: &Window) -> Vec<String> {
        let keys = ToolbarKeys::lookup(&self.focus_handle, window);
        let label = |binding: Option<KeyBinding>| {
            binding
                .map(|binding| KeyLabel::for_binding(&binding).text.to_string())
                .unwrap_or_default()
        };
        vec![
            append_keybinding("Previous", keys.previous.as_ref()).to_string(),
            append_keybinding("Next", keys.next.as_ref()).to_string(),
            label(keys.accept),
            label(keys.accept_word),
        ]
    }
}

/// Monaco `appendKeybinding`: `label (keys)`, or `label` when unbound.
fn append_keybinding(label: &str, binding: Option<&KeyBinding>) -> SharedString {
    match binding.map(crate::keybinding_label::binding_label) {
        Some(keys) if !keys.is_empty() => format!("{label} ({keys})").into(),
        _ => label.to_string().into(),
    }
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
            more_bounds: Rc::default(),
        }
    }

    /// Where the "⋯" menu shows: below the button, at its left (Monaco's
    /// toolbar dropdown, a context menu anchored to it).
    pub(crate) fn menu_anchor(&self) -> gpui::Point<Pixels> {
        let bounds = self.more_bounds.get();
        gpui::point(bounds.left(), bounds.bottom())
    }

    fn open_menu(&self, window: &mut Window, cx: &mut gpui::App) {
        let anchor = self.menu_anchor();
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| {
                editor.show_inline_toolbar_menu(anchor, window, cx)
            });
        }
    }

    /// A toolbar button: hovered in `toolbar.hoverBackground` (Monaco's
    /// `.action-label:hover`), and with the keys that run it as keycaps
    /// (Monaco's `StatusBarViewItem`).
    fn button(
        &self,
        id: &'static str,
        label: impl IntoElement,
        keys: Option<KeyLabel>,
        enabled: bool,
        run: ToolbarAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.theme.colors.clone();
        let editor = self.editor.clone();
        // `.inlineSuggestionsHints .keybinding`: 4px after the label at 0.6
        // opacity, with `unthemedKeybindingLabelOptions` keycaps (8px keys,
        // `2px 3px` padding, borders in the text color).
        let foreground = self.text_color();
        let keycap_style = KeycapStyle {
            font_size: px(8.),
            line_height: px(10.),
            padding: (px(2.), px(3.)),
            margin: px(2.),
            background: None,
            foreground: None,
            border: Some(foreground),
            bottom_border: Some(foreground),
            shadow: None,
        };
        div()
            .id(id)
            .flex()
            .items_center()
            .px(px(3.))
            .rounded(px(3.))
            .border_1()
            .border_color(gpui::transparent_black())
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(move |style| colors.toolbar_hover(style))
            })
            .when(!enabled, |this| this.opacity(0.4))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    if !enabled {
                        return;
                    }
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| run(editor, cx));
                    }
                }),
            )
            .child(label)
            .children(keys.filter(|keys| !keys.chords.is_empty()).map(|keys| {
                div()
                    .flex()
                    .items_center()
                    .ml(px(4.))
                    .opacity(0.6)
                    .child(keycaps_styled(&keys, &keycap_style))
            }))
            .into_any_element()
    }

    /// The text color: the hover's, in its own card or inside the hover.
    fn text_color(&self) -> gpui::Hsla {
        self.theme.colors.hover_foreground
    }
}

impl Render for InlineToolbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div().into_any_element();
        };
        let (count, warning, markdown_style, focus) = {
            let editor = editor.read(cx);
            (
                editor.inline_suggestion_index(cx),
                editor.inline_warning(cx),
                editor.markdown_style(cx),
                editor.focus_handle.clone(),
            )
        };
        let ToolbarKeys {
            previous,
            next,
            accept,
            accept_word,
        } = ToolbarKeys::lookup(&focus, window);
        let colors = &self.theme.colors;
        // Monaco enables previous/next with more than one suggestion.
        let several = count.is_some_and(|(_, count)| count > 1);
        let mut bar = div().flex().items_center().gap(px(2.));
        bar = bar.child(
            div()
                .id("inline-previous")
                .tooltip(crate::tooltip::tooltip(
                    append_keybinding("Previous", previous.as_ref()),
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
                    append_keybinding("Next", next.as_ref()),
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
            accept.as_ref().map(KeyLabel::for_binding),
            true,
            Arc::new(|editor, cx| {
                editor.accept_inline_suggestion(cx);
            }),
            cx,
        ));
        bar = bar.child(self.button(
            "inline-accept-word",
            "Accept Word",
            accept_word.as_ref().map(KeyLabel::for_binding),
            true,
            Arc::new(|editor, cx| {
                editor.accept_next_inline_word(cx);
            }),
            cx,
        ));
        // The secondary group, behind "⋯" (a context menu below it).
        let more_bounds = self.more_bounds.clone();
        let hover_colors = colors.clone();
        bar = bar.child(
            div()
                .id("inline-more")
                .relative()
                .px(px(3.))
                .rounded(px(3.))
                .border_1()
                .border_color(gpui::transparent_black())
                .cursor_pointer()
                .hover(move |style| hover_colors.toolbar_hover(style))
                .tooltip(crate::tooltip::tooltip("More Actions...", colors))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_menu(window, cx);
                    }),
                )
                // `toolbar-more`.
                .child(codicon(Codicon::ToolBarMore))
                .child(
                    gpui::canvas(move |bounds, _, _| more_bounds.set(bounds), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                ),
        );
        div()
            .id("inline-toolbar")
            .relative()
            .p(px(4.))
            .text_color(self.text_color())
            // `withBorder` adds `monaco-hover`: the hover's colors, large
            // corners and `shadow-lg`.
            .when(self.with_border, |this| {
                this.bg(colors.hover_background)
                    .border_1()
                    .border_color(colors.hover_border)
                    .rounded(card::CORNER_RADIUS)
                    .shadow(card::shadow_lg())
            })
            // Clicks stay out of the text behind it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // Monaco's `warningMessage`: above the toolbar, at most 400px
            // wide, the message alone (`icon` is the ghost text's).
            .children(warning.map(|warning| {
                div()
                    .max_w(px(400.))
                    .m(px(4.))
                    .child(crate::markdown::render_documentation(
                        &warning.message,
                        &markdown_style,
                        &self.theme,
                        cx,
                    ))
            }))
            .child(bar)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_keybinding_adds_the_keys() {
        let binding = KeyBinding::new("alt-]", crate::actions::ShowNextInlineSuggestion, None);
        let label = crate::keybinding_label::binding_label(&binding);
        assert_eq!(
            append_keybinding("Next", Some(&binding)).to_string(),
            format!("Next ({label})")
        );
        assert_eq!(append_keybinding("Next", None).as_ref(), "Next");
    }
}
