//! Code lenses (Monaco's `contrib/codelens`): commands from providers shown
//! in a short view zone above lines, refreshed after edits and resolved
//! when they scroll into view; clicking one runs its command.
//!
//! Ports `codelensController.ts` and `codelensWidget.ts` (MIT). Monaco's
//! `CodeLensContentWidget` is drawn inside the lens's `CodeLensViewZone`,
//! reading the editor's font, theme, zoom and scroll position each time
//! it's drawn, as Monaco's lenses follow its `--vscode-editorCodeLens-*`
//! CSS variables and scroll with the text.

use std::sync::Arc;
use std::time::Duration;

use web_time::Instant;

use crate::codicons::{LabelPart, icon, parse_label_with_icons};
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::providers::{CodeLens, CodeLensProvider, Command, code_lens_providers};
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};
use gpui::{
    AnyElement, App, AppContext as _, Context, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled, Task,
    WeakEntity, Window, div, px,
};
use text::PointUtf16;

/// Monaco's debounce floor for requesting and resolving lenses.
const MIN_DELAY: Duration = Duration::from_millis(250);

/// Monaco's `.codelens-decoration.fadein` (`animation: fadein 0.1s linear`).
const FADE_IN: Duration = Duration::from_millis(100);

/// One lens and its provider.
#[derive(Clone)]
struct Item {
    lens: CodeLens,
    provider: Arc<dyn CodeLensProvider>,
    /// The provider's position, for ordering.
    rank: usize,
    /// Asked to resolve already.
    resolving: bool,
}

/// The lenses of one line and their zone (Monaco `CodeLensWidget`).
struct Group {
    row: u32,
    items: Vec<Item>,
    view: gpui::Entity<CodeLensLine>,
    zone: WidgetId,
}

#[derive(Default)]
pub(crate) struct CodeLensState {
    groups: Vec<Group>,
    /// The version the lenses were requested for.
    version: Option<text::Version>,
    /// The `refresh_code_lenses` count they were requested at.
    epoch: u64,
    pending: Option<text::Version>,
    task: Option<Task<()>>,
    samples: Vec<Duration>,
    /// The zones' height (Monaco `_getLayoutInfo().codeLensHeight`), to
    /// re-lay them out when the font or zoom changes.
    height: Pixels,
}

/// What one lens shows (Monaco `withCommands`' input): `None` when
/// resolving it failed, else its command, if any.
type Shown = Option<Option<Command>>;

/// The lenses above one line (Monaco `CodeLensContentWidget`): titles
/// separated by `|`, the ones with commands clickable.
pub struct CodeLensLine {
    editor: WeakEntity<CodeEditor>,
    /// The zone it's drawn in, which tracks its line.
    zone: Option<WidgetId>,
    /// `None` until a lens has a command: Monaco adds the content widget
    /// only then, so lenses waiting to resolve leave the zone empty.
    lenses: Option<Vec<Shown>>,
    /// When Monaco's `fadein` started (resolved lenses filled an empty
    /// zone).
    fade_start: Option<Instant>,
}

/// How a lens line is drawn, from the editor's current options, theme,
/// font and scroll position (Monaco's `--vscode-editorCodeLens-*` CSS
/// variables and the content widget's position).
struct LensStyle {
    font_family: SharedString,
    features: gpui::FontFeatures,
    font_size: Pixels,
    line_height: Pixels,
    color: Hsla,
    hover: Hsla,
    /// The x of the line's first non-whitespace character, relative to the
    /// zone (the text area's left edge), after horizontal scrolling.
    left: Pixels,
    tooltip: (Hsla, Hsla, Hsla),
}

/// Monaco `getLineFirstNonWhitespaceColumn` (0-based, `0` for a blank
/// line) and the visible column it starts at, with tabs expanded to
/// `tab_size` stops.
fn first_non_whitespace(line: &str, tab_size: u32) -> (u32, u32) {
    let tab_size = tab_size.max(1);
    let mut visible = 0;
    for (ix, ch) in line.char_indices() {
        match ch {
            ' ' => visible += 1,
            '\t' => visible += tab_size - visible % tab_size,
            _ => return (ix as u32, visible),
        }
    }
    (0, 0)
}

impl CodeLensLine {
    /// The text it shows (for tests): empty while waiting to resolve.
    fn text(&self) -> String {
        let Some(lenses) = &self.lenses else {
            return String::new();
        };
        if lenses.iter().all(Option::is_none) {
            return "no commands".into();
        }
        let mut text = String::new();
        for (ix, shown) in lenses.iter().enumerate() {
            let Some(Some(command)) = shown else {
                continue;
            };
            text.push_str(command.title.trim());
            if ix + 1 < lenses.len() {
                text.push_str(" | ");
            }
        }
        text
    }

    /// Monaco `withCommands`: `animate` fades the lenses in when they fill
    /// an empty zone.
    fn set_lenses(&mut self, lenses: Vec<Shown>, animate: bool) {
        let was_empty = self
            .lenses
            .as_ref()
            .is_none_or(|lenses| lenses.iter().all(Option::is_none));
        if was_empty && animate && lenses.iter().any(Option::is_some) {
            self.fade_start = Some(Instant::now());
        }
        self.lenses = Some(lenses);
    }
}

/// Monaco `renderLabelWithIcons`: the title's text, with its `$(icon)`s
/// as codicons the lens's size.
fn title_with_icons(title: &str, size: Pixels) -> Vec<AnyElement> {
    parse_label_with_icons(title)
        .into_iter()
        .map(|part| match part {
            LabelPart::Text(text) => {
                // Spaces around icons would collapse in a flex row.
                SharedString::from(text.replace(' ', "\u{a0}")).into_any_element()
            }
            LabelPart::Icon(codicon) => icon(codicon).size(size).into_any_element(),
        })
        .collect()
}

impl Render for CodeLensLine {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self
            .editor
            .upgrade()
            .and_then(|editor| editor.read(cx).code_lens_style(self.zone, cx));
        let Some(style) = style else {
            return div();
        };
        // Monaco `.codelens-decoration`, at the content widget's position.
        let mut row = div()
            .relative()
            .left(style.left)
            .h(style.line_height)
            .flex()
            .items_center()
            .whitespace_nowrap()
            .font_family(style.font_family.clone())
            .font_features(style.features.clone())
            .text_size(style.font_size)
            .text_color(style.color);
        if let Some(start) = self.fade_start {
            let elapsed = start.elapsed();
            if elapsed < FADE_IN {
                row = row.opacity(elapsed.as_secs_f32() / FADE_IN.as_secs_f32());
                window.request_animation_frame();
            } else {
                self.fade_start = None;
            }
        }
        let container = div().size_full().overflow_hidden();
        let Some(lenses) = &self.lenses else {
            return container;
        };
        if lenses.iter().all(Option::is_none) {
            // Monaco: symbols, but resolving them gave no commands.
            return container.child(row.child(div().child("no commands")));
        }
        let (background, foreground, border) = style.tooltip;
        let count = lenses.len();
        for (ix, shown) in lenses.iter().enumerate() {
            let Some(Some(command)) = shown else {
                continue;
            };
            let mut lens = div()
                .id(("code-lens", ix))
                .flex()
                .items_center()
                .children(title_with_icons(command.title.trim(), style.font_size));
            // Monaco shows `Command.tooltip` as the lens's title attribute.
            if let Some(text) = command.tooltip.clone() {
                lens = lens.tooltip(crate::tooltip::tooltip_with_colors(
                    text, background, foreground, border,
                ));
            }
            if command.id.is_empty() {
                row = row.child(lens);
            } else {
                let editor = self.editor.clone();
                let command = command.clone();
                row = row.child(
                    lens.cursor_pointer()
                        .hover(|s| s.text_color(style.hover))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| editor.run_code_lens(&command, cx));
                            }
                        }),
                );
            }
            if ix + 1 < count {
                row = row.child(div().child("\u{a0}|\u{a0}"));
            }
        }
        container.child(row)
    }
}

impl CodeEditor {
    /// Monaco `_getLayoutInfo`: the lens font size and zone height.
    fn code_lens_layout(&self) -> (Pixels, Pixels) {
        let editor_size = f32::from(self.options.font_size);
        let line_height = f32::from(self.options.resolved_line_height());
        let factor = (line_height / editor_size).max(1.3);
        let mut size = self.options.code_lens_font_size;
        if size < 5. {
            size = (editor_size * 0.9).floor();
        }
        (px(size), px((size * factor).floor()))
    }

    /// How the lenses in `zone` are drawn now (Monaco `_updateLensStyle`,
    /// and the content widget at the line's first non-whitespace column).
    fn code_lens_style(&self, zone: Option<WidgetId>, cx: &App) -> Option<LensStyle> {
        let (font_size, line_height) = self.code_lens_layout();
        let layout = self.layout.as_ref()?;
        let zone = self.widgets.zones.iter().find(|z| Some(z.id) == zone)?;
        // The zone is anchored to the end of the line above the lens's.
        let row = match &zone.anchor {
            Some(anchor) => {
                let buffer = self.model().read(cx).buffer();
                buffer.offset_to_point(buffer.resolve(anchor)).row + 1
            }
            None => 0,
        }
        .min(self.snapshot.max_point().row);
        let line = self.snapshot.line(row);
        let (column, visible) = first_non_whitespace(&line, self.cursor_config(cx).tab_size);
        // Where the text puts that column (after any inlay hints), else
        // where it would be: leading whitespace is drawn as spaces.
        let x = layout
            .line_for_point(PointUtf16::new(row, column))
            .map_or(layout.metrics.space_width * visible as f32, |line| {
                line.x_for_column(column)
            });
        let colors = &self.theme.colors;
        Some(LensStyle {
            font_family: self
                .options
                .code_lens_font_family
                .clone()
                .unwrap_or_else(|| self.options.font_family.clone()),
            // Monaco `--vscode-editorCodeLens-fontFeatureSettings`: the
            // editor font's.
            features: self.options.font_ligatures.features(),
            font_size,
            line_height,
            color: colors.code_lens_foreground,
            hover: colors.link_active_foreground,
            left: x - layout.scroll_left,
            tooltip: (
                colors.widget_background,
                colors.widget_foreground,
                colors.widget_border,
            ),
        })
    }

    /// Monaco `_updateLensStyle`'s `updateHeight`: re-lays out the zones
    /// when the font size, line height or zoom changed. Called before the
    /// zones are laid out.
    pub(crate) fn update_code_lens_heights(&mut self) {
        if self.code_lens.groups.is_empty() {
            return;
        }
        let (_, height) = self.code_lens_layout();
        if height == self.code_lens.height {
            return;
        }
        self.code_lens.height = height;
        for group in &self.code_lens.groups {
            if let Some(zone) = self.widgets.zones.iter_mut().find(|z| z.id == group.zone) {
                zone.height = ZoneHeight::Pixels(height);
            }
        }
    }

    fn code_lens_delay(&self) -> Duration {
        if self.code_lens.samples.is_empty() {
            return MIN_DELAY.mul_f32(1.5);
        }
        let average =
            self.code_lens.samples.iter().sum::<Duration>() / self.code_lens.samples.len() as u32;
        average.max(MIN_DELAY)
    }

    /// Requests lenses when the text changed: right away the first time,
    /// after a pause otherwise. Called before painting.
    pub(crate) fn refresh_code_lenses(&mut self, cx: &mut Context<Self>) {
        let model = self.model().read(cx);
        // Monaco: no lenses for models too large to tokenize.
        if !self.options.code_lens
            || model.is_too_large_for_tokenization()
            || code_lens_providers(cx, self.provider_target(cx)).is_empty()
        {
            if !self.code_lens.groups.is_empty() || self.code_lens.task.is_some() {
                self.clear_code_lenses(cx);
            }
            return;
        }
        let version = model.buffer().version();
        // `refresh_code_lenses` asks again for the same text.
        let epoch = cx
            .try_global::<crate::providers::CodeLensEpoch>()
            .map_or(0, |e| e.0);
        if epoch == self.code_lens.epoch
            && (self.code_lens.version.as_ref() == Some(&version)
                || self.code_lens.pending.as_ref() == Some(&version))
        {
            return;
        }
        self.code_lens.epoch = epoch;
        let delay = if self.code_lens.version.is_some() {
            self.code_lens_delay()
        } else {
            Duration::ZERO
        };
        self.code_lens.pending = Some(version);
        self.code_lens.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok(requests) = this.update(cx, |this, cx| {
                let model = this.model().read(cx);
                let context = model.provider_context();
                code_lens_providers(cx, this.provider_target(cx))
                    .into_iter()
                    .enumerate()
                    .map(|(rank, provider)| {
                        let task = provider.provide_code_lenses(&context, cx);
                        (rank, provider, task)
                    })
                    .collect::<Vec<_>>()
            }) else {
                return;
            };
            let started = Instant::now();
            let mut items = Vec::new();
            for (rank, provider, task) in requests {
                match task.await {
                    Ok(lenses) => items.extend(lenses.into_iter().map(|lens| Item {
                        lens,
                        provider: provider.clone(),
                        rank,
                        resolving: false,
                    })),
                    Err(err) => log::warn!("code lenses failed: {err:#}"),
                }
            }
            this.update(cx, |this, cx| {
                if this.code_lens.samples.len() == 6 {
                    this.code_lens.samples.remove(0);
                }
                this.code_lens.samples.push(started.elapsed());
                this.code_lens.pending = None;
                this.code_lens.task = None;
                if this.model().read(cx).buffer().version() != version {
                    return;
                }
                this.code_lens.version = Some(version);
                this.render_code_lenses(items, cx);
            })
            .ok();
        }));
    }

    /// Monaco `_renderCodeLensSymbols`: one zone per line with lenses,
    /// sorted by line, provider and column.
    fn render_code_lenses(&mut self, mut items: Vec<Item>, cx: &mut Context<Self>) {
        let max_row = self.model().read(cx).buffer().max_point().row;
        items.retain(|item| item.lens.range.start.row <= max_row);
        items.sort_by_key(|item| {
            (
                item.lens.range.start.row,
                item.rank,
                item.lens.range.start.column,
            )
        });
        let mut groups: Vec<(u32, Vec<Item>)> = Vec::new();
        for item in items {
            match groups.last_mut() {
                Some((row, group)) if *row == item.lens.range.start.row => group.push(item),
                _ => groups.push((item.lens.range.start.row, vec![item])),
            }
        }
        self.clear_code_lenses(cx);
        let (_, height) = self.code_lens_layout();
        self.code_lens.height = height;
        let editor = cx.entity().downgrade();
        for (row, items) in groups {
            // Monaco `CodeLensWidget`: only the lenses that have commands
            // already, without fading in; none leaves the zone empty.
            let resolved: Vec<Shown> = items
                .iter()
                .filter_map(|item| item.lens.command.clone())
                .map(|command| Some(Some(command)))
                .collect();
            let view = cx.new(|_| CodeLensLine {
                editor: editor.clone(),
                zone: None,
                lenses: (!resolved.is_empty()).then_some(resolved),
                fade_start: None,
            });
            let zone = self.add_view_zone(
                ViewZone::new(row, ZoneHeight::Pixels(height), view.clone().into()),
                cx,
            );
            view.update(cx, |view, _| view.zone = Some(zone));
            self.code_lens.groups.push(Group {
                row,
                items,
                view,
                zone,
            });
        }
        self.resolve_visible_code_lenses(cx);
    }

    /// Monaco `_resolveCodeLensesInViewport`: resolves the lenses without
    /// commands on (or near) the visible lines, then shows each line's
    /// lenses once all of them are back (`updateCommands`).
    pub(crate) fn resolve_visible_code_lenses(&mut self, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let (Some(first), Some(last)) = (layout.lines.first(), layout.lines.last()) else {
            return;
        };
        let rows = first.row.saturating_sub(1)..last.row + 2;
        let context = self.model().read(cx).provider_context();
        // (group, its items' resolves: `None` for those that keep their
        // lens).
        let mut requests = Vec::new();
        for group in &mut self.code_lens.groups {
            if !rows.contains(&group.row)
                || group
                    .items
                    .iter()
                    .all(|item| item.lens.command.is_some() || item.resolving)
            {
                continue;
            }
            let tasks: Vec<_> = group
                .items
                .iter_mut()
                .map(|item| {
                    if item.lens.command.is_some() || item.resolving {
                        return None;
                    }
                    item.resolving = true;
                    item.provider.resolve_code_lens(&context, &item.lens, cx)
                })
                .collect();
            requests.push((group.zone, tasks));
        }
        if requests.is_empty() {
            return;
        }
        // Not cancelled by later requests (which skip these lenses); the
        // zone tells whether the line's lenses are still the same.
        cx.spawn(async move |this, cx| {
            for (zone, tasks) in requests {
                let mut resolved = Vec::with_capacity(tasks.len());
                for task in tasks {
                    resolved.push(match task {
                        Some(task) => Some(task.await.map_err(|err| {
                            log::warn!("resolving a code lens failed: {err:#}");
                        })),
                        None => None,
                    });
                }
                let done = this.update(cx, |this, cx| {
                    let Some(group) = this.code_lens.groups.iter_mut().find(|g| g.zone == zone)
                    else {
                        return true;
                    };
                    let mut shown = Vec::with_capacity(resolved.len());
                    for (item, result) in group.items.iter_mut().zip(resolved) {
                        shown.push(match result {
                            Some(Ok(lens)) => {
                                item.lens.command = lens.command.clone();
                                Some(lens.command)
                            }
                            // Monaco: a failed resolve has no symbol.
                            Some(Err(())) => None,
                            None => Some(item.lens.command.clone()),
                        });
                    }
                    group.view.update(cx, |view, cx| {
                        view.set_lenses(shown, true);
                        cx.notify();
                    });
                    true
                });
                if done.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Monaco `CodeLensContribution._onModelChange`: the old model's lenses
    /// and requests go, and the new model's are asked for on the next
    /// frame (a new model can share the old one's text version).
    pub(crate) fn reset_code_lenses(&mut self, cx: &mut Context<Self>) {
        self.clear_code_lenses(cx);
        self.code_lens.task = None;
        self.code_lens.pending = None;
        self.code_lens.version = None;
    }

    fn clear_code_lenses(&mut self, cx: &mut Context<Self>) {
        let groups = std::mem::take(&mut self.code_lens.groups);
        for group in groups {
            self.remove_widget(group.zone, cx);
        }
        if !self.options.code_lens {
            self.code_lens.task = None;
            self.code_lens.pending = None;
            self.code_lens.version = None;
        }
    }

    /// Runs a clicked lens's command: through the command registry, or
    /// [`CodeEditorEvent::ExecuteCommand`] for the host.
    pub(crate) fn run_code_lens(&mut self, command: &Command, cx: &mut Context<Self>) {
        if crate::commands::execute_command(cx, command).is_err() {
            cx.emit(CodeEditorEvent::ExecuteCommand(command.clone()));
        }
    }

    /// Lists the commands of the code lenses above the cursor's line in the
    /// quick input and runs the chosen one (Monaco
    /// `codelens.showLensesInCurrentLine`). Nothing shows when the line has
    /// none.
    pub fn show_lenses_in_current_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.cursors.primary().position.row;
        let commands: Vec<Command> = self
            .code_lens
            .groups
            .iter()
            .filter(|group| group.row == row)
            .flat_map(|group| group.items.iter())
            .filter_map(|item| item.lens.command.clone())
            .collect();
        if commands.is_empty() {
            return;
        }
        let labels = commands.iter().map(|c| c.title.clone()).collect();
        self.show_quick_pick(
            Some("Select a command".into()),
            labels,
            move |editor, ix, _, cx| {
                if let Some(command) = commands.get(ix) {
                    editor.run_code_lens(command, cx);
                }
            },
            window,
            cx,
        );
    }

    /// The lenses shown, as (row, titles), for tests.
    #[doc(hidden)]
    pub fn code_lenses_for_tests(&self) -> Vec<(u32, Vec<Option<String>>)> {
        self.code_lens
            .groups
            .iter()
            .map(|group| {
                (
                    group.row,
                    group
                        .items
                        .iter()
                        .map(|item| item.lens.command.as_ref().map(|c| c.title.to_string()))
                        .collect(),
                )
            })
            .collect()
    }

    /// Each lens line's text as drawn (empty while waiting to resolve),
    /// whether it's fading in, and its zone height, by row, for tests.
    #[doc(hidden)]
    pub fn code_lens_lines_for_tests(&self, cx: &App) -> Vec<(u32, String, bool, Pixels)> {
        self.code_lens
            .groups
            .iter()
            .map(|group| {
                let view = group.view.read(cx);
                let height = self
                    .widgets
                    .zones
                    .iter()
                    .find(|z| z.id == group.zone)
                    .map_or(px(0.), |zone| match zone.height {
                        ZoneHeight::Pixels(height) => height,
                        ZoneHeight::Lines(lines) => self.options.resolved_line_height() * lines,
                    });
                (group.row, view.text(), view.fade_start.is_some(), height)
            })
            .collect()
    }

    /// Where the lenses above `row` start (window x, and the middle of
    /// their zone when the line is laid out), their font size and color,
    /// as drawn, for tests.
    #[doc(hidden)]
    pub fn code_lens_style_for_tests(
        &self,
        row: u32,
        cx: &App,
    ) -> Option<(gpui::Point<Pixels>, Pixels, Hsla)> {
        let group = self.code_lens.groups.iter().find(|g| g.row == row)?;
        let style = self.code_lens_style(Some(group.zone), cx)?;
        let layout = self.layout.as_ref()?;
        let y = layout
            .line_for_point(PointUtf16::new(row, 0))
            .map_or(px(0.), |line| {
                layout.line_top(line.view_line) - style.line_height / 2.
            });
        Some((
            gpui::point(layout.text_bounds.left() + style.left, y),
            style.font_size,
            style.color,
        ))
    }

    /// Runs the command of the `index`th lens above `row` (for tests).
    #[doc(hidden)]
    pub fn click_code_lens_for_tests(&mut self, row: u32, index: usize, cx: &mut Context<Self>) {
        let command = self
            .code_lens
            .groups
            .iter()
            .find(|g| g.row == row)
            .and_then(|g| g.items.get(index))
            .and_then(|item| item.lens.command.clone());
        if let Some(command) = command {
            self.run_code_lens(&command, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monaco `getLineFirstNonWhitespaceColumn`, with the visible column
    /// tabs expand to.
    #[test]
    fn first_non_whitespace_columns() {
        assert_eq!(first_non_whitespace("fn a()", 4), (0, 0));
        assert_eq!(first_non_whitespace("    fn a()", 4), (4, 4));
        assert_eq!(first_non_whitespace("\tfn a()", 4), (1, 4));
        assert_eq!(first_non_whitespace("\t\tfn a()", 4), (2, 8));
        // A tab after two spaces goes to the next stop.
        assert_eq!(first_non_whitespace("  \tfn a()", 4), (3, 4));
        assert_eq!(first_non_whitespace("\tfn a()", 2), (1, 2));
        // Blank lines: column 0, as Monaco's 0 is validated to column 1.
        assert_eq!(first_non_whitespace("  \t", 4), (0, 0));
        assert_eq!(first_non_whitespace("", 4), (0, 0));
    }

    fn command(title: &str) -> Command {
        Command::new("x", title)
    }

    fn line() -> CodeLensLine {
        CodeLensLine {
            editor: WeakEntity::new_invalid(),
            zone: None,
            lenses: None,
            fade_start: None,
        }
    }

    /// Monaco `withCommands`: "no commands" only when every resolve
    /// failed; the separator follows each lens with a command but the
    /// last.
    #[test]
    fn with_commands() {
        let mut view = line();
        assert_eq!(view.text(), "");
        view.set_lenses(vec![None, None], true);
        assert_eq!(view.text(), "no commands");
        assert!(view.fade_start.is_none());
        view.set_lenses(vec![Some(Some(command("a"))), Some(None)], true);
        assert_eq!(view.text(), "a | ");
        assert!(view.fade_start.is_some());
        let mut view = line();
        view.set_lenses(vec![Some(None), Some(Some(command("b")))], true);
        assert_eq!(view.text(), "b");
    }

    /// Monaco fades lenses in only when resolved ones fill an empty zone.
    #[test]
    fn fades_in_once() {
        let mut view = line();
        view.set_lenses(vec![Some(Some(command("a")))], false);
        assert!(view.fade_start.is_none());
        let mut view = line();
        view.set_lenses(vec![Some(Some(command("a")))], true);
        assert!(view.fade_start.is_some());
        view.fade_start = None;
        view.set_lenses(vec![Some(Some(command("b")))], true);
        assert!(view.fade_start.is_none());
    }

    struct WithTooltip;

    impl CodeLensProvider for WithTooltip {
        fn provide_code_lenses(
            &self,
            _: &crate::providers::ProviderContext,
            _: &mut App,
        ) -> Task<anyhow::Result<Vec<CodeLens>>> {
            Task::ready(Ok(vec![CodeLens {
                range: PointUtf16::new(1, 0)..PointUtf16::new(1, 2),
                command: Some(Command {
                    tooltip: Some("Show the references".into()),
                    ..Command::new("", "2 references")
                }),
                ..Default::default()
            }]))
        }
    }

    /// Monaco shows `Command.tooltip` as the lens's title.
    #[gpui::test]
    fn lenses_show_their_tooltips(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            crate::providers::register_code_lens_provider(
                cx,
                crate::providers::LanguageSelector::All,
                WithTooltip,
            );
        });
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let model = cx.new(|_| crate::TextModel::new("\nfn a() {}\n"));
            CodeEditor::new(model, window, cx)
        });
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
        };
        draw(cx);
        draw(cx);
        let (at, _, _) = editor
            .read_with(cx, |e, cx| e.code_lens_style_for_tests(1, cx))
            .unwrap();
        cx.simulate_mouse_move(
            at + gpui::point(px(10.), px(0.)),
            None,
            gpui::Modifiers::none(),
        );
        cx.executor().advance_clock(Duration::from_secs(1));
        draw(cx);
        assert!(
            crate::tooltip::RENDERED
                .with(|r| r.borrow().iter().any(|text| text == "Show the references"))
        );
    }

    #[gpui::test]
    fn titles_render_icons(_cx: &mut gpui::TestAppContext) {
        let parts = parse_label_with_icons("$(references) 2 references");
        assert_eq!(parts.len(), 2);
        assert_eq!(title_with_icons("$(zap) run", px(12.)).len(), 2);
        assert_eq!(title_with_icons("plain", px(12.)).len(), 1);
    }
}
