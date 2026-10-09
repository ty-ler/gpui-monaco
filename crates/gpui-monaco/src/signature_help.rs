//! Parameter hints (Monaco's `contrib/parameterHints`): typing a trigger
//! character (e.g. `(`) shows the signature with the active parameter; it
//! updates while typing and closes with Escape or when the provider returns
//! nothing.

use gpui::{
    AppContext as _, Context, Entity, FontWeight, HighlightStyle, IntoElement, ParentElement,
    Render, Styled, StyledText, Task, UnderlineStyle, Window, div, px,
};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_markdown};
use crate::providers::{
    ParameterLabel, SignatureHelp, SignatureHelpTrigger, signature_help_providers,
};
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

#[derive(Default)]
pub(crate) struct SignatureHelpState {
    task: Option<Task<()>>,
    widget: Option<(Entity<SignatureHelpWidget>, WidgetId)>,
}

pub struct SignatureHelpWidget {
    help: SignatureHelp,
    style: MarkdownStyle,
    theme: std::sync::Arc<crate::Theme>,
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
        let registry = crate::model::language_registry(cx);
        let Some((signature, active)) = self.active() else {
            return div();
        };
        let mut highlights = Vec::new();
        if let Some(range) = signature
            .parameters
            .get(active)
            .and_then(|p| parameter_range(&signature.label, &p.label))
        {
            highlights.push((
                range,
                HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(colors.widget_foreground),
                        wavy: false,
                    }),
                    ..Default::default()
                },
            ));
        }
        let count = self.help.signatures.len();
        let parameter_doc = signature
            .parameters
            .get(active)
            .and_then(|p| p.documentation.clone());
        div()
            .max_w(px(500.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(6.))
            .bg(colors.widget_background)
            .border_1()
            .border_color(colors.widget_border)
            .text_color(colors.widget_foreground)
            .text_size(self.style.font_size)
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .when_some((count > 1).then_some(()), |this, _| {
                        this.child(format!("{}/{}", self.help.active_signature + 1, count))
                    })
                    .child(div().font_family(self.style.code_font.clone()).child(
                        StyledText::new(signature.label.clone()).with_highlights(highlights),
                    )),
            )
            .when_some(parameter_doc, |this, doc| {
                this.child(render_markdown(&doc.0, &self.style, &self.theme, &registry))
            })
            .when_some(signature.documentation.clone(), |this, doc| {
                this.child(render_markdown(&doc.0, &self.style, &self.theme, &registry))
            })
    }
}

use gpui::prelude::FluentBuilder as _;

impl CodeEditor {
    pub fn is_signature_help_visible(&self) -> bool {
        self.signature_help.widget.is_some()
    }

    pub fn hide_signature_help(&mut self, cx: &mut Context<Self>) {
        self.signature_help.task = None;
        if let Some((_, id)) = self.signature_help.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Called after `text` was typed: opens or updates parameter hints.
    pub(crate) fn signature_help_after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(ch) = text.chars().last() else {
            return;
        };
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let providers = signature_help_providers(cx, language.as_deref());
        if providers.is_empty() {
            return;
        }
        let is_trigger = providers
            .iter()
            .any(|p| p.trigger_characters().contains(&ch));
        let is_retrigger = providers
            .iter()
            .any(|p| p.retrigger_characters().contains(&ch));
        if is_trigger {
            self.trigger_signature_help(SignatureHelpTrigger::TriggerCharacter(ch), cx);
        } else if self.is_signature_help_visible() {
            let trigger = if is_retrigger {
                SignatureHelpTrigger::TriggerCharacter(ch)
            } else {
                SignatureHelpTrigger::ContentChange
            };
            self.trigger_signature_help(trigger, cx);
        }
    }

    /// Requests parameter hints at the cursor (Cmd/Ctrl+Shift+Space or a
    /// trigger character).
    pub fn trigger_signature_help(
        &mut self,
        trigger: SignatureHelpTrigger,
        cx: &mut Context<Self>,
    ) {
        let position = self.cursors.primary().position;
        let model = self.model().read(cx);
        let context = model.provider_context();
        let language = model.language_id().map(|l| l.to_string());
        let tasks: Vec<_> = signature_help_providers(cx, language.as_deref())
            .into_iter()
            .map(|provider| {
                provider.provide_signature_help(&context, position, trigger.clone(), cx)
            })
            .collect();
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
            this.update(cx, |this, cx| match result {
                Some(help) => this.show_signature_help(help, position, cx),
                None => this.hide_signature_help(cx),
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
            return;
        }
        let style = self.markdown_style();
        let theme = self.theme.clone();
        let widget = cx.new(|_| SignatureHelpWidget { help, style, theme });
        let id = self.add_content_widget(
            ContentWidget {
                position,
                preference: vec![
                    ContentWidgetPreference::Above,
                    ContentWidgetPreference::Below,
                ],
                view: widget.clone().into(),
            },
            cx,
        );
        self.signature_help.widget = Some((widget, id));
    }

    /// Cycles overloads (Up/Down while hints with several signatures show).
    /// Returns whether it handled the key.
    pub(crate) fn cycle_signature(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some((widget, _)) = &self.signature_help.widget else {
            return false;
        };
        widget.update(cx, |widget, cx| {
            let count = widget.help.signatures.len();
            if count < 2 {
                return false;
            }
            let next = (widget.help.active_signature as isize + delta).rem_euclid(count as isize);
            widget.help.active_signature = next as usize;
            cx.notify();
            true
        })
    }
}
