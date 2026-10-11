//! Inlay hints: provider text shown inline, such as inferred types and
//! parameter names.
//!
//! Ports Monaco's `src/vs/editor/contrib/inlayHints/browser/`
//! (`inlayHintsController.ts`, `inlayHintsHover.ts`, `inlayHints.ts`) (MIT):
//! label parts with their own hover, Cmd/Ctrl+click to a part's location or
//! command, double-click to apply a hint's text edits, and the hints' font.
//! Not ported: the label part context menu, hovers from a part's location,
//! and the cursor-line length freezing.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::Shared;
use gpui::{Context, Modifiers, Pixels, Point, Task, px};
use text::PointUtf16;

use crate::api::DecorationsCollection;
use crate::decorations::{DecorationId, DecorationOptions, InjectedText};
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::options::{InlayHintsMode, MultiCursorModifier};
use crate::providers::{
    Command, InlayHint, InlayHintKind, InlayHintsEpoch, InlayHintsProvider, MarkdownString,
    inlay_hints_providers,
};
use crate::theme::TokenStyle;

/// Monaco's adaptive debounce starts here.
const SCROLL_DELAY: Duration = Duration::from_millis(25);
/// After edits.
const EDIT_DELAY: Duration = Duration::from_millis(250);

/// A hint and the provider that gave it (Monaco `InlayHintItem`).
#[derive(Clone)]
struct InlayHintItem {
    hint: InlayHint,
    provider: Arc<dyn InlayHintsProvider>,
    resolved: bool,
    /// A resolve under way.
    pending: Option<Shared<Task<Option<InlayHint>>>>,
}

/// A rendered label part (Monaco `RenderedInlayHintLabelPart`): the
/// generation of the hints it's from, the hint's index and the part's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InlayHintPart {
    generation: u64,
    item: usize,
    part: usize,
}

/// A label part's hover (Monaco `InlayHintsHover`).
pub(crate) struct InlayHintHover {
    /// The hint's position.
    pub position: PointUtf16,
    /// The tooltips and hints on how to follow the part.
    pub contents: Vec<MarkdownString>,
    /// The part's command, with the keys that run it.
    pub command: Option<(Command, String)>,
}

#[derive(Default)]
pub(crate) struct InlayHintState {
    task: Option<Task<()>>,
    items: Vec<InlayHintItem>,
    /// Bumped whenever `items` is replaced.
    generation: u64,
    decorations: Option<DecorationsCollection>,
    /// The label part each rendered decoration shows.
    parts: HashMap<DecorationId, InlayHintPart>,
    /// The part under the mouse that has a location or command, and
    /// whether the link modifier is held (Monaco `_activeInlayHintPart`).
    active: Option<(InlayHintPart, bool)>,
    /// Rows the last request covered.
    requested: Option<Range<u32>>,
    /// Ctrl+Alt is held.
    pressed: bool,
    epoch: u64,
}

/// What's shown for each hint: its label parts (as part index and text),
/// after Monaco's per-line `maximumLength`. Text past it is cut with `…`
/// and later hints on the line are dropped.
fn limit_per_line(hints: &[InlayHint], maximum: usize) -> Vec<(usize, Vec<(usize, String)>)> {
    let mut out = Vec::new();
    let mut row = None;
    let mut total = 0;
    for (ix, hint) in hints.iter().enumerate() {
        if row != Some(hint.position.row) {
            row = Some(hint.position.row);
            total = 0;
        }
        if maximum != 0 && total > maximum {
            continue;
        }
        let mut parts = Vec::new();
        for (part_ix, part) in hint.label.parts().into_iter().enumerate() {
            let mut text = part.label.to_string();
            total += text.chars().count();
            let over = if maximum == 0 {
                0
            } else {
                total.saturating_sub(maximum)
            };
            if over > 0 {
                let keep = text.chars().count().saturating_sub(over);
                text = text.chars().take(keep).collect();
                text.push('…');
                parts.push((part_ix, text));
                break;
            }
            parts.push((part_ix, text));
        }
        out.push((ix, parts));
    }
    out
}

/// Monaco `fixSpace`: spaces show as no-break spaces.
fn fix_space(text: &str) -> String {
    text.replace(' ', "\u{a0}")
}

impl CodeEditor {
    fn inlay_hints_active(&self, cx: &gpui::App) -> bool {
        self.options.inlay_hints.enabled != InlayHintsMode::Off
            && !self.options.single_line
            && !inlay_hints_providers(cx, self.provider_target(cx)).is_empty()
    }

    /// Requests hints for the viewport after `delay` (Monaco's
    /// `RunOnceScheduler`).
    pub(crate) fn schedule_inlay_hints(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if !self.inlay_hints_active(cx) {
            self.inlay.task = None;
            if !self.inlay.items.is_empty() {
                self.set_inlay_hint_items(Vec::new(), cx);
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
            let mut items = Vec::new();
            for (provider, task) in tasks {
                match task.await {
                    Ok(found) => items.extend(found.into_iter().map(|hint| InlayHintItem {
                        hint,
                        provider: provider.clone(),
                        resolved: false,
                        pending: None,
                    })),
                    Err(err) => log::warn!("inlay hints failed: {err:#}"),
                }
            }
            items.sort_by_key(|item| item.hint.position);
            this.update(cx, |this, cx| {
                this.inlay.task = None;
                this.set_inlay_hint_items(items, cx);
            })
            .ok();
        }));
    }

    fn set_inlay_hint_items(&mut self, items: Vec<InlayHintItem>, cx: &mut Context<Self>) {
        self.inlay.items = items;
        self.inlay.generation += 1;
        self.inlay.active = None;
        self.render_inlay_hints(cx);
    }

    /// Monaco `getVisibleRangesPlusViewportAboveBelow`.
    #[allow(clippy::type_complexity)]
    fn request_inlay_hints(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<(
        Arc<dyn InlayHintsProvider>,
        Task<anyhow::Result<Vec<InlayHint>>>,
    )> {
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
        // Monaco `InlayHintsFragments.create` asks `ordered(model).reverse()`.
        inlay_hints_providers(cx, self.provider_target(cx))
            .into_iter()
            .rev()
            .map(|provider| {
                let task = provider.provide_inlay_hints(&context, range.clone(), cx);
                (provider, task)
            })
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
        self.inlay.parts.clear();
        self.inlay.items.clear();
        self.inlay.generation += 1;
        self.inlay.active = None;
        self.inlay.requested = None;
        self.schedule_inlay_hints(Duration::ZERO, cx);
    }

    /// Ctrl+Alt toggles hints in the "unless pressed" modes, and the link
    /// modifier turns a hovered part into a link.
    pub(crate) fn inlay_hints_modifiers(&mut self, modifiers: &Modifiers, cx: &mut Context<Self>) {
        if let Some((part, _)) = self.inlay.active {
            let link = self.is_definition_modifier(modifiers);
            self.set_active_inlay_part(Some((part, link)), cx);
        }
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

    /// Monaco `_getLayoutInfo`: the hints' font size, unless it's the
    /// editor's, and font family.
    fn inlay_hints_font(&self) -> (Option<Pixels>, Option<gpui::SharedString>) {
        let options = &self.options.inlay_hints;
        let editor_size = f32::from(self.options.font_size);
        let size = options.font_size;
        let size = (size >= 5. && size < editor_size).then_some(px(size));
        let family = options
            .font_family
            .clone()
            .filter(|family| !family.is_empty() && *family != self.options.font_family);
        (size, family)
    }

    /// Monaco `_fillInColors`: (foreground, background) for a hint kind.
    fn inlay_hint_colors(&self, kind: Option<InlayHintKind>) -> (gpui::Hsla, gpui::Hsla) {
        let colors = &self.theme.colors;
        match kind {
            Some(InlayHintKind::Type) => (
                colors.inlay_hint_type_foreground,
                colors.inlay_hint_type_background,
            ),
            Some(InlayHintKind::Parameter) => (
                colors.inlay_hint_parameter_foreground,
                colors.inlay_hint_parameter_background,
            ),
            None => (colors.inlay_hint_foreground, colors.inlay_hint_background),
        }
    }

    /// A label part's text style: Monaco underlines the part under the
    /// mouse when it has a location or command, in the link color while the
    /// link modifier is held.
    fn inlay_part_style(&self, part: InlayHintPart) -> TokenStyle {
        let kind = self.inlay_hint_item(part).and_then(|item| item.hint.kind);
        let mut style = TokenStyle {
            foreground: Some(self.inlay_hint_colors(kind).0),
            ..Default::default()
        };
        if let Some((active, modifier)) = self.inlay.active
            && active == part
        {
            style.underline = true;
            if modifier {
                style.foreground = Some(self.theme.colors.link_active_foreground);
            }
        }
        style
    }

    /// Makes `active` the active part, restyling the old and new one in
    /// place (keeping their decorations, which the mouse hit-tests).
    fn set_active_inlay_part(
        &mut self,
        active: Option<(InlayHintPart, bool)>,
        cx: &mut Context<Self>,
    ) {
        let previous = std::mem::replace(&mut self.inlay.active, active);
        if previous == active {
            return;
        }
        let touched = |part: &InlayHintPart| {
            [previous, active]
                .iter()
                .any(|a| a.is_some_and(|(a, _)| a == *part))
        };
        let restyle: Vec<(DecorationId, TokenStyle)> = self
            .inlay
            .parts
            .iter()
            .filter(|(_, part)| touched(part))
            .map(|(id, part)| (*id, self.inlay_part_style(*part)))
            .collect();
        self.model().update(cx, |model, cx| {
            for (id, style) in restyle {
                let Some(mut options) = model.decoration(id).map(|d| (*d.options).clone()) else {
                    continue;
                };
                if let Some(before) = &mut options.before {
                    before.style = style;
                }
                model.change_decoration_options(id, options, cx);
            }
        });
        cx.notify();
    }

    /// Shows the hints as injected text the editor owns: one decoration per
    /// label part, plus one per padding.
    fn render_inlay_hints(&mut self, cx: &mut Context<Self>) {
        if let Some(mut decorations) = self.inlay.decorations.take() {
            decorations.clear(cx);
        }
        self.inlay.parts.clear();
        if !self.inlay_hints_visible() || self.inlay.items.is_empty() {
            cx.notify();
            return;
        }
        let padding = self.options.inlay_hints.padding;
        let (font_size, font_family) = self.inlay_hints_font();
        let hints: Vec<InlayHint> = self.inlay.items.iter().map(|i| i.hint.clone()).collect();
        let mut decorations = Vec::new();
        // The part each decoration shows, by index into `decorations`.
        let mut parts = Vec::new();
        for (item, shown) in limit_per_line(&hints, self.options.inlay_hints.maximum_length) {
            let hint = &hints[item];
            let (_, background) = self.inlay_hint_colors(hint.kind);
            let position = hint.position..hint.position;
            // Monaco pads with a third of the font size; a thin space is
            // close.
            let whitespace = |decorations: &mut Vec<_>| {
                decorations.push((
                    position.clone(),
                    DecorationOptions {
                        before: Some(InjectedText {
                            content: "\u{2009}".into(),
                            font_size,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ));
            };
            if hint.padding_left {
                whitespace(&mut decorations);
            }
            let count = shown.len();
            for (ix, (part_ix, mut text)) in shown.into_iter().enumerate() {
                let part = InlayHintPart {
                    generation: self.inlay.generation,
                    item,
                    part: part_ix,
                };
                let style = self.inlay_part_style(part);
                // Monaco's `padding` option pads the label's background.
                if padding && ix == 0 {
                    text.insert(0, '\u{2009}');
                }
                if padding && ix + 1 == count {
                    text.push('\u{2009}');
                }
                parts.push((decorations.len(), part));
                decorations.push((
                    position.clone(),
                    DecorationOptions {
                        before: Some(InjectedText {
                            content: fix_space(&text).into(),
                            style,
                            background: Some(background),
                            swatch: None,
                            font_family: font_family.clone(),
                            font_size,
                        }),
                        ..Default::default()
                    },
                ));
            }
            if hint.padding_right {
                whitespace(&mut decorations);
            }
        }
        let model = self.model().clone();
        let mut collection = DecorationsCollection::for_owner(model, cx.entity_id());
        collection.set(decorations, cx);
        let ids = collection.ids();
        for (ix, part) in parts {
            if let Some(id) = ids.get(ix) {
                self.inlay.parts.insert(*id, part);
            }
        }
        self.inlay.decorations = Some(collection);
        cx.notify();
    }

    /// The label part at `point` (window coordinates), if the mouse is over
    /// one (Monaco `_getInlayHintLabelPart`).
    pub(crate) fn inlay_hint_part_at(&self, point: Point<Pixels>) -> Option<InlayHintPart> {
        if self.inlay.parts.is_empty() {
            return None;
        }
        let layout = self.layout.as_ref()?;
        if !layout.text_bounds.contains(&point) {
            return None;
        }
        let line = layout.lines.iter().find(|line| {
            let top = layout.line_top(line.view_line);
            top <= point.y && point.y < top + layout.line_height_of(line.view_line)
        })?;
        let x = point.x - layout.text_origin_x() - line.indent_x;
        line.injected
            .iter()
            .find(|injected| {
                line.shaped.x_for_index(injected.range.start) <= x
                    && x < line.shaped.x_for_index(injected.range.end)
            })
            .and_then(|injected| self.inlay.parts.get(&injected.decoration?).copied())
    }

    fn inlay_hint_item(&self, part: InlayHintPart) -> Option<&InlayHintItem> {
        (part.generation == self.inlay.generation)
            .then(|| self.inlay.items.get(part.item))
            .flatten()
    }

    /// Resolves `part`'s hint once (Monaco `InlayHintItem.resolve`), and
    /// returns it.
    fn resolve_inlay_hint(
        &mut self,
        part: InlayHintPart,
        cx: &mut Context<Self>,
    ) -> Task<Option<InlayHint>> {
        let Some(item) = self.inlay_hint_item(part).cloned() else {
            return Task::ready(None);
        };
        if item.resolved {
            return Task::ready(Some(item.hint));
        }
        // Monaco waits for a resolve that's under way.
        if let Some(pending) = item.pending {
            return cx.spawn(async move |_, _| pending.await);
        }
        let Some(task) = item.provider.resolve_inlay_hint(&item.hint, cx) else {
            return Task::ready(Some(item.hint));
        };
        let pending = cx
            .spawn(async move |this, cx| {
                let result = task.await;
                let resolved = result.is_ok();
                let mut hint = item.hint.clone();
                match result {
                    Ok(resolved) => {
                        hint.label = resolved.label;
                        if resolved.tooltip.is_some() {
                            hint.tooltip = resolved.tooltip;
                        }
                        if !resolved.text_edits.is_empty() {
                            hint.text_edits = resolved.text_edits;
                        }
                    }
                    Err(err) => log::warn!("resolving an inlay hint failed: {err:#}"),
                }
                this.update(cx, |this, cx| {
                    if this.inlay_hint_item(part).is_some() {
                        let item = &mut this.inlay.items[part.item];
                        let changed = item.hint.label != hint.label;
                        item.hint = hint.clone();
                        item.resolved = resolved;
                        item.pending = None;
                        if changed {
                            this.render_inlay_hints(cx);
                        }
                    }
                })
                .ok();
                Some(hint)
            })
            .shared();
        self.inlay.items[part.item].pending = Some(pending.clone());
        cx.spawn(async move |_, _| pending.await)
    }

    /// The mouse moved over `part` (or off every part), with or without
    /// the link modifier: underlines a part that has a location or command
    /// (Monaco's link gesture `onMouseMoveOrRelevantKeyDown`).
    pub(crate) fn inlay_hints_mouse_move(
        &mut self,
        part: Option<InlayHintPart>,
        modifier: bool,
        cx: &mut Context<Self>,
    ) {
        let active = part
            .filter(|part| {
                self.inlay_hint_item(*part)
                    .and_then(|item| item.hint.label.parts().get(part.part).cloned())
                    .is_some_and(|p| p.location.is_some() || p.command.is_some())
            })
            .map(|part| (part, modifier));
        self.set_active_inlay_part(active, cx);
        if let Some(part) = part {
            // Monaco resolves the item as soon as the mouse is over it.
            self.resolve_inlay_hint(part, cx).detach();
        }
    }

    /// Whether Cmd/Ctrl is held over a clickable part (for the pointer
    /// cursor).
    pub(crate) fn has_active_inlay_hint_link(&self) -> bool {
        self.inlay.active.is_some_and(|(_, modifier)| modifier)
    }

    /// Cmd/Ctrl+click on `part` (Monaco's link gesture `onExecute`): opens
    /// its location like go to definition, else runs its command. Returns
    /// whether it had either.
    pub(crate) fn inlay_hint_link_click(
        &mut self,
        part: InlayHintPart,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(label_part) = self
            .inlay_hint_item(part)
            .and_then(|item| item.hint.label.parts().get(part.part).cloned())
        else {
            return false;
        };
        if let Some(location) = label_part.location {
            self.set_active_inlay_part(None, cx);
            self.open_location(&location, true, cx);
            true
        } else if let Some(command) = label_part.command {
            self.run_inlay_hint_command(&command, cx);
            true
        } else {
            false
        }
    }

    /// Runs a label part's command: through the command registry, or
    /// [`CodeEditorEvent::ExecuteCommand`] for the host.
    pub(crate) fn run_inlay_hint_command(&mut self, command: &Command, cx: &mut Context<Self>) {
        if crate::commands::execute_command(cx, command).is_err() {
            cx.emit(CodeEditorEvent::ExecuteCommand(command.clone()));
        }
    }

    /// A double-click on `part` (Monaco `_installDblClickGesture`):
    /// resolves its hint and applies the hint's text edits, then asks for
    /// hints again.
    pub(crate) fn inlay_hint_double_click(&mut self, part: InlayHintPart, cx: &mut Context<Self>) {
        let resolve = self.resolve_inlay_hint(part, cx);
        cx.spawn(async move |this, cx| {
            let Some(hint) = resolve.await else {
                return;
            };
            if hint.text_edits.is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                this.execute_edits(hint.text_edits, None, cx);
                this.schedule_inlay_hints(Duration::ZERO, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Monaco `InlayHintsHover.computeAsync`: resolves `part`'s hint, then
    /// returns where it is, its hover contents and the part's command.
    pub(crate) fn inlay_hint_hover_contents(
        &mut self,
        part: InlayHintPart,
        cx: &mut Context<Self>,
    ) -> Task<Option<InlayHintHover>> {
        let resolve = self.resolve_inlay_hint(part, cx);
        let keys = match (
            self.options.multi_cursor_modifier,
            cfg!(target_os = "macos"),
        ) {
            (MultiCursorModifier::Alt, true) => "cmd + click",
            (MultiCursorModifier::Alt, false) => "ctrl + click",
            (MultiCursorModifier::CtrlCmd, true) => "option + click",
            (MultiCursorModifier::CtrlCmd, false) => "alt + click",
        };
        cx.spawn(async move |_, _| {
            let hint = resolve.await?;
            let label_part = hint.label.parts().get(part.part).cloned()?;
            let mut contents = Vec::new();
            // The hint's tooltip, then the part's (Monaco's ordinals 0, 1).
            contents.extend(hint.tooltip.clone());
            contents.extend(label_part.tooltip.clone());
            let mut command = None;
            if label_part.location.is_some() {
                contents.push(format!("Go to Definition ({keys})").into());
            } else if let Some(part_command) = label_part.command {
                command = Some((part_command, format!(" ({keys})")));
            }
            if !hint.text_edits.is_empty() {
                contents.push("Double-click to insert".into());
            }
            Some(InlayHintHover {
                position: hint.position,
                contents,
                command,
            })
        })
    }

    /// The inlay hints shown, as (position, text).
    pub fn visible_inlay_hints(&self, cx: &gpui::App) -> Vec<(PointUtf16, String)> {
        let Some(decorations) = &self.inlay.decorations else {
            return Vec::new();
        };
        let model = self.model().read(cx);
        let mut hints: Vec<(PointUtf16, String)> = Vec::new();
        let mut last_item = None;
        for id in decorations.ids() {
            let Some(part) = self.inlay.parts.get(id) else {
                continue;
            };
            let Some(decoration) = model.decoration(*id) else {
                continue;
            };
            let Some(before) = decoration.options.before.as_ref() else {
                continue;
            };
            let text = before.content.replace('\u{a0}', " ");
            match hints.last_mut() {
                // The parts of one hint show as one.
                Some((_, label)) if last_item == Some(part.item) => label.push_str(&text),
                _ => {
                    let position = model
                        .buffer()
                        .rope()
                        .offset_to_point_utf16(decoration.range.start);
                    hints.push((position, text));
                }
            }
            last_item = Some(part.item);
        }
        hints
    }

    /// The window point over the middle of the shown label part `text`, for
    /// tests.
    #[doc(hidden)]
    pub fn inlay_hint_part_point_for_tests(
        &self,
        text: &str,
        cx: &gpui::App,
    ) -> Option<Point<Pixels>> {
        let layout = self.layout.as_ref()?;
        let model = self.model().read(cx);
        let decorations = self.inlay.decorations.as_ref()?;
        let id = decorations.ids().iter().copied().find(|id| {
            self.inlay.parts.contains_key(id)
                && model
                    .decoration(*id)
                    .and_then(|d| d.options.before.clone())
                    .is_some_and(|before| before.content.replace('\u{a0}', " ") == text)
        })?;
        for line in &layout.lines {
            for injected in &line.injected {
                if injected.decoration == Some(id) {
                    let from = line.shaped.x_for_index(injected.range.start);
                    let to = line.shaped.x_for_index(injected.range.end);
                    let top = layout.line_top(line.view_line);
                    return Some(gpui::point(
                        layout.text_origin_x() + line.indent_x + (from + to) / 2.,
                        top + layout.line_height_of(line.view_line) / 2.,
                    ));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::InlayHintLabelPart;

    fn hint(row: u32, column: u32, label: &str) -> InlayHint {
        InlayHint {
            position: PointUtf16::new(row, column),
            label: label.into(),
            ..Default::default()
        }
    }

    fn texts(hints: &[InlayHint], maximum: usize) -> Vec<Vec<String>> {
        limit_per_line(hints, maximum)
            .into_iter()
            .map(|(_, parts)| parts.into_iter().map(|(_, text)| text).collect())
            .collect()
    }

    #[test]
    fn long_lines_are_cut() {
        let hints = [
            hint(0, 1, "abcdef"),
            hint(0, 5, "ghijkl"),
            hint(0, 9, "x"),
            hint(1, 0, "abcdefgh"),
        ];
        assert_eq!(
            texts(&hints, 8),
            [vec!["abcdef"], vec!["gh…"], vec!["abcdefgh"]]
        );
        // No limit.
        assert_eq!(texts(&hints, 0).len(), 4);
    }

    #[test]
    fn long_label_parts_are_cut() {
        // Monaco cuts the part that crosses the limit and drops the rest.
        let hints = [InlayHint {
            label: vec![
                InlayHintLabelPart {
                    label: "abc".into(),
                    ..Default::default()
                },
                InlayHintLabelPart {
                    label: "defg".into(),
                    ..Default::default()
                },
                InlayHintLabelPart {
                    label: "h".into(),
                    ..Default::default()
                },
            ]
            .into(),
            ..Default::default()
        }];
        assert_eq!(texts(&hints, 5), [vec!["abc", "de…"]]);
        assert_eq!(texts(&hints, 8), [vec!["abc", "defg", "h"]]);
    }
}
