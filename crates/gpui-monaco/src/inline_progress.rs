//! Inline progress (Monaco `contrib/inlineProgress/browser/inlineProgress.ts`
//! and `inlineProgressWidget.css`, MIT): while a slow operation runs, a
//! spinner shows at a position after a delay, in a 1em gap a decoration
//! makes there. Clicking it (an `x` while hovered) cancels the operation.
//! Paste and drop use it for their providers (Monaco's
//! `CopyPasteController` and `DropIntoEditorController`).
//!
//! The codicon spinner (`loading` with `codicon-modifier-spin`) is drawn as
//! a rotating arc, as GPUI can't rotate text.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AppContext as _, Context, Hsla, InteractiveElement, IntoElement,
    ParentElement, PathBuilder, Pixels, Render, SharedString, StatefulInteractiveElement, Styled,
    Task, WeakEntity, Window, canvas, div, point, px,
};
use text::PointUtf16;

use crate::codicons::{Codicon, icon as codicon};
use crate::cursor::Selection;
use crate::decorations::{DecorationId, DecorationOptions, InjectedText, InjectedTextBox};
use crate::editor::CodeEditor;
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

/// Monaco `InlineProgressManager._showDelay`: the spinner shows after
/// 500ms.
pub(crate) const SHOW_DELAY: Duration = Duration::from_millis(500);

/// `codicon-modifier-spin`: a turn every 1.5s, in 30 steps.
const SPIN_DURATION: Duration = Duration::from_millis(1500);
const SPIN_STEPS: f32 = 30.;

/// Which of the editor's progress managers (Monaco creates one per
/// controller: `pasteIntoEditor` and `dropIntoEditor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProgressKind {
    Paste,
    Drop,
}

/// What clicking the spinner does (Monaco's `InlineProgressDelegate`), and
/// what the operation is cancelled with when the editor's state changes
/// (Monaco's `EditorStateCancellationTokenSource`).
#[derive(Clone)]
pub(crate) enum ProgressCancel {
    /// Cancels the drop providers.
    Drop,
    /// Cancels the paste providers and does the basic paste of this item,
    /// unless the text or selections changed meanwhile.
    Paste(gpui::ClipboardItem),
    /// Cancels resolving the paste edit about to be applied.
    ResolvePaste,
}

/// One operation's progress (Monaco `showWhile`'s state).
struct Operation {
    id: u64,
    cancel: ProgressCancel,
    /// The text version and selections when it started: changing the text
    /// (and, for paste, the selections) cancels it.
    version: text::Version,
    selections: Option<Vec<Selection>>,
    /// Shows the spinner after the delay.
    show: Option<Task<()>>,
    /// The gap's decoration and the spinner, once shown.
    shown: Option<(DecorationId, WidgetId)>,
}

/// Monaco `InlineProgressManager`: at most one operation's progress.
#[derive(Default)]
pub(crate) struct InlineProgressManager {
    /// Monaco `_operationIdPool`.
    next_id: u64,
    /// Monaco `_currentOperation`, with its progress.
    current: Option<Operation>,
}

/// The spinner (Monaco `InlineProgressWidget`).
struct InlineProgressWidget {
    editor: WeakEntity<CodeEditor>,
    kind: ProgressKind,
    title: SharedString,
    line_height: Pixels,
    font_size: Pixels,
    colors: crate::theme::EditorColors,
    hovered: bool,
}

/// A codicon `loading` spinner turned by `turn` (0 to 1), `size` square.
fn spinner(turn: f32, size: Pixels, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let center = bounds.center();
            // The glyph's ring: most of the box, with a quarter left open.
            let stroke = (size * 0.12).max(px(1.));
            let radius = size * 0.5 - stroke;
            let start = turn * std::f32::consts::TAU;
            let end = start + std::f32::consts::TAU * 0.75;
            let at = |angle: f32| {
                point(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                )
            };
            let mut path = PathBuilder::stroke(stroke);
            path.move_to(at(start));
            path.arc_to(point(radius, radius), px(0.), true, true, at(end));
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(size)
    .flex_none()
}

impl Render for InlineProgressWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.clone();
        let kind = self.kind;
        // `.icon`: 80% of the font size, 90% and `x` while hovered.
        let icon_size = self.font_size * 0.8;
        let color = self.colors.foreground;
        let icon = if self.hovered {
            codicon(Codicon::Close)
                .size(self.font_size * 0.9)
                .into_any_element()
        } else {
            div()
                .with_animation(
                    "inline-progress-spin",
                    Animation::new(SPIN_DURATION).repeat(),
                    move |this, delta| {
                        // `steps(30)`.
                        let turn = (delta * SPIN_STEPS).floor() / SPIN_STEPS;
                        this.child(spinner(turn, icon_size, color))
                    },
                )
                .into_any_element()
        };
        // `.inline-progress-widget`: a line high and 0.8 lines wide, the
        // icon centered; a button titled with what's running.
        div()
            .id("inline-progress")
            .h(self.line_height)
            .w((self.line_height * 0.8).ceil())
            .flex()
            .items_center()
            .justify_center()
            .text_color(color)
            .cursor_pointer()
            .tooltip(crate::tooltip::tooltip(self.title.clone(), &self.colors))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .on_click(move |_, _, cx| {
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |editor, cx| editor.cancel_inline_progress(kind, cx));
                }
            })
            .child(icon)
    }
}

impl CodeEditor {
    fn inline_progress(&mut self, kind: ProgressKind) -> &mut InlineProgressManager {
        match kind {
            ProgressKind::Paste => &mut self.drop_paste.paste_progress,
            ProgressKind::Drop => &mut self.drop_paste.drop_progress,
        }
    }

    /// Monaco `InlineProgressManager.showWhile`: starts operation progress
    /// at `position`, replacing any; the spinner shows after `delay`.
    /// Returns the operation, for [`Self::end_inline_progress`].
    pub(crate) fn start_inline_progress(
        &mut self,
        kind: ProgressKind,
        position: PointUtf16,
        title: impl Into<SharedString>,
        cancel: ProgressCancel,
        delay: Duration,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.clear_inline_progress(kind, cx);
        let manager = self.inline_progress(kind);
        let id = manager.next_id;
        manager.next_id += 1;
        let title = title.into();
        let show = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                this.show_inline_progress(kind, id, position, title, cx)
            })
            .ok();
        });
        // Monaco: paste is cancelled by text and selection changes, drop by
        // text changes.
        let selections = match cancel {
            ProgressCancel::Drop => None,
            _ => Some(self.selections()),
        };
        let version = self.snapshot.version();
        self.inline_progress(kind).current = Some(Operation {
            id,
            cancel,
            version,
            selections,
            show: Some(show),
            shown: None,
        });
        id
    }

    /// The delay passed: the gap and the spinner show.
    fn show_inline_progress(
        &mut self,
        kind: ProgressKind,
        id: u64,
        position: PointUtf16,
        title: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self
            .inline_progress(kind)
            .current
            .as_ref()
            .is_none_or(|operation| operation.id != id || operation.shown.is_some())
        {
            return;
        }
        // `inline-editor-progress-decoration`: a 1em gap after the position
        // (`widthInEm: 1`), even on an empty range.
        let font_size = self.options.font_size;
        let space = self
            .layout
            .as_ref()
            .map_or(font_size * 0.6, |layout| layout.metrics.space_width);
        let rope = self.model().read(cx).buffer().rope().clone();
        let offset = rope.point_utf16_to_offset(rope.clip_point_utf16(position, text::Bias::Left));
        let owner = Some(cx.entity_id());
        let decoration = self.model().update(cx, |model, cx| {
            model.delta_decorations(
                owner,
                &[],
                vec![(
                    offset..offset,
                    DecorationOptions {
                        after: Some(InjectedText {
                            content: "\u{a0}".into(),
                            text_box: Some(InjectedTextBox {
                                margin: (px(0.), (font_size - space).max(px(0.))),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        show_if_collapsed: true,
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        ..Default::default()
                    },
                )],
                cx,
            )
        });
        let Some(decoration) = decoration.first().copied() else {
            return;
        };
        let editor = cx.entity().downgrade();
        let view = cx.new(|_| InlineProgressWidget {
            editor,
            kind,
            title,
            line_height: self.line_height(),
            font_size,
            colors: self.theme.colors.clone(),
            hovered: false,
        });
        let widget = self.add_content_widget(
            ContentWidget {
                suppress_mouse_down: true,
                ..ContentWidget::new(position, vec![ContentWidgetPreference::Exact], view.into())
            },
            cx,
        );
        if let Some(operation) = self.inline_progress(kind).current.as_mut() {
            operation.show = None;
            operation.shown = Some((decoration, widget));
        }
        cx.notify();
    }

    /// The operation finished (Monaco `showWhile`'s `finally`): its progress
    /// goes, unless another replaced it.
    pub(crate) fn end_inline_progress(
        &mut self,
        kind: ProgressKind,
        id: u64,
        cx: &mut Context<Self>,
    ) {
        if self
            .inline_progress(kind)
            .current
            .as_ref()
            .is_some_and(|operation| operation.id == id)
        {
            self.clear_inline_progress(kind, cx);
        }
    }

    /// Monaco `InlineProgressManager.clear`.
    pub(crate) fn clear_inline_progress(&mut self, kind: ProgressKind, cx: &mut Context<Self>) {
        let Some(operation) = self.inline_progress(kind).current.take() else {
            return;
        };
        if let Some((decoration, widget)) = operation.shown {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &[decoration], Vec::new(), cx)
            });
            self.remove_widget(widget, cx);
        }
    }

    /// Stops `operation`'s work.
    fn abort_inline_progress_operation(&mut self, cancel: &ProgressCancel) {
        match cancel {
            ProgressCancel::Drop | ProgressCancel::Paste(_) => self.drop_paste.task = None,
            ProgressCancel::ResolvePaste => self.drop_paste.resolve_task = None,
        }
    }

    /// The spinner was clicked (Monaco's delegate `cancel`): the operation
    /// stops; a paste then does the basic paste.
    pub(crate) fn cancel_inline_progress(&mut self, kind: ProgressKind, cx: &mut Context<Self>) {
        let Some(operation) = self.inline_progress(kind).current.as_ref() else {
            return;
        };
        let cancel = operation.cancel.clone();
        let unchanged = self.inline_progress_state_unchanged(kind);
        self.abort_inline_progress_operation(&cancel);
        self.clear_inline_progress(kind, cx);
        if let ProgressCancel::Paste(item) = cancel
            && unchanged
        {
            self.paste_item_for_providers(item, cx);
        }
        cx.notify();
    }

    /// Whether the text (and, for paste, the selections) are as when the
    /// operation started.
    fn inline_progress_state_unchanged(&mut self, kind: ProgressKind) -> bool {
        let version = self.snapshot.version();
        let selections = self.selections();
        self.inline_progress(kind)
            .current
            .as_ref()
            .is_none_or(|operation| {
                operation.version == version
                    && operation
                        .selections
                        .as_ref()
                        .is_none_or(|start| *start == selections)
            })
    }

    /// Monaco's `EditorStateCancellationTokenSource`s: an operation whose
    /// text (or, for paste, selections) changed is cancelled, its progress
    /// with it. Called when the cursors change and before painting.
    pub(crate) fn check_inline_progress(&mut self, cx: &mut Context<Self>) {
        for kind in [ProgressKind::Paste, ProgressKind::Drop] {
            if self.inline_progress(kind).current.is_none()
                || self.inline_progress_state_unchanged(kind)
            {
                continue;
            }
            if let Some(operation) = self.inline_progress(kind).current.as_ref() {
                let cancel = operation.cancel.clone();
                self.abort_inline_progress_operation(&cancel);
            }
            self.clear_inline_progress(kind, cx);
        }
    }

    /// Whether the spinner of `kind` shows, and its title, for tests.
    #[doc(hidden)]
    pub fn inline_progress_for_tests(&mut self, paste: bool) -> Option<bool> {
        let kind = if paste {
            ProgressKind::Paste
        } else {
            ProgressKind::Drop
        };
        self.inline_progress(kind)
            .current
            .as_ref()
            .map(|operation| operation.shown.is_some())
    }

    /// Clicks the spinner of `kind`, for tests.
    #[doc(hidden)]
    pub fn click_inline_progress_for_tests(&mut self, paste: bool, cx: &mut Context<Self>) {
        let kind = if paste {
            ProgressKind::Paste
        } else {
            ProgressKind::Drop
        };
        self.cancel_inline_progress(kind, cx);
    }
}
