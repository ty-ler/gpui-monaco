//! Parameter hints (Monaco's `contrib/parameterHints`): typing a trigger
//! character (e.g. `(`) shows the signature with the active parameter; it
//! updates while typing and closes with Escape or when the provider returns
//! nothing. Up/Down (and the widget's arrows) go through overloads.
//!
//! Ports `parameterHints.ts`, `parameterHintsModel.ts` and
//! `parameterHintsWidget.ts` (MIT).

use gpui::{
    AppContext as _, Context, Entity, FontWeight, HighlightStyle, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Render, StatefulInteractiveElement as _, Styled,
    StyledText, Task, WeakEntity, Window, div, px,
};
use text::PointUtf16;

use crate::codicons::{Codicon, icon as codicon};
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_documentation};
use crate::providers::{
    ParameterLabel, SignatureHelp, SignatureHelpTrigger, signature_help_providers,
};
use crate::widgets::{ContentWidgetPreference, WidgetId};

#[derive(Default)]
pub(crate) struct SignatureHelpState {
    task: Option<Task<()>>,
    /// A request is in flight (Monaco's `Pending` state).
    pending: bool,
    widget: Option<(Entity<SignatureHelpWidget>, WidgetId)>,
}

/// The parameter hints card, shown by [`CodeEditor::trigger_signature_help`]
/// as a content widget.
pub struct SignatureHelpWidget {
    help: SignatureHelp,
    editor: WeakEntity<CodeEditor>,
    style: MarkdownStyle,
    theme: std::sync::Arc<crate::Theme>,
    /// Monaco `updateMaxHeight`: a quarter of the editor's height, at least
    /// 250px.
    max_height: gpui::Pixels,
    /// The room it has in the window (it overflows the editor).
    room: crate::widgets::OverflowRoomCell,
}

impl SignatureHelpWidget {
    fn active(&self) -> Option<(&crate::providers::SignatureInformation, usize)> {
        let signature = self.help.signatures.get(self.help.active_signature)?;
        Some((
            signature,
            signature
                .active_parameter
                .unwrap_or(self.help.active_parameter),
        ))
    }

    /// Monaco's overload counter, e.g. `02/12`.
    fn overloads(&self) -> String {
        let count = self.help.signatures.len();
        let width = count.to_string().len();
        format!("{:0width$}/{count}", self.help.active_signature + 1)
    }

    /// A previous/next arrow (Monaco's `parameter-hints-previous`/`-next`
    /// buttons).
    fn button(&self, next: bool) -> impl IntoElement {
        let editor = self.editor.clone();
        div()
            .id(if next {
                "next-signature"
            } else {
                "previous-signature"
            })
            .w(px(16.))
            .h(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |editor, cx| {
                        if next {
                            editor.show_next_parameter_hint(cx);
                        } else {
                            editor.show_previous_parameter_hint(cx);
                        }
                    });
                }
            })
            // `parameter-hints-next` / `-previous`: `chevron-down` and
            // `chevron-up`.
            .child(codicon(if next {
                Codicon::ChevronDown
            } else {
                Codicon::ChevronUp
            }))
    }
}

/// UTF-8 byte range of parameter `label` within `signature`.
fn parameter_range(signature: &str, label: &ParameterLabel) -> Option<std::ops::Range<usize>> {
    match label {
        ParameterLabel::Text(text) => {
            let start = signature.find(text.as_ref())?;
            Some(start..start + text.len())
        }
        ParameterLabel::Range(range) => {
            // UTF-16 offsets into the label.
            let mut units = 0;
            let mut start = None;
            let mut end = None;
            for (ix, ch) in signature.char_indices() {
                if units == range.start {
                    start = Some(ix);
                }
                if units == range.end {
                    end = Some(ix);
                }
                units += ch.len_utf16();
            }
            if units == range.end {
                end = Some(signature.len());
            }
            Some(start?..end?)
        }
    }
}

impl Render for SignatureHelpWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors.clone();
        let Some((signature, active)) = self.active() else {
            return div();
        };
        let mut highlights = Vec::new();
        if let Some(range) = signature
            .parameters
            .get(active)
            .and_then(|p| parameter_range(&signature.label, &p.label))
        {
            // Monaco's `.parameter.active`: bold, in
            // `editorHoverWidget.highlightForeground`.
            highlights.push((
                range,
                HighlightStyle {
                    color: Some(colors.hover_highlight_foreground),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                },
            ));
        }
        let multiple = self.help.signatures.len() > 1;
        let parameter_doc = signature
            .parameters
            .get(active)
            .and_then(|p| p.documentation.clone());
        let body = div()
            .id("parameter-hints-body")
            // Monaco's body is a scrollable element.
            .overflow_y_scroll()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(6.))
            .child(
                div()
                    .font_family(self.style.code_font.clone())
                    .child(StyledText::new(signature.label.clone()).with_highlights(highlights)),
            )
            .when_some(parameter_doc, |this, doc| {
                this.child(render_documentation(&doc, &self.style, &self.theme, cx))
            })
            .when_some(signature.documentation.clone(), |this, doc| {
                this.child(render_documentation(&doc, &self.style, &self.theme, cx))
            });
        div()
            // Monaco `.phwrapper { max-width: 440px }`, and `updateMaxHeight`
            // capped to the room in the window.
            .max_w(self.room.cap_width(px(440.), px(100.)))
            .max_h(self.room.cap_height(self.max_height, px(24.)))
            .flex()
            // `border-radius: var(--vscode-cornerRadius-large)` and
            // `box-shadow: var(--vscode-shadow-lg)` (`parameterHints.css`).
            .rounded(crate::widgets::card::CORNER_RADIUS)
            .shadow(crate::widgets::card::shadow_lg())
            .bg(colors.hover_background)
            .border_1()
            .border_color(colors.hover_border)
            .text_color(colors.hover_foreground)
            .text_size(self.style.font_size)
            // The editor's line height (Monaco sets `line-height` to it over
            // the font size), not GPUI's default.
            .line_height(gpui::relative(self.style.line_height))
            // Monaco's `.controls` column, shown with several signatures.
            .when(multiple, |this| {
                this.child(
                    div()
                        .flex_none()
                        .min_w(px(22.))
                        .px(px(2.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_between()
                        .border_r_1()
                        .border_color(colors.hover_border)
                        .child(self.button(false))
                        .child(
                            div()
                                .text_size(self.style.font_size * 0.85)
                                .child(self.overloads()),
                        )
                        .child(self.button(true)),
                )
            })
            .child(body)
    }
}

use gpui::prelude::FluentBuilder as _;

impl CodeEditor {
    /// Whether parameter hints are showing.
    pub fn is_signature_help_visible(&self) -> bool {
        self.signature_help.widget.is_some()
    }

    /// Whether the parameter hints show several signatures (Monaco's
    /// `parameterHintsMultipleSignatures`).
    pub(crate) fn has_multiple_signatures(&self, cx: &gpui::App) -> bool {
        self.signature_help
            .widget
            .as_ref()
            .is_some_and(|(widget, _)| widget.read(cx).help.signatures.len() > 1)
    }

    /// Closes parameter hints and cancels a pending request (Monaco
    /// `closeParameterHints`).
    pub fn hide_signature_help(&mut self, cx: &mut Context<Self>) {
        self.signature_help.task = None;
        self.signature_help.pending = false;
        if let Some((_, id)) = self.signature_help.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Called after `text` was typed: opens or updates parameter hints
    /// (Monaco `ParameterHintsModel.onDidType`, only with
    /// `parameter_hints.enabled`).
    pub(crate) fn signature_help_after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.options.parameter_hints.enabled {
            return;
        }
        let Some(ch) = text.chars().last() else {
            return;
        };
        let providers = signature_help_providers(cx, self.provider_target(cx));
        if providers.is_empty() {
            return;
        }
        let is_trigger = providers
            .iter()
            .any(|p| p.trigger_characters().contains(&ch));
        let is_retrigger = providers
            .iter()
            .any(|p| p.retrigger_characters().contains(&ch));
        // Other typing re-queries as a content change via the cursor change.
        if is_trigger || (is_retrigger && self.is_signature_help_visible()) {
            self.trigger_signature_help(SignatureHelpTrigger::TriggerCharacter(ch), cx);
        }
    }

    /// Requests parameter hints at the cursor (Cmd/Ctrl+Shift+Space or a
    /// trigger character).
    pub fn trigger_signature_help(
        &mut self,
        trigger: SignatureHelpTrigger,
        cx: &mut Context<Self>,
    ) {
        // Widget inputs are plain inputs in Monaco.
        if self.widget_input {
            return;
        }
        let position = self.cursors.primary().position;
        let model = self.model().read(cx);
        let context = model.provider_context();
        // Monaco `isRetrigger` and `activeSignatureHelp`.
        let help_context = crate::providers::SignatureHelpContext {
            trigger,
            is_retrigger: self.signature_help.pending || self.signature_help.widget.is_some(),
            active_signature_help: self
                .signature_help
                .widget
                .as_ref()
                .map(|(widget, _)| widget.read(cx).help.clone()),
        };
        let tasks: Vec<_> = signature_help_providers(cx, self.provider_target(cx))
            .into_iter()
            .map(|provider| {
                provider.provide_signature_help_with_context(&context, position, &help_context, cx)
            })
            .collect();
        self.signature_help.pending = true;
        self.signature_help.task = Some(cx.spawn(async move |this, cx| {
            let mut result = None;
            for task in tasks {
                if let Ok(Some(help)) = task.await
                    && !help.signatures.is_empty()
                {
                    result = Some(help);
                    break;
                }
            }
            this.update(cx, |this, cx| {
                this.signature_help.pending = false;
                match result {
                    Some(help) => this.show_signature_help(help, position, cx),
                    None => this.hide_signature_help(cx),
                }
            })
            .ok();
        }));
    }

    fn show_signature_help(
        &mut self,
        help: SignatureHelp,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) {
        if let Some((widget, id)) = &self.signature_help.widget {
            let id = *id;
            widget.update(cx, |widget, cx| {
                widget.help = help;
                cx.notify();
            });
            self.set_content_widget_position(id, position, cx);
            cx.notify();
            return;
        }
        let style = self.markdown_style();
        let theme = self.theme.clone();
        let editor = cx.entity().downgrade();
        let max_height = (self.layout_size().unwrap_or_default().height / 4.).max(px(250.));
        let widget = cx.new(|_| SignatureHelpWidget {
            help,
            editor,
            style,
            theme,
            max_height,
            room: Default::default(),
        });
        let (id, room) = self.add_overflow_content_widget(
            position,
            vec![
                ContentWidgetPreference::Above,
                ContentWidgetPreference::Below,
            ],
            widget.clone().into(),
            crate::widgets::layer::PARAMETER_HINTS,
            cx,
        );
        widget.update(cx, |widget, _| widget.room = room);
        self.signature_help.widget = Some((widget, id));
        cx.notify();
    }

    /// Shows the next signature (Monaco `showNextParameterHint`): past the
    /// last one it wraps around with `parameter_hints.cycle`, else closes.
    pub fn show_next_parameter_hint(&mut self, cx: &mut Context<Self>) {
        self.step_signature(true, cx);
    }

    /// Shows the previous signature (Monaco `showPrevParameterHint`):
    /// before the first one it wraps around with `parameter_hints.cycle`,
    /// else closes.
    pub fn show_previous_parameter_hint(&mut self, cx: &mut Context<Self>) {
        self.step_signature(false, cx);
    }

    /// Monaco `ParameterHintsModel.next`/`previous`.
    fn step_signature(&mut self, next: bool, cx: &mut Context<Self>) {
        let Some((widget, _)) = &self.signature_help.widget else {
            return;
        };
        let cycle = self.options.parameter_hints.cycle;
        let stepped = widget.update(cx, |widget, cx| {
            let length = widget.help.signatures.len();
            let active = widget.help.active_signature;
            let at_end = if next {
                active % length == length - 1
            } else {
                active == 0
            };
            if (length < 2 || at_end) && !cycle {
                return false;
            }
            widget.help.active_signature = match (next, at_end) {
                (true, true) => 0,
                (true, false) => active + 1,
                (false, true) => length - 1,
                (false, false) => active - 1,
            };
            cx.notify();
            true
        });
        if !stepped {
            self.hide_signature_help(cx);
        }
    }

    /// The signature shown, as (label, index, count), for tests.
    #[doc(hidden)]
    pub fn active_signature_for_tests(&self, cx: &gpui::App) -> Option<(String, usize, usize)> {
        let (widget, _) = self.signature_help.widget.as_ref()?;
        let help = &widget.read(cx).help;
        let signature = help.signatures.get(help.active_signature)?;
        Some((
            signature.label.to_string(),
            help.active_signature,
            help.signatures.len(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_ranges() {
        let label = "add(a: i32, b: i32)";
        assert_eq!(
            parameter_range(label, &ParameterLabel::Text("b: i32".into())),
            Some(12..18)
        );
        assert_eq!(
            parameter_range(label, &ParameterLabel::Range(4..10)),
            Some(4..10)
        );
        assert_eq!(
            parameter_range(label, &ParameterLabel::Range(12..19)),
            Some(12..19)
        );
        assert_eq!(
            parameter_range(label, &ParameterLabel::Text("c".into())),
            None
        );
    }
}
