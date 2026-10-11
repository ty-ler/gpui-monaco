//! Code lenses (Monaco's `contrib/codelens`): commands from providers shown
//! in a short view zone above lines, refreshed after edits and resolved
//! when they scroll into view; clicking one runs its command.

use std::sync::Arc;
use std::time::Duration;

use web_time::Instant;

use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::providers::{CodeLens, CodeLensProvider, Command, code_lens_providers};
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};
use gpui::{
    AppContext as _, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Render, SharedString, Styled, Task, WeakEntity, Window, div, px,
};

/// Monaco's debounce floor for requesting and resolving lenses.
const MIN_DELAY: Duration = Duration::from_millis(250);

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

/// The lenses of one line and their zone.
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
    pending: Option<text::Version>,
    task: Option<Task<()>>,
    resolve_task: Option<Task<()>>,
    samples: Vec<Duration>,
}

/// The lenses above one line (Monaco `CodeLensContentWidget`): titles
/// separated by `|`, the ones with commands clickable.
pub struct CodeLensLine {
    editor: WeakEntity<CodeEditor>,
    lenses: Vec<Option<Command>>,
    indent: Pixels,
    font_family: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    color: gpui::Hsla,
    hover: gpui::Hsla,
}

impl Render for CodeLensLine {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .pl(self.indent)
            .h(self.line_height)
            .flex()
            .items_center()
            .whitespace_nowrap()
            .overflow_hidden()
            .font_family(self.font_family.clone())
            .text_size(self.font_size)
            .text_color(self.color);
        let count = self.lenses.len();
        for (ix, command) in self.lenses.iter().enumerate() {
            let Some(command) = command else {
                continue;
            };
            let title = SharedString::from(command.title.trim().to_string());
            if command.id.is_empty() {
                row = row.child(div().child(title));
            } else {
                let editor = self.editor.clone();
                let command = command.clone();
                row = row.child(
                    div()
                        .id(("code-lens", ix))
                        .cursor_pointer()
                        .hover(|style| style.text_color(self.hover))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = editor.upgrade() {
                                editor.update(cx, |editor, cx| editor.run_code_lens(&command, cx));
                            }
                        })
                        .child(title),
                );
            }
            if ix + 1 < count {
                row = row.child(div().child("\u{a0}|\u{a0}"));
            }
        }
        if self.lenses.iter().all(Option::is_none) && count > 0 {
            row = row.child(div().child("no commands"));
        }
        row
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
        let language = model.language_id().map(|l| l.to_string());
        if !self.options.code_lens || code_lens_providers(cx, language.as_deref()).is_empty() {
            if !self.code_lens.groups.is_empty() || self.code_lens.task.is_some() {
                self.clear_code_lenses(cx);
            }
            return;
        }
        let version = model.buffer().version();
        if self.code_lens.version.as_ref() == Some(&version)
            || self.code_lens.pending.as_ref() == Some(&version)
        {
            return;
        }
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
                let language = model.language_id().map(|l| l.to_string());
                code_lens_providers(cx, language.as_deref())
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
        let (font_size, height) = self.code_lens_layout();
        let font_family = self
            .options
            .code_lens_font_family
            .clone()
            .unwrap_or_else(|| self.options.font_family.clone());
        let editor = cx.entity().downgrade();
        for (row, items) in groups {
            // Lined up with the line's first non-whitespace character.
            let line = self.snapshot.line(row);
            let indent_columns = line.len() - line.trim_start().len();
            let indent = self.layout.as_ref().map_or(px(0.), |layout| {
                layout.metrics.em_width * indent_columns as f32
            });
            let view = cx.new(|_| CodeLensLine {
                editor: editor.clone(),
                lenses: items.iter().map(|item| item.lens.command.clone()).collect(),
                indent,
                font_family: font_family.clone(),
                font_size,
                line_height: height,
                color: self.theme.colors.code_lens_foreground,
                hover: self.theme.colors.link_active_foreground,
            });
            let zone = self.add_view_zone(
                ViewZone {
                    after_line: row,
                    height: ZoneHeight::Pixels(height),
                    view: view.clone().into(),
                },
                cx,
            );
            self.code_lens.groups.push(Group {
                row,
                items,
                view,
                zone,
            });
        }
        self.resolve_visible_code_lenses(cx);
    }

    /// Monaco `_resolveCodeLensesInViewport`: resolves the unresolved lenses
    /// on (or near) the visible lines.
    pub(crate) fn resolve_visible_code_lenses(&mut self, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let (Some(first), Some(last)) = (layout.lines.first(), layout.lines.last()) else {
            return;
        };
        let rows = first.row.saturating_sub(1)..last.row + 2;
        let context = self.model().read(cx).provider_context();
        let mut requests = Vec::new();
        for (group_ix, group) in self.code_lens.groups.iter_mut().enumerate() {
            if !rows.contains(&group.row) {
                continue;
            }
            for (item_ix, item) in group.items.iter_mut().enumerate() {
                if item.lens.command.is_some() || item.resolving {
                    continue;
                }
                if let Some(task) = item.provider.resolve_code_lens(&context, &item.lens, cx) {
                    item.resolving = true;
                    requests.push((group_ix, item_ix, task));
                }
            }
        }
        if requests.is_empty() {
            return;
        }
        let version = self.code_lens.version;
        self.code_lens.resolve_task = Some(cx.spawn(async move |this, cx| {
            let mut resolved = Vec::new();
            for (group_ix, item_ix, task) in requests {
                if let Ok(lens) = task.await {
                    resolved.push((group_ix, item_ix, lens));
                }
            }
            this.update(cx, |this, cx| {
                if this.code_lens.version != version {
                    return;
                }
                let mut changed = Vec::new();
                for (group_ix, item_ix, lens) in resolved {
                    if let Some(item) = this
                        .code_lens
                        .groups
                        .get_mut(group_ix)
                        .and_then(|g| g.items.get_mut(item_ix))
                    {
                        item.lens = lens;
                        changed.push(group_ix);
                    }
                }
                changed.dedup();
                for group_ix in changed {
                    let group = &this.code_lens.groups[group_ix];
                    let lenses = group.items.iter().map(|i| i.lens.command.clone()).collect();
                    group.view.update(cx, |view, cx| {
                        view.lenses = lenses;
                        cx.notify();
                    });
                }
            })
            .ok();
        }));
    }

    fn clear_code_lenses(&mut self, cx: &mut Context<Self>) {
        let groups = std::mem::take(&mut self.code_lens.groups);
        for group in groups {
            self.remove_widget(group.zone, cx);
        }
        self.code_lens.resolve_task = None;
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
