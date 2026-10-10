//! Quick access (Monaco's standalone `quickAccess` and quick input): a
//! filtered list over the editor, opened with a prefix that picks the
//! provider:
//! - `:` goes to a line and column (`::` to a character offset), Ctrl+G,
//! - `@` goes to a symbol (`@:` grouped by kind), Cmd/Ctrl+Shift+O,
//! - anything else lists the prefixes (help).
//!
//! Ports `gotoLineQuickAccess.ts`, `gotoSymbolQuickAccess.ts` and
//! `editorNavigationQuickAccess.ts` (MIT); the widget follows VS Code's
//! quick input.

use std::ops::Range;

use gpui::{
    AppContext as _, Context, Entity, FontWeight, HighlightStyle, Hsla, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString, Styled, StyledText,
    Subscription, Task, WeakEntity, Window, div, px, rgb,
};
use text::PointUtf16;

use crate::actions::{
    Newline, QuickInputFirst, QuickInputLast, QuickInputNext, QuickInputPageNext,
    QuickInputPagePrevious, QuickInputPrevious, RemoveSecondaryCursors,
};
use crate::cursor::Selection;
use crate::decorations::{DecorationId, DecorationOptions, OverviewRulerLane};
use crate::editor::{CodeEditor, CodeEditorEvent, ViewState};
use crate::model::TextModelEvent;
use crate::options::EditorOptions;
use crate::outline::flatten_symbols;
use crate::providers::{DocumentSymbol, SymbolKind};
use crate::view::reveal::RevealType;
use crate::widgets::{OverlayPosition, OverlayWidget, WidgetId};

/// The key context of the quick input.
pub const QUICK_INPUT_KEY_CONTEXT: &str = "QuickInput";

/// Row height (VS Code's quick input list).
const ROW_HEIGHT: f32 = 22.;

/// What choosing an item does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum QuickAction {
    /// Nothing (a message).
    None,
    /// Replace the input with this text (help items switch provider).
    SetValue(String),
    /// Move the cursor to `selection` and reveal it; `highlight` is
    /// previewed while the item is active.
    GoTo {
        selection: PointUtf16,
        highlight: Range<PointUtf16>,
    },
}

/// One row of the list.
#[derive(Clone, Debug)]
pub(crate) enum QuickRow {
    /// A labelled separator.
    Separator(SharedString),
    /// A choosable item.
    Item(QuickItem),
}

#[derive(Clone, Debug)]
pub(crate) struct QuickItem {
    pub label: SharedString,
    /// Byte ranges of `label` that matched the query.
    pub highlights: Vec<Range<usize>>,
    pub description: Option<SharedString>,
    pub icon: Option<(&'static str, Hsla)>,
    pub strikethrough: bool,
    pub action: QuickAction,
}

impl QuickItem {
    fn message(label: impl Into<SharedString>, action: QuickAction) -> Self {
        Self {
            label: label.into(),
            highlights: Vec::new(),
            description: None,
            icon: None,
            strikethrough: false,
            action,
        }
    }
}

/// The editor's quick access state.
pub(crate) struct QuickAccessState {
    widget: Option<(Entity<QuickInput>, WidgetId)>,
    /// Restored when the list closes without choosing (Monaco).
    view_state: ViewState,
    decorations: Vec<DecorationId>,
    /// The document's symbols, fetched once per session.
    symbols: Option<Vec<DocumentSymbol>>,
    _symbols_task: Option<Task<()>>,
}

/// The quick input: an input box and a list (an overlay widget at the top
/// center of the editor).
pub struct QuickInput {
    editor: WeakEntity<CodeEditor>,
    input: Entity<CodeEditor>,
    rows: Vec<QuickRow>,
    active: Option<usize>,
    scroll: usize,
    visible_rows: usize,
    width: Pixels,
    colors: crate::theme::EditorColors,
    _subscriptions: Vec<Subscription>,
}

/// A short glyph per symbol kind, standing in for Monaco's codicons.
fn symbol_icon(kind: SymbolKind) -> (&'static str, Hsla) {
    use SymbolKind::*;
    let purple: Hsla = rgb(0xb180d7).into();
    let blue: Hsla = rgb(0x75beff).into();
    let orange: Hsla = rgb(0xee9d28).into();
    let gray: Hsla = rgb(0xc5c5c5).into();
    match kind {
        Method | Function | Constructor | Operator => ("ƒ", purple),
        Field | Variable | Property | Key | TypeParameter => ("𝑥", blue),
        Class | Struct | Interface | Enum | Object => ("◆", orange),
        EnumMember | Constant | Number | Boolean | Null => ("π", blue),
        Module | Namespace | Package | File => ("{}", gray),
        String | Array | Event => ("ab", gray),
    }
}

/// Monaco `NLS_SYMBOL_KIND_CACHE` groups for `@:`.
fn kind_group(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Null => "properties",
        kind => kind.name(),
    }
}

/// The visible column of UTF-16 `column` in `line` (tabs expand).
fn visible_column(line: &str, column: u32, tab_size: u32) -> u32 {
    let mut visible = 0;
    for (units, ch) in line.chars().scan(0u32, |units, ch| {
        let at = *units;
        *units += ch.len_utf16() as u32;
        Some((at, ch))
    }) {
        if units >= column {
            break;
        }
        visible = if ch == '\t' {
            visible + tab_size - visible % tab_size
        } else {
            visible + 1
        };
    }
    visible
}

/// The UTF-16 column at visible column `visible` in `line`.
fn column_from_visible(line: &str, visible: u32, tab_size: u32) -> u32 {
    let mut current = 0;
    let mut column = 0;
    for ch in line.chars() {
        let next = if ch == '\t' {
            current + tab_size - current % tab_size
        } else {
            current + 1
        };
        if next > visible {
            break;
        }
        current = next;
        column += ch.len_utf16() as u32;
    }
    column
}

impl QuickInput {
    #[allow(clippy::too_many_arguments)]
    fn new(
        editor: WeakEntity<CodeEditor>,
        value: &str,
        (rows, active): (Vec<QuickRow>, Option<usize>),
        width: Pixels,
        visible_rows: usize,
        theme: std::sync::Arc<crate::Theme>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let colors = theme.colors.clone();
        let input = cx.new(|cx| {
            let mut input = CodeEditor::with_text(value, window, cx);
            input.update_options(EditorOptions::single_line_input(), cx);
            input.set_theme(theme, cx);
            let end = PointUtf16::new(0, value.encode_utf16().count() as u32);
            input.set_selections(&[Selection::collapsed(end)], cx);
            input
        });
        let model = input.read(cx).model().clone();
        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event: &TextModelEvent, cx| {
                if matches!(event, TextModelEvent::Edited { .. }) {
                    this.refresh(cx);
                }
            }),
            // Monaco hides the quick input when it loses focus.
            cx.subscribe(&input, |this, _, event: &CodeEditorEvent, cx| {
                if matches!(event, CodeEditorEvent::Blurred) {
                    this.close(false, cx);
                }
            }),
        ];
        window.focus(&input.read(cx).focus_handle.clone(), cx);
        let mut this = Self {
            editor,
            input,
            rows,
            active: None,
            scroll: 0,
            visible_rows,
            width,
            colors,
            _subscriptions: subscriptions,
        };
        this.active = active.or_else(|| this.item_indices().first().copied());
        this.keep_active_visible();
        this
    }

    /// The active item's action.
    fn active_action(&self) -> Option<QuickAction> {
        self.active_item().map(|item| item.action.clone())
    }

    fn value(&self, cx: &gpui::App) -> String {
        self.input.read(cx).model().read(cx).text()
    }

    /// Recomputes the list for the input's text.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let value = self.value(cx);
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let (rows, active) = editor.update(cx, |editor, cx| editor.quick_access_rows(&value, cx));
        self.rows = rows;
        self.active = active.or_else(|| self.item_indices().first().copied());
        self.scroll = 0;
        self.keep_active_visible();
        self.preview(cx);
        cx.notify();
    }

    fn item_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, QuickRow::Item(_)))
            .map(|(ix, _)| ix)
            .collect()
    }

    fn active_item(&self) -> Option<&QuickItem> {
        match self.rows.get(self.active?) {
            Some(QuickRow::Item(item)) => Some(item),
            _ => None,
        }
    }

    fn keep_active_visible(&mut self) {
        let Some(active) = self.active else {
            return;
        };
        if active < self.scroll {
            self.scroll = active;
        } else if active >= self.scroll + self.visible_rows {
            self.scroll = active + 1 - self.visible_rows;
        }
    }

    /// Moves the active item by `delta` items (wrapping for single steps).
    fn move_active(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.item_indices();
        if items.is_empty() {
            return;
        }
        let current = self
            .active
            .and_then(|a| items.iter().position(|ix| *ix == a))
            .unwrap_or(0) as isize;
        let count = items.len() as isize;
        let next = if delta.abs() == 1 {
            (current + delta).rem_euclid(count)
        } else {
            (current + delta).clamp(0, count - 1)
        };
        self.active = Some(items[next as usize]);
        self.keep_active_visible();
        self.preview(cx);
        cx.notify();
    }

    fn preview(&mut self, cx: &mut Context<Self>) {
        let action = self.active_item().map(|item| item.action.clone());
        if let (Some(action), Some(editor)) = (action, self.editor.upgrade()) {
            editor.update(cx, |editor, cx| editor.quick_access_preview(&action, cx));
        }
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        let Some(action) = self.active_item().map(|item| item.action.clone()) else {
            return;
        };
        match action {
            QuickAction::None => {}
            QuickAction::SetValue(value) => {
                let end = PointUtf16::new(0, value.encode_utf16().count() as u32);
                self.input.update(cx, |input, cx| {
                    input
                        .model()
                        .update(cx, |model, cx| model.set_value(&value, cx));
                    input.set_selections(&[Selection::collapsed(end)], cx);
                });
            }
            QuickAction::GoTo { selection, .. } => {
                if let Some(editor) = self.editor.upgrade() {
                    editor.update(cx, |editor, cx| editor.quick_access_go_to(selection, cx));
                }
                self.close(true, cx);
            }
        }
    }

    fn close(&mut self, accepted: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            let this = cx.entity().downgrade();
            // Closing removes this view; do it after this update.
            cx.defer(move |cx| {
                editor.update(cx, |editor, cx| editor.close_quick_access(accepted, cx));
                drop(this);
            });
        }
    }

    fn render_item(&self, ix: usize, item: &QuickItem, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = &self.colors;
        let is_active = self.active == Some(ix);
        let highlight: Hsla = rgb(0x0097fb).into();
        let label_highlights: Vec<(Range<usize>, HighlightStyle)> = if item.strikethrough {
            vec![(
                0..item.label.len(),
                HighlightStyle {
                    strikethrough: Some(gpui::StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..Default::default()
                },
            )]
        } else {
            item.highlights
                .iter()
                .map(|range| {
                    (
                        range.clone(),
                        HighlightStyle {
                            color: Some(highlight),
                            font_weight: Some(FontWeight::BOLD),
                            ..Default::default()
                        },
                    )
                })
                .collect()
        };
        div()
            .id(("quick-item", ix))
            .h(px(ROW_HEIGHT))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .overflow_hidden()
            .whitespace_nowrap()
            .cursor_pointer()
            .when(is_active, |this| {
                this.bg(colors.suggest_selected_background)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.active = Some(ix);
                    this.accept(cx);
                }),
            )
            .children(
                item.icon
                    .map(|(glyph, color)| div().w(px(16.)).text_color(color).child(glyph)),
            )
            .child(StyledText::new(item.label.clone()).with_highlights(label_highlights))
            .children(
                item.description
                    .clone()
                    .map(|description| div().opacity(0.7).text_size(px(12.)).child(description)),
            )
            .into_any_element()
    }
}

impl Render for QuickInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors.clone();
        let end = (self.scroll + self.visible_rows).min(self.rows.len());
        let rows: Vec<gpui::AnyElement> = (self.scroll..end)
            .map(|ix| match self.rows[ix].clone() {
                QuickRow::Separator(label) => div()
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .border_t_1()
                    .border_color(colors.widget_border)
                    .text_size(px(11.))
                    .opacity(0.7)
                    .child(label)
                    .into_any_element(),
                QuickRow::Item(item) => self.render_item(ix, &item, cx),
            })
            .collect();
        div()
            .key_context(QUICK_INPUT_KEY_CONTEXT)
            .w(self.width)
            .flex()
            .flex_col()
            .bg(colors.widget_background)
            .border_1()
            .border_color(colors.widget_border)
            .shadow_md()
            .text_size(px(13.))
            .text_color(colors.widget_foreground)
            .on_action(cx.listener(|this, _: &QuickInputNext, _, cx| this.move_active(1, cx)))
            .on_action(cx.listener(|this, _: &QuickInputPrevious, _, cx| this.move_active(-1, cx)))
            .on_action(cx.listener(|this, _: &QuickInputPageNext, _, cx| {
                let page = this.visible_rows as isize;
                this.move_active(page, cx)
            }))
            .on_action(cx.listener(|this, _: &QuickInputPagePrevious, _, cx| {
                let page = this.visible_rows as isize;
                this.move_active(-page, cx)
            }))
            .on_action(cx.listener(|this, _: &QuickInputFirst, _, cx| {
                this.move_active(-(this.rows.len() as isize), cx)
            }))
            .on_action(cx.listener(|this, _: &QuickInputLast, _, cx| {
                this.move_active(this.rows.len() as isize, cx)
            }))
            .on_action(cx.listener(|this, _: &Newline, _, cx| this.accept(cx)))
            .on_action(cx.listener(|this, _: &RemoveSecondaryCursors, _, cx| this.close(false, cx)))
            .child(
                div().p(px(6.)).child(
                    div()
                        .h(px(24.))
                        .px(px(4.))
                        .bg(colors.input_background)
                        .border_1()
                        .border_color(colors.input_option_active_border)
                        .child(self.input.clone()),
                ),
            )
            .child(div().pb(px(4.)).children(rows))
    }
}

use gpui::prelude::FluentBuilder as _;

impl CodeEditor {
    /// Opens quick access with `value` in the input (Monaco
    /// `quickInputService.quickAccess.show(prefix)`): `":"` goes to a line,
    /// `"@"` to a symbol, `""` shows the help.
    pub fn open_quick_access(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_quick_access(false, cx);
        let (width, height) = self.layout.as_ref().map_or((px(600.), px(400.)), |layout| {
            (layout.bounds.size.width, layout.bounds.size.height)
        });
        // VS Code: 62% of the editor's width (at most 600), a list at most
        // 40% of its height.
        let width = (width * 0.62).min(px(600.));
        let visible_rows = ((height * 0.4 / px(ROW_HEIGHT)).floor() as usize).max(3);
        self.quick = Some(QuickAccessState {
            widget: None,
            view_state: self.save_view_state(),
            decorations: Vec::new(),
            symbols: None,
            _symbols_task: None,
        });
        // The first rows come from here: the widget can't update this
        // editor while it's being updated.
        let rows = self.quick_access_rows(value, cx);
        let editor = cx.entity().downgrade();
        let theme = self.theme.clone();
        let widget = cx
            .new(|cx| QuickInput::new(editor, value, rows, width, visible_rows, theme, window, cx));
        if let Some(action) = widget.read(cx).active_action() {
            self.quick_access_preview(&action, cx);
        }
        let id = self.add_overlay_widget(
            OverlayWidget {
                position: OverlayPosition::TopCenter,
                view: widget.clone().into(),
            },
            cx,
        );
        if let Some(state) = self.quick.as_mut() {
            state.widget = Some((widget, id));
        }
    }

    /// Whether quick access is open.
    pub fn is_quick_access_open(&self) -> bool {
        self.quick.is_some()
    }

    /// Closes quick access; without `accepted` it restores the view as it
    /// was when it opened (Monaco).
    pub(crate) fn close_quick_access(&mut self, accepted: bool, cx: &mut Context<Self>) {
        let Some(state) = self.quick.take() else {
            return;
        };
        if let Some((_, id)) = state.widget {
            self.remove_widget(id, cx);
        }
        if !state.decorations.is_empty() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &state.decorations, Vec::new(), cx)
            });
        }
        if !accepted {
            self.restore_view_state(&state.view_state, cx);
        }
        self.refocus = true;
        cx.notify();
    }

    /// Previews the active item: reveals it and highlights its lines.
    pub(crate) fn quick_access_preview(&mut self, action: &QuickAction, cx: &mut Context<Self>) {
        let QuickAction::GoTo {
            selection,
            highlight,
        } = action
        else {
            return;
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let start = rope.point_utf16_to_offset(PointUtf16::new(highlight.start.row, 0));
        let end = rope.point_utf16_to_offset(highlight.end);
        let decorations = vec![(
            start..end.max(start),
            DecorationOptions {
                background: Some(self.theme.colors.range_highlight_background),
                is_whole_line: true,
                overview_ruler: Some((
                    self.theme.colors.range_highlight_background.opacity(1.),
                    OverviewRulerLane::Right,
                )),
                ..Default::default()
            },
        )];
        let old = self
            .quick
            .as_mut()
            .map(|q| std::mem::take(&mut q.decorations))
            .unwrap_or_default();
        let owner = Some(cx.entity_id());
        let ids = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, decorations, cx)
        });
        if let Some(quick) = self.quick.as_mut() {
            quick.decorations = ids;
        }
        let point = *selection;
        self.reveal_range(point..point, RevealType::Center, cx);
    }

    pub(crate) fn quick_access_go_to(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        self.set_selections(&[Selection::collapsed(position)], cx);
        self.reveal_range(position..position, RevealType::Center, cx);
    }

    /// The rows for the input's `value`, and the item to activate.
    pub(crate) fn quick_access_rows(
        &mut self,
        value: &str,
        cx: &mut Context<Self>,
    ) -> (Vec<QuickRow>, Option<usize>) {
        let trimmed = value.trim();
        if let Some(rest) = trimmed.strip_prefix("::") {
            (vec![QuickRow::Item(self.goto_offset_item(rest, cx))], None)
        } else if let Some(rest) = trimmed.strip_prefix(':') {
            (vec![QuickRow::Item(self.goto_line_item(rest, cx))], None)
        } else if let Some(rest) = trimmed.strip_prefix('@') {
            self.goto_symbol_rows(rest, cx)
        } else {
            (help_rows(), None)
        }
    }

    /// Monaco `AbstractGotoLineQuickAccessProvider` for `:line[:,#]column`.
    fn goto_line_item(&self, query: &str, cx: &gpui::App) -> QuickItem {
        let rope = self.model().read(cx).buffer().rope().clone();
        let line_count = rope.max_point().row + 1;
        let tab_size = self.model().read(cx).options().tab_size;
        let mut parts = query.split([',', ':', '#']);
        let line_part = parts.next().unwrap_or("").trim();
        let column_part = parts.next();
        let Ok(line) = line_part
            .parse::<i64>()
            .map(|l| if l < 0 { line_count as i64 + 1 + l } else { l })
        else {
            return QuickItem::message(
                format!("Type a line number to go to (from 1 to {line_count})."),
                QuickAction::None,
            );
        };
        let line = line.clamp(1, line_count as i64) as u32;
        let row = line - 1;
        let text = self.snapshot.line(row);
        let max_visible = visible_column(&text, text.encode_utf16().count() as u32, tab_size) + 1;
        let go = |column: u32| QuickAction::GoTo {
            selection: PointUtf16::new(row, column),
            highlight: PointUtf16::new(row, 0)..PointUtf16::new(row, 0),
        };
        match column_part.map(str::trim) {
            None => QuickItem::message(
                format!(
                    "Press 'Enter' to go to line {line} or enter colon : to add a column number."
                ),
                go(0),
            ),
            Some(column) => match column.parse::<i64>() {
                Ok(column) => {
                    let column = if column < 0 {
                        max_visible as i64 + column
                    } else {
                        column
                    }
                    .clamp(1, max_visible as i64) as u32;
                    QuickItem::message(
                        format!("Press 'Enter' to go to line {line} at column {column}."),
                        go(column_from_visible(&text, column - 1, tab_size)),
                    )
                }
                Err(_) => QuickItem::message(
                    format!(
                        "Press 'Enter' to go to line {line} or enter a column number (from 1 to {max_visible})."
                    ),
                    go(0),
                ),
            },
        }
    }

    /// Monaco's `::offset` mode (1-based character offsets).
    fn goto_offset_item(&self, query: &str, cx: &gpui::App) -> QuickItem {
        let rope = self.model().read(cx).buffer().rope().clone();
        let length = rope.len_utf16().0 as i64;
        let tab_size = self.model().read(cx).options().tab_size;
        let Ok(mut offset) = query.trim().parse::<i64>() else {
            return QuickItem::message(
                format!("Type a character position to go to (from 1 to {length})."),
                QuickAction::None,
            );
        };
        offset -= offset.signum();
        if offset < 0 {
            offset += length;
        }
        let offset =
            rope.offset_utf16_to_offset(text::OffsetUtf16(offset.clamp(0, length) as usize));
        let point = rope.offset_to_point_utf16(offset);
        let text = self.snapshot.line(point.row);
        QuickItem::message(
            format!(
                "Press 'Enter' to go to line {} at column {}.",
                point.row + 1,
                visible_column(&text, point.column, tab_size) + 1
            ),
            QuickAction::GoTo {
                selection: point,
                highlight: PointUtf16::new(point.row, 0)..PointUtf16::new(point.row, 0),
            },
        )
    }

    /// Monaco `AbstractGotoSymbolQuickAccessProvider`.
    fn goto_symbol_rows(
        &mut self,
        query: &str,
        cx: &mut Context<Self>,
    ) -> (Vec<QuickRow>, Option<usize>) {
        let symbols = match self.quick.as_ref().and_then(|q| q.symbols.clone()) {
            Some(symbols) => symbols,
            None => {
                self.fetch_quick_symbols(cx);
                return (Vec::new(), None);
            }
        };
        let by_category = query.starts_with(':');
        let query = query.trim_start_matches(':').trim();
        // The first word matches the name, the rest the container.
        let mut words = query.split_whitespace();
        let symbol_query = words.next().unwrap_or("");
        let container_query: String = words.collect::<Vec<_>>().join(" ");
        // Monaco: sorted by start, then end descending.
        let mut flat: Vec<(Option<String>, DocumentSymbol)> = flatten_symbols(&symbols)
            .into_iter()
            .map(|(_, container, symbol)| (container, symbol.clone()))
            .collect();
        flat.sort_by_key(|(_, s)| (s.range.start, std::cmp::Reverse(s.range.end)));
        let options = crate::suggest::FuzzyScoreOptions::MONACO_DEFAULT;
        let mut items: Vec<(i32, usize, SymbolKind, QuickItem)> = Vec::new();
        for (ix, (container, symbol)) in flat.iter().enumerate() {
            let mut score = 0;
            let mut highlights = Vec::new();
            if !symbol_query.is_empty() {
                let Some(found) = crate::suggest::fuzzy_score(symbol_query, &symbol.name, options)
                else {
                    continue;
                };
                score = found.score;
                highlights = utf16_matches_to_bytes(&symbol.name, &found.matches);
                if !container_query.is_empty() {
                    let Some(container_score) = container
                        .as_deref()
                        .and_then(|c| crate::suggest::fuzzy_score(&container_query, c, options))
                    else {
                        continue;
                    };
                    score += container_score.score;
                }
            }
            items.push((
                score,
                ix,
                symbol.kind,
                QuickItem {
                    label: symbol.name.clone(),
                    highlights,
                    description: container.clone().map(Into::into),
                    icon: Some(symbol_icon(symbol.kind)),
                    strikethrough: symbol.deprecated,
                    action: QuickAction::GoTo {
                        selection: symbol.selection_range.start,
                        highlight: symbol.range.clone(),
                    },
                },
            ));
        }
        if items.is_empty() {
            let message = if query.is_empty() {
                "No editor symbols"
            } else {
                "No matching editor symbols"
            };
            return (
                vec![QuickRow::Item(QuickItem::message(
                    message,
                    QuickAction::None,
                ))],
                None,
            );
        }
        // Scored first, by score, then by position.
        items.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut rows = Vec::new();
        if by_category {
            items.sort_by(|a, b| {
                kind_group(a.2)
                    .cmp(kind_group(b.2))
                    .then(b.0.cmp(&a.0))
                    .then(a.1.cmp(&b.1))
            });
            let mut ix = 0;
            while ix < items.len() {
                let group = kind_group(items[ix].2);
                let count = items[ix..]
                    .iter()
                    .take_while(|i| kind_group(i.2) == group)
                    .count();
                rows.push(QuickRow::Separator(format!("{group} ({count})").into()));
                rows.extend(
                    items[ix..ix + count]
                        .iter()
                        .map(|i| QuickRow::Item(i.3.clone())),
                );
                ix += count;
            }
        } else {
            rows.push(QuickRow::Separator(
                format!("symbols ({})", items.len()).into(),
            ));
            rows.extend(items.iter().map(|i| QuickRow::Item(i.3.clone())));
        }
        // With no query, the innermost symbol around the cursor is active.
        let active = query.is_empty().then(|| {
            let cursor = self.cursors.primary().position;
            rows.iter().rposition(|row| {
                matches!(row, QuickRow::Item(QuickItem { action: QuickAction::GoTo { highlight, .. }, .. })
                    if highlight.start <= cursor && cursor <= highlight.end)
            })
        });
        (rows, active.flatten())
    }

    fn fetch_quick_symbols(&mut self, cx: &mut Context<Self>) {
        if self
            .quick
            .as_ref()
            .is_none_or(|q| q._symbols_task.is_some())
        {
            return;
        }
        let task = self.document_symbols(cx);
        let fetch = cx.spawn(async move |this, cx| {
            let symbols = task.await;
            let Ok(Some(widget)) = this.update(cx, |this, _| {
                let quick = this.quick.as_mut()?;
                quick.symbols = Some(symbols);
                quick.widget.as_ref().map(|(widget, _)| widget.clone())
            }) else {
                return;
            };
            // Outside the editor's update: refreshing asks it for rows.
            widget.update(cx, |widget, cx| widget.refresh(cx));
        });
        if let Some(quick) = self.quick.as_mut() {
            quick._symbols_task = Some(fetch);
        }
    }

    /// The quick access list as labels (separators in brackets), for tests.
    #[doc(hidden)]
    pub fn quick_access_rows_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        let Some((widget, _)) = self.quick.as_ref().and_then(|q| q.widget.as_ref()) else {
            return Vec::new();
        };
        let widget = widget.read(cx);
        widget
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| match row {
                QuickRow::Separator(label) => format!("[{label}]"),
                QuickRow::Item(item) => {
                    let active = if widget.active == Some(ix) { "> " } else { "" };
                    match &item.description {
                        Some(description) => format!("{active}{} ({description})", item.label),
                        None => format!("{active}{}", item.label),
                    }
                }
            })
            .collect()
    }

    /// Types `text` into the quick access input (for tests).
    #[doc(hidden)]
    pub fn quick_access_input_for_tests(&self, cx: &gpui::App) -> Option<Entity<CodeEditor>> {
        let (widget, _) = self.quick.as_ref()?.widget.as_ref()?;
        Some(widget.read(cx).read_input())
    }
}

impl QuickInput {
    fn read_input(&self) -> Entity<CodeEditor> {
        self.input.clone()
    }
}

/// Monaco's help provider: one item per prefix.
fn help_rows() -> Vec<QuickRow> {
    [
        (":", "Go to Line/Column..."),
        ("::", "Go to Offset..."),
        ("@", "Go to Symbol..."),
        ("@:", "Go to Symbol by Category..."),
    ]
    .into_iter()
    .map(|(prefix, help)| {
        QuickRow::Item(QuickItem {
            label: prefix.into(),
            highlights: Vec::new(),
            description: Some(help.into()),
            icon: None,
            strikethrough: false,
            action: QuickAction::SetValue(prefix.to_string()),
        })
    })
    .collect()
}

/// UTF-16 match ranges (from the fuzzy scorer) as byte ranges.
fn utf16_matches_to_bytes(text: &str, matches: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut byte_at = Vec::with_capacity(text.len() + 1);
    for (ix, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            byte_at.push(ix);
        }
    }
    byte_at.push(text.len());
    matches
        .iter()
        .filter_map(|r| Some(*byte_at.get(r.start)?..*byte_at.get(r.end)?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_columns() {
        assert_eq!(visible_column("\tab", 1, 4), 4);
        assert_eq!(visible_column("\tab", 2, 4), 5);
        assert_eq!(column_from_visible("\tab", 4, 4), 1);
        assert_eq!(column_from_visible("\tab", 5, 4), 2);
        assert_eq!(column_from_visible("ab", 1, 4), 1);
    }
}
