//! Inlay hints: provider text shown inline, such as inferred types and
//! parameter names.
//!
//! Ports Monaco's `src/vs/editor/contrib/inlayHints/browser/`
//! (`inlayHintsController.ts`, `inlayHints.ts`) (MIT), without label part
//! navigation, tooltips, double-click edits or the cursor-line length
//! freezing.

use std::ops::Range;
use std::time::Duration;

use gpui::{Context, Modifiers, Task};
use text::PointUtf16;

use crate::api::DecorationsCollection;
use crate::decorations::{DecorationOptions, InjectedText};
use crate::editor::CodeEditor;
use crate::options::InlayHintsMode;
use crate::providers::{InlayHint, InlayHintKind, InlayHintsEpoch, inlay_hints_providers};
use crate::theme::TokenStyle;

/// Monaco's adaptive debounce starts here.
const SCROLL_DELAY: Duration = Duration::from_millis(25);
/// After edits.
const EDIT_DELAY: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(crate) struct InlayHintState {
    task: Option<Task<()>>,
    hints: Vec<InlayHint>,
    decorations: Option<DecorationsCollection>,
    /// Rows the last request covered.
    requested: Option<Range<u32>>,
    /// Ctrl+Alt is held.
    pressed: bool,
    epoch: u64,
}

/// Monaco's per-line `maximumLength`: hint text past it is cut with `…`
/// and later hints on the line are dropped.
fn limit_per_line(hints: &[InlayHint], maximum: usize) -> Vec<(InlayHint, String)> {
    let mut out = Vec::new();
    let mut row = None;
    let mut total = 0;
    for hint in hints {
        if row != Some(hint.position.row) {
            row = Some(hint.position.row);
            total = 0;
        }
        let len = hint.label.chars().count();
        if maximum == 0 || total + len <= maximum {
            total += len;
            out.push((hint.clone(), hint.label.to_string()));
        } else if total < maximum {
            let keep = maximum - total;
            let text: String = hint.label.chars().take(keep).collect();
            total = maximum + 1;
            out.push((hint.clone(), format!("{text}…")));
        }
    }
    out
}

impl CodeEditor {
    fn inlay_hints_active(&self, cx: &gpui::App) -> bool {
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        self.options.inlay_hints.enabled != InlayHintsMode::Off
            && !self.options.single_line
            && !inlay_hints_providers(cx, language.as_deref()).is_empty()
    }

    /// Requests hints for the viewport after `delay` (Monaco's
    /// `RunOnceScheduler`).
    pub(crate) fn schedule_inlay_hints(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if !self.inlay_hints_active(cx) {
            self.inlay.task = None;
            if !self.inlay.hints.is_empty() {
                self.inlay.hints.clear();
                self.render_inlay_hints(cx);
            }
            return;
        }
        self.inlay.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok(tasks) = this.update(cx, |this, cx| this.request_inlay_hints(cx)) else {
                return;
            };
            let mut hints = Vec::new();
            for task in tasks {
                match task.await {
                    Ok(found) => hints.extend(found),
                    Err(err) => log::warn!("inlay hints failed: {err:#}"),
                }
            }
            hints.sort_by_key(|hint| hint.position);
            this.update(cx, |this, cx| {
                this.inlay.task = None;
                this.inlay.hints = hints;
                this.render_inlay_hints(cx);
            })
            .ok();
        }));
    }

    /// Monaco `getVisibleRangesPlusViewportAboveBelow`.
    fn request_inlay_hints(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<Task<anyhow::Result<Vec<InlayHint>>>> {
        let last_row = self.snapshot.max_point().row;
        let visible = self
            .layout
            .as_ref()
            .and_then(|layout| Some(layout.lines.first()?.row..layout.lines.last()?.row + 1))
            .unwrap_or(0..100.min(last_row + 1));
        let height = visible.len() as u32;
        let rows = visible.start.saturating_sub(height)..(visible.end + height).min(last_row + 1);
        self.inlay.requested = Some(rows.clone());
        let range = PointUtf16::new(rows.start, 0)
            ..PointUtf16::new(
                rows.end.saturating_sub(1),
                self.snapshot
                    .rope()
                    .line_len_utf16(rows.end.saturating_sub(1)),
            );
        let model = self.model().read(cx);
        let context = model.provider_context();
        let language = model.language_id().map(|l| l.to_string());
        inlay_hints_providers(cx, language.as_deref())
            .into_iter()
            .map(|provider| provider.provide_inlay_hints(&context, range.clone(), cx))
            .collect()
    }

    /// The viewport shows `rows`: request hints if they weren't covered
    /// (Monaco requests on scroll).
    pub(crate) fn inlay_hints_viewport(&mut self, rows: Range<u32>, cx: &mut Context<Self>) {
        let epoch = cx.try_global::<InlayHintsEpoch>().map_or(0, |e| e.0);
        let covered = self
            .inlay
            .requested
            .as_ref()
            .is_some_and(|r| r.start <= rows.start && rows.end <= r.end);
        if epoch != self.inlay.epoch {
            self.inlay.epoch = epoch;
            self.schedule_inlay_hints(Duration::ZERO, cx);
        } else if !covered && self.inlay.task.is_none() && self.inlay_hints_active(cx) {
            self.schedule_inlay_hints(SCROLL_DELAY, cx);
        }
    }

    /// The text changed: hints move with it until the new ones arrive.
    pub(crate) fn inlay_hints_after_edit(&mut self, cx: &mut Context<Self>) {
        if self.inlay_hints_active(cx) {
            self.schedule_inlay_hints(EDIT_DELAY, cx);
        }
    }

    /// The model, its language or the options changed.
    pub(crate) fn reset_inlay_hints(&mut self, cx: &mut Context<Self>) {
        if let Some(mut decorations) = self.inlay.decorations.take() {
            decorations.clear(cx);
        }
        self.inlay.hints.clear();
        self.inlay.requested = None;
        self.schedule_inlay_hints(Duration::ZERO, cx);
    }

    /// Ctrl+Alt toggles hints in the "unless pressed" modes.
    pub(crate) fn inlay_hints_modifiers(&mut self, modifiers: &Modifiers, cx: &mut Context<Self>) {
        let pressed = modifiers.control && modifiers.alt && !modifiers.shift && !modifiers.platform;
        if pressed != self.inlay.pressed {
            self.inlay.pressed = pressed;
            if matches!(
                self.options.inlay_hints.enabled,
                InlayHintsMode::OnUnlessPressed | InlayHintsMode::OffUnlessPressed
            ) {
                self.render_inlay_hints(cx);
            }
        }
    }

    fn inlay_hints_visible(&self) -> bool {
        match self.options.inlay_hints.enabled {
            InlayHintsMode::On => true,
            InlayHintsMode::OnUnlessPressed => !self.inlay.pressed,
            InlayHintsMode::OffUnlessPressed => self.inlay.pressed,
            InlayHintsMode::Off => false,
        }
    }

    /// Shows the hints as injected text the editor owns.
    fn render_inlay_hints(&mut self, cx: &mut Context<Self>) {
        if let Some(mut decorations) = self.inlay.decorations.take() {
            decorations.clear(cx);
        }
        if !self.inlay_hints_visible() || self.inlay.hints.is_empty() {
            cx.notify();
            return;
        }
        let colors = &self.theme.colors;
        let padding = self.options.inlay_hints.padding;
        let decorations: Vec<_> =
            limit_per_line(&self.inlay.hints, self.options.inlay_hints.maximum_length)
                .into_iter()
                .map(|(hint, mut text)| {
                    let (foreground, background) = match hint.kind {
                        Some(InlayHintKind::Type) => (
                            colors.inlay_hint_type_foreground,
                            colors.inlay_hint_type_background,
                        ),
                        Some(InlayHintKind::Parameter) => (
                            colors.inlay_hint_parameter_foreground,
                            colors.inlay_hint_parameter_background,
                        ),
                        None => (colors.inlay_hint_foreground, colors.inlay_hint_background),
                    };
                    // Monaco pads with a third of the font size; a thin
                    // space is close.
                    if hint.padding_left || padding {
                        text.insert(0, '\u{2009}');
                    }
                    if hint.padding_right || padding {
                        text.push('\u{2009}');
                    }
                    (
                        hint.position..hint.position,
                        DecorationOptions {
                            before: Some(InjectedText {
                                content: text.into(),
                                style: TokenStyle {
                                    foreground: Some(foreground),
                                    ..Default::default()
                                },
                                background: Some(background),
                            }),
                            ..Default::default()
                        },
                    )
                })
                .collect();
        let model = self.model().clone();
        let mut collection = DecorationsCollection::for_owner(model, cx.entity_id());
        collection.set(decorations, cx);
        self.inlay.decorations = Some(collection);
        cx.notify();
    }

    /// The inlay hints shown, as (position, text).
    pub fn visible_inlay_hints(&self, cx: &gpui::App) -> Vec<(PointUtf16, String)> {
        let Some(decorations) = &self.inlay.decorations else {
            return Vec::new();
        };
        let model = self.model().read(cx);
        decorations
            .ids()
            .iter()
            .filter_map(|id| {
                let decoration = model.decoration(*id)?;
                let text = decoration.options.before.as_ref()?.content.to_string();
                let position = model
                    .buffer()
                    .rope()
                    .offset_to_point_utf16(decoration.range.start);
                Some((position, text))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(row: u32, column: u32, label: &str) -> InlayHint {
        InlayHint {
            position: PointUtf16::new(row, column),
            label: label.to_string().into(),
            kind: None,
            tooltip: None,
            padding_left: false,
            padding_right: false,
        }
    }

    #[test]
    fn long_lines_are_cut() {
        let hints = [
            hint(0, 1, "abcdef"),
            hint(0, 5, "ghijkl"),
            hint(0, 9, "x"),
            hint(1, 0, "abcdefgh"),
        ];
        let texts: Vec<String> = limit_per_line(&hints, 8)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(texts, ["abcdef", "gh…", "abcdefgh"]);
    }
}
