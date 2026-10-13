//! Parameter hints (Monaco's `contrib/parameterHints`): typing a trigger
//! character (e.g. `(`) shows the signature with the active parameter; it
//! updates while typing and closes with Escape or when the provider returns
//! nothing. Up/Down (and the widget's arrows) go through overloads.
//!
//! Ports `parameterHints.ts`, `parameterHintsModel.ts` and
//! `parameterHintsWidget.ts` (MIT).

use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, FontWeight, HighlightStyle, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Render, StatefulInteractiveElement as _, Styled,
    StyledText, Task, WeakEntity, Window, div, px,
};

use crate::codicons::{Codicon, icon as codicon};
use crate::editor::CodeEditor;
use crate::markdown::{MarkdownStyle, render_documentation};
use crate::providers::{
    ParameterLabel, SignatureHelp, SignatureHelpTrigger, SignatureInformation,
    signature_help_providers,
};
use crate::widgets::{ContentWidgetPreference, WidgetId};

/// Monaco `ParameterHintsModel.DEFAULT_DELAY`: typing, cursor moves and
/// edits ask the providers this long after the last of them.
pub(crate) const PARAMETER_HINTS_DELAY: Duration = Duration::from_millis(120);

#[derive(Default)]
pub(crate) struct SignatureHelpState {
    task: Option<Task<()>>,
    /// A request is in flight (Monaco's `Pending` state).
    pending: bool,
    /// Monaco's `throttledDelayer`: the request waiting out
    /// [`PARAMETER_HINTS_DELAY`].
    delayed: Option<Task<()>>,
    /// Monaco `_pendingTriggers`: what asked since the last request, merged
    /// when it runs (`mergeTriggerContexts`).
    pending_triggers: Vec<SignatureHelpTrigger>,
    widget: Option<(Entity<SignatureHelpWidget>, WidgetId)>,
    /// Monaco `announcedLabel`: what screen readers last read, so the same
    /// parameter isn't read again on every keystroke.
    announced: Option<String>,
}

/// Monaco `mergeTriggerContexts`: an explicit request wins, a content
/// change keeps what asked before, a trigger character replaces it.
fn merge_triggers(triggers: Vec<SignatureHelpTrigger>) -> Option<SignatureHelpTrigger> {
    triggers
        .into_iter()
        .reduce(|previous, current| match current {
            SignatureHelpTrigger::ContentChange => previous,
            SignatureHelpTrigger::Invoke | SignatureHelpTrigger::TriggerCharacter(_) => current,
        })
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
    /// The body's scroll position, for its scrollbar.
    scroll: gpui::ScrollHandle,
}

/// The body's group, whose hover shows its scrollbar.
const BODY_SCROLL_GROUP: &str = "parameter-hints-scrollable";

/// Monaco's `.parameter-hints-widget` border: 2px in high contrast themes
/// (`parameterHints.css`), else 1px.
pub(crate) fn border_width(high_contrast: bool) -> gpui::Pixels {
    px(if high_contrast { 2. } else { 1. })
}

impl SignatureHelpWidget {
    fn active(&self) -> Option<(&SignatureInformation, usize)> {
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

/// Whether `ch` is a JavaScript regex word character (`\w`: ASCII letters,
/// numerals and `_`).
fn is_regex_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// UTF-8 byte range of parameter `label` within `signature` (Monaco
/// `ParameterHintsWidget.getParameterLabelOffsets`). A text label matches
/// on word boundaries, as Monaco's `(\W|^)label(?=\W|$)`: parameter `a` in
/// `add(a)` is the one in the parentheses, not the `a` of `add`. An empty
/// label, or one that isn't found, highlights nothing.
fn parameter_range(signature: &str, label: &ParameterLabel) -> Option<std::ops::Range<usize>> {
    match label {
        ParameterLabel::Text(text) => {
            if text.is_empty() {
                return None;
            }
            let mut from = 0;
            while let Some(found) = signature[from..].find(text.as_ref()) {
                let start = from + found;
                let end = start + text.len();
                let before = signature[..start].chars().next_back();
                let after = signature[end..].chars().next();
                if before.is_none_or(|ch| !is_regex_word(ch))
                    && after.is_none_or(|ch| !is_regex_word(ch))
                {
                    return Some(start..end);
                }
                // The next candidate starts a character later.
                from = start + signature[start..].chars().next().map_or(1, char::len_utf8);
            }
            None
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
            if units == range.start {
                start = Some(signature.len());
            }
            if units == range.end {
                end = Some(signature.len());
            }
            Some(start?..end?)
        }
    }
}

/// Monaco `ParameterHintsWidget.render`'s screen reader alert for the active
/// parameter: `"{label}, {parameter docs}, {signature docs}"` (read with
/// ", hint" after it).
fn hint_announcement(help: &SignatureHelp) -> Option<String> {
    let signature = help.signatures.get(help.active_signature)?;
    let active = signature.active_parameter.unwrap_or(help.active_parameter);
    let parameter = signature.parameters.get(active)?;
    let mut label = match &parameter.label {
        ParameterLabel::Text(text) => text.to_string(),
        ParameterLabel::Range(_) => parameter_range(&signature.label, &parameter.label)
            .map(|range| signature.label[range].to_string())
            .unwrap_or_default(),
    };
    if let Some(docs) = &parameter.documentation {
        label.push_str(", ");
        label.push_str(docs.as_str());
    }
    if let Some(docs) = &signature.documentation {
        label.push_str(", ");
        label.push_str(docs.as_str());
    }
    Some(label)
}

/// Monaco `ParameterHintsWidget.hasDocs`: the active parameter or the
/// signature has non-empty docs.
fn has_docs(signature: &SignatureInformation, active: usize) -> bool {
    let parameter_docs = signature
        .parameters
        .get(active)
        .and_then(|p| p.documentation.as_ref());
    parameter_docs
        .into_iter()
        .chain(signature.documentation.as_ref())
        .any(|docs| !docs.as_str().is_empty())
}

impl Render for SignatureHelpWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors.clone();
        let extent = crate::scrollbars::extent(&self.scroll);
        let scroll = self.scroll.clone();
        let view = cx.entity().downgrade();
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
        let has_docs = has_docs(signature, active);
        // Monaco's rules are drawn in the border color at half opacity.
        let rule = colors.hover_border.opacity(0.5);
        let docs = [parameter_doc, signature.documentation.clone()]
            .into_iter()
            .flatten()
            .filter(|doc| !doc.as_str().is_empty())
            .map(|doc| render_documentation(&doc, &self.style, &self.theme, cx));
        let body = div()
            .id("parameter-hints-body")
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .size_full()
            .flex()
            .flex_col()
            .child(
                // `.signature { padding: 4px 5px }` in the editor font;
                // `.signature.has-docs::after` rules it off from the docs.
                div()
                    .px(px(5.))
                    .py(px(4.))
                    .font_family(self.style.code_font.clone())
                    .when(has_docs, |this| this.border_b_1().border_color(rule))
                    .child(StyledText::new(signature.label.clone()).with_highlights(highlights)),
            )
            .when(has_docs, |this| {
                // `.docs { padding: 0 10px 0 5px }`, with `p { margin: 8px 0 }`.
                this.child(
                    div()
                        .pl(px(5.))
                        .pr(px(10.))
                        .py(px(8.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .children(docs),
                )
            });
        // Monaco's body is a `DomScrollableElement`: its 10px scrollbar
        // shows while the mouse is over it and the content doesn't fit.
        let scrollbars = crate::scrollbars::scrollbars(
            "parameter-hints",
            BODY_SCROLL_GROUP.into(),
            &self.scroll,
            px(10.),
            px(0.),
            &colors,
        );
        let body = div()
            .group(BODY_SCROLL_GROUP)
            .relative()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .flex()
            // `.multiple .body::before`: a rule between the controls and
            // the body.
            .when(multiple, |this| this.border_l_1().border_color(rule))
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        crate::scrollbars::follow_extent(&scroll, extent, view, window, cx)
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(body)
            .children(scrollbars);
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
            .border(border_width(self.theme.high_contrast))
            .border_color(colors.hover_border)
            .text_color(colors.hover_foreground)
            .text_size(self.style.font_size)
            // The editor's line height (Monaco sets `line-height` to it over
            // the font size), not GPUI's default.
            .line_height(gpui::relative(self.style.line_height))
            // Monaco's `.controls` column, shown with several signatures
            // (`.multiple { min-height: 3.3em }`), its buttons at the bottom.
            .when(multiple, |this| {
                this.min_h(self.style.font_size * 3.3).child(
                    div()
                        .flex_none()
                        .min_w(px(22.))
                        .px(px(2.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_end()
                        .child(self.button(false))
                        .child(
                            // `.overloads`: 12px tall, in the editor font.
                            div()
                                .h(px(12.))
                                .line_height(px(12.))
                                .font_family(self.style.code_font.clone())
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
        self.signature_help.delayed = None;
        self.signature_help.pending_triggers.clear();
        self.signature_help.announced = None;
        if let Some((_, id)) = self.signature_help.widget.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Whether parameter hints show, are being requested or wait to be
    /// (Monaco `ParameterHintsModel.isTriggered`): cursor moves and edits
    /// then ask again.
    pub(crate) fn is_signature_help_triggered(&self) -> bool {
        self.signature_help.widget.is_some()
            || self.signature_help.pending
            || self.signature_help.delayed.is_some()
    }

    /// Asks for parameter hints after [`PARAMETER_HINTS_DELAY`], merged with
    /// whatever else asks before then (Monaco `ParameterHintsModel.trigger`
    /// with its default delay): typing, cursor moves and edits.
    pub(crate) fn schedule_signature_help(
        &mut self,
        trigger: SignatureHelpTrigger,
        cx: &mut Context<Self>,
    ) {
        if self.widget_input || !self.has_signature_help_providers(cx) {
            return;
        }
        self.signature_help.pending_triggers.push(trigger);
        self.signature_help.delayed = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PARAMETER_HINTS_DELAY).await;
            this.update(cx, |this, cx| {
                this.signature_help.delayed = None;
                this.request_signature_help(cx);
            })
            .ok();
        }));
    }

    fn has_signature_help_providers(&self, cx: &gpui::App) -> bool {
        !signature_help_providers(cx, self.provider_target(cx)).is_empty()
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
        if is_trigger || (is_retrigger && self.is_signature_help_triggered()) {
            self.schedule_signature_help(SignatureHelpTrigger::TriggerCharacter(ch), cx);
        }
    }

    /// Requests parameter hints at the cursor now (Cmd/Ctrl+Shift+Space;
    /// Monaco `ParameterHintsController.trigger`, without the delay typing
    /// has), merged with any request still waiting.
    pub fn trigger_signature_help(
        &mut self,
        trigger: SignatureHelpTrigger,
        cx: &mut Context<Self>,
    ) {
        // Widget inputs are plain inputs in Monaco.
        if self.widget_input || !self.has_signature_help_providers(cx) {
            return;
        }
        self.signature_help.delayed = None;
        self.signature_help.pending_triggers.push(trigger);
        self.request_signature_help(cx);
    }

    /// Monaco `ParameterHintsModel.doTrigger`: asks the providers with the
    /// merged triggers.
    fn request_signature_help(&mut self, cx: &mut Context<Self>) {
        let triggers = std::mem::take(&mut self.signature_help.pending_triggers);
        let Some(trigger) = merge_triggers(triggers) else {
            return;
        };
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
        let providers = signature_help_providers(cx, self.provider_target(cx));
        self.signature_help.pending = true;
        self.signature_help.task = Some(cx.spawn(async move |this, cx| {
            // Monaco `provideSignatureHelp`: the providers are asked in
            // order, and the first result wins, even one without
            // signatures (which hides the hints); a failure asks the next.
            let mut result = None;
            for provider in providers {
                let task = cx.update(|cx| {
                    provider.provide_signature_help_with_context(
                        &context,
                        position,
                        &help_context,
                        cx,
                    )
                });
                if let Ok(Some(help)) = task.await {
                    result = Some(help);
                    break;
                }
            }
            this.update(cx, |this, cx| {
                this.signature_help.pending = false;
                match result.filter(|help| !help.signatures.is_empty()) {
                    Some(help) => this.show_signature_help(help, cx),
                    None => this.hide_signature_help(cx),
                }
            })
            .ok();
        }));
    }

    /// Shows `help` at the cursor (Monaco's widget `getPosition` is the
    /// editor's position, not the request's).
    fn show_signature_help(&mut self, help: SignatureHelp, cx: &mut Context<Self>) {
        let position = self.cursors.primary().position;
        self.announce_signature_help(&help, cx);
        // Monaco `updateMaxHeight`, on each layout change.
        let max_height = (self.layout_size().unwrap_or_default().height / 4.).max(px(250.));
        if let Some((widget, id)) = &self.signature_help.widget {
            let id = *id;
            widget.update(cx, |widget, cx| {
                widget.help = help;
                widget.max_height = max_height;
                cx.notify();
            });
            self.set_content_widget_position(id, position, cx);
            cx.notify();
            return;
        }
        let style = MarkdownStyle {
            // Monaco `.parameter-hints-widget .docs code`.
            code_corner_radius: px(3.),
            ..self.markdown_style(cx)
        };
        let theme = self.theme.clone();
        let editor = cx.entity().downgrade();
        let widget = cx.new(|_| SignatureHelpWidget {
            help,
            editor,
            style,
            theme,
            max_height,
            room: Default::default(),
            scroll: gpui::ScrollHandle::new(),
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

    /// Monaco `ParameterHintsWidget` lays itself out again on each
    /// selection change, at the cursor: the hints follow the caret while
    /// the next request is pending.
    pub(crate) fn signature_help_cursor_changed(&mut self, cx: &mut Context<Self>) {
        if let Some((_, id)) = &self.signature_help.widget {
            let id = *id;
            let position = self.cursors.primary().position;
            self.set_content_widget_position(id, position, cx);
        }
    }

    /// Has screen readers read the active parameter, its docs and the
    /// signature's when they change (Monaco's `aria.alert("{0}, hint")`).
    fn announce_signature_help(&mut self, help: &SignatureHelp, cx: &mut Context<Self>) {
        let Some(label) = hint_announcement(help) else {
            return;
        };
        if self.signature_help.announced.as_ref() != Some(&label) {
            self.announce(format!("{label}, hint"), cx);
            self.signature_help.announced = Some(label);
        }
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
            return;
        }
        if let Some((widget, _)) = &self.signature_help.widget {
            let help = widget.read(cx).help.clone();
            self.announce_signature_help(&help, cx);
        }
    }

    /// The byte range of the signature shown that's highlighted as the
    /// active parameter, for tests.
    #[doc(hidden)]
    pub fn active_parameter_range_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<std::ops::Range<usize>> {
        let (widget, _) = self.signature_help.widget.as_ref()?;
        let (signature, active) = widget.read(cx).active()?;
        parameter_range(&signature.label, &signature.parameters.get(active)?.label)
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

    /// `parameterHints.css`: `.hc-black`/`.hc-light` borders are 2px.
    #[test]
    fn high_contrast_border() {
        assert_eq!(border_width(false), px(1.));
        assert_eq!(border_width(true), px(2.));
    }

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

    /// Monaco `getParameterLabelOffsets`' `(\W|^)label(?=\W|$)`.
    #[test]
    fn parameter_labels_match_on_word_boundaries() {
        fn text<'a>(label: &'a str, param: &str) -> Option<(usize, &'a str)> {
            parameter_range(label, &ParameterLabel::Text(param.to_string().into()))
                .map(|range| (range.start, &label[range]))
        }
        // Not the `a` of `add` or the `cat` of `concat`.
        assert_eq!(text("add(a: int)", "a"), Some((4, "a")));
        assert_eq!(text("concat(cat)", "cat"), Some((7, "cat")));
        assert_eq!(text("f(ab, a)", "a"), Some((6, "a")));
        // At the start and end of the label.
        assert_eq!(text("a, b", "a"), Some((0, "a")));
        assert_eq!(text("f(x, b", "b"), Some((5, "b")));
        // `_` and ASCII numerals are word characters; other characters
        // aren't (JavaScript's `\w`).
        assert_eq!(text("f(_a, a1, a)", "a"), Some((10, "a")));
        assert_eq!(text("f(éa)", "a"), Some((4, "a")));
        // Labels that start or end with a non-word character.
        assert_eq!(text("f(...rest)", "...rest"), Some((2, "...rest")));
        // Empty or missing labels highlight nothing.
        assert_eq!(text("f(a)", ""), None);
        assert_eq!(text("f(abc)", "b"), None);
        // Ranges are UTF-16 offsets, up to the label's end.
        assert_eq!(
            parameter_range("f(é, 😀)", &ParameterLabel::Range(5..7)),
            Some(6..10)
        );
        assert_eq!(
            parameter_range("f()", &ParameterLabel::Range(3..3)),
            Some(3..3)
        );
    }

    /// Monaco `mergeTriggerContexts`.
    #[test]
    fn triggers_merge() {
        use SignatureHelpTrigger::*;
        assert_eq!(merge_triggers(Vec::new()), None);
        assert_eq!(
            merge_triggers(vec![TriggerCharacter('('), ContentChange, ContentChange]),
            Some(TriggerCharacter('('))
        );
        assert_eq!(
            merge_triggers(vec![ContentChange, TriggerCharacter(',')]),
            Some(TriggerCharacter(','))
        );
        assert_eq!(
            merge_triggers(vec![TriggerCharacter('('), Invoke, ContentChange]),
            Some(Invoke)
        );
        assert_eq!(merge_triggers(vec![ContentChange]), Some(ContentChange));
    }

    #[test]
    fn announcements_and_docs() {
        use crate::providers::{Documentation, ParameterInformation};
        let parameter = |label: ParameterLabel, docs: Option<&str>| ParameterInformation {
            label,
            documentation: docs.map(|d| Documentation::PlainText(d.to_string().into())),
        };
        let mut help = SignatureHelp {
            signatures: vec![SignatureInformation {
                label: "f(a, b)".into(),
                documentation: Some(Documentation::PlainText("Does f.".into())),
                parameters: vec![
                    parameter(ParameterLabel::Text("a".into()), Some("The a.")),
                    parameter(ParameterLabel::Range(5..6), None),
                ],
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: 0,
        };
        assert_eq!(
            hint_announcement(&help).as_deref(),
            Some("a, The a., Does f.")
        );
        assert!(has_docs(&help.signatures[0], 0));
        help.active_parameter = 1;
        assert_eq!(hint_announcement(&help).as_deref(), Some("b, Does f."));
        help.signatures[0].documentation = None;
        assert!(!has_docs(&help.signatures[0], 1));
        // No parameter there: nothing to read.
        help.active_parameter = 2;
        assert_eq!(hint_announcement(&help), None);
    }
}
