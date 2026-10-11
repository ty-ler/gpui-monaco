//! Quick access (Monaco's standalone `quickAccess` and quick input): a
//! filtered list over the editor, opened with a prefix that picks the
//! provider:
//! - `:` goes to a line and column (`::` to a character offset), Ctrl+G,
//! - `@` goes to a symbol (`@:` grouped by kind), Cmd/Ctrl+Shift+O,
//! - `>` runs an editor action (the command palette), F1,
//! - anything else lists the prefixes (help).
//!
//! Ports `gotoLineQuickAccess.ts`, `gotoSymbolQuickAccess.ts`,
//! `editorNavigationQuickAccess.ts`, `commandsQuickAccess.ts` and the word
//! filters of `filters.ts` (MIT); the widget follows VS Code's quick input.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{
    AppContext as _, Context, Entity, FontWeight, HighlightStyle, Hsla, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled, StyledText, Subscription, Task, WeakEntity, Window,
    div, px,
};
use text::PointUtf16;

use crate::actions::{
    Newline, QuickInputFirst, QuickInputLast, QuickInputNext, QuickInputPageNext,
    QuickInputPagePrevious, QuickInputPrevious, RemoveSecondaryCursors,
};
use crate::codicons::{Codicon, icon as codicon};
use crate::cursor::Selection;
use crate::decorations::{DecorationId, DecorationOptions, OverviewRulerLane};
use crate::editor::{CodeEditor, CodeEditorEvent, ViewState};
use crate::model::TextModelEvent;
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
    /// Run the editor action with this id.
    Run(SharedString),
    /// Choose item `.0` of a [`CodeEditor::show_quick_pick`] list.
    Pick(usize),
    /// Select find match `.0` (0-based), for "Go to Match...".
    GoToMatch(usize),
}

/// What happens when a quick pick item is chosen.
type PickHandler = Box<dyn FnOnce(&mut CodeEditor, usize, &mut Window, &mut Context<CodeEditor>)>;

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
    pub icon: Option<(Codicon, Hsla)>,
    pub strikethrough: bool,
    /// The keys that run it, shown on the right.
    pub keybinding: Option<SharedString>,
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
            keybinding: None,
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
    /// The keys bound to each action, looked up when it opened.
    action_keys: HashMap<SharedString, SharedString>,
    /// A [`CodeEditor::show_quick_pick`] list instead of the providers.
    pick: Option<(Vec<SharedString>, Option<PickHandler>)>,
    /// Monaco's "Go to Match..." input box instead of the providers; the
    /// view isn't restored when it closes.
    go_to_match: bool,
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
    /// Wheel movement not yet a whole row.
    wheel_remainder: Pixels,
    width: Pixels,
    /// Monaco's title bar above the input (quick picks).
    title: Option<SharedString>,
    colors: crate::theme::EditorColors,
    _subscriptions: Vec<Subscription>,
}

/// The symbol kind's codicon (Monaco `SymbolKinds.toIcon`), in the theme's
/// `symbolIcon.*Foreground` color.
pub(crate) fn symbol_icon(
    kind: SymbolKind,
    colors: &crate::theme::EditorColors,
) -> (Codicon, Hsla) {
    (kind.icon(), colors.symbol_icons.for_symbol_kind(kind))
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
            let mut input = CodeEditor::new_widget_input(value, theme, window, cx);
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
            wheel_remainder: px(0.),
            visible_rows,
            width,
            title: None,
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

    fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            QuickAction::GoToMatch(index) => {
                if let Some(editor) = self.editor.upgrade() {
                    editor.update(cx, |editor, cx| editor.find_go_to_match(index, cx));
                }
                self.close(true, cx);
            }
            QuickAction::Pick(ix) => {
                let editor = self.editor.clone();
                window.defer(cx, move |window, cx| {
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| {
                            let handler = editor
                                .quick
                                .as_mut()
                                .and_then(|state| state.pick.as_mut())
                                .and_then(|(_, handler)| handler.take());
                            editor.close_quick_access(true, cx);
                            if let Some(handler) = handler {
                                handler(editor, ix, window, cx);
                            }
                        });
                    }
                });
            }
            QuickAction::Run(id) => {
                // Monaco hides the picker, then runs the command.
                crate::editor_actions::push_command_history(id.clone(), cx);
                let editor = self.editor.clone();
                window.defer(cx, move |window, cx| {
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| {
                            editor.close_quick_access(true, cx);
                            editor.run_action(&id, window, cx);
                        });
                    }
                });
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
        // Monaco `list.highlightForeground`, and
        // `quickInputList.focusHighlightForeground` on the focused row.
        let highlight: Hsla = if is_active {
            colors.quick_input_list_focus_highlight_foreground
        } else {
            colors.list_highlight_foreground
        };
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
                colors.focused_list_row(
                    this,
                    colors.quick_input_list_focus_background,
                    colors.quick_input_list_focus_foreground,
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.active = Some(ix);
                    this.accept(window, cx);
                }),
            )
            .children(item.icon.map(|(glyph, color)| codicon(glyph).color(color)))
            .child(StyledText::new(item.label.clone()).with_highlights(label_highlights))
            .children(
                item.description
                    .clone()
                    .map(|description| div().opacity(0.7).text_size(px(12.)).child(description)),
            )
            .children(item.keybinding.clone().map(|keys| {
                // Monaco's `KeybindingLabel` (`keybindingLabel.*`), whose
                // bottom edge is a second, darker border.
                div().ml_auto().pl(px(8.)).flex_none().child(
                    div()
                        .rounded(px(3.))
                        .border_b_1()
                        .border_color(colors.keybinding_label_bottom_border)
                        .child(
                            div()
                                .px(px(3.))
                                .rounded(px(3.))
                                .text_size(px(11.))
                                .bg(colors.keybinding_label_background)
                                .text_color(colors.keybinding_label_foreground)
                                .border_1()
                                .border_color(colors.keybinding_label_border)
                                .child(keys),
                        ),
                )
            }))
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
                    .border_color(colors.picker_group_border)
                    .text_size(px(11.))
                    .text_color(colors.picker_group_foreground)
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
            .bg(colors.quick_input_background)
            .border_1()
            .border_color(colors.widget_border)
            .shadow_md()
            .text_size(px(13.))
            .text_color(colors.quick_input_foreground)
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
            .on_action(cx.listener(|this, _: &Newline, window, cx| this.accept(window, cx)))
            .on_action(cx.listener(|this, _: &RemoveSecondaryCursors, _, cx| this.close(false, cx)))
            .when_some(self.title.clone(), |this, title| {
                this.child(
                    div()
                        .pt(px(6.))
                        .flex()
                        .justify_center()
                        .text_size(px(11.))
                        .bg(colors.quick_input_title_background)
                        .child(title),
                )
            })
            .child(
                div().p(px(6.)).child(
                    div()
                        .h(px(24.))
                        .px(px(4.))
                        .flex()
                        .items_center()
                        .bg(colors.input_background)
                        .border_1()
                        .border_color(colors.input_option_active_border)
                        .child(div().flex_1().min_w_0().child(self.input.clone()))
                        // Monaco's "Toggle Zero-Based Offset" button, shown
                        // in `::` mode.
                        .when(self.value(cx).trim_start().starts_with("::"), |this| {
                            let checked = goto_offset_zero_based(cx);
                            this.child(
                                div()
                                    .id("quick-zero-based-offset")
                                    .flex_none()
                                    .px(px(4.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .cursor_pointer()
                                    .when(checked, |this| {
                                        this.bg(colors.input_option_active_background)
                                            .border_color(colors.input_option_active_border)
                                            .text_color(colors.input_option_active_foreground)
                                    })
                                    .when(!checked, |this| {
                                        this.border_color(gpui::transparent_black())
                                    })
                                    .tooltip(crate::tooltip::tooltip(
                                        "Toggle Zero-Based Offset",
                                        &colors,
                                    ))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            let zero_based = !goto_offset_zero_based(cx);
                                            set_goto_offset_zero_based(zero_based, cx);
                                            this.refresh(cx);
                                        }),
                                    )
                                    .child("0"),
                            )
                        }),
                ),
            )
            .child(
                div()
                    .id("quick-rows")
                    .pb(px(4.))
                    .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll_wheel(event, cx)))
                    .children(rows),
            )
    }
}

impl QuickInput {
    /// Monaco's list scrolls with the wheel, which it consumes.
    fn scroll_wheel(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let row = px(ROW_HEIGHT);
        let delta = event.delta.pixel_delta(row).y + self.wheel_remainder;
        let rows = (delta / row).trunc();
        self.wheel_remainder = delta - row * rows;
        let max = self.rows.len().saturating_sub(self.visible_rows);
        let scroll = (self.scroll as f32 - rows).clamp(0., max as f32) as usize;
        if scroll != self.scroll {
            self.scroll = scroll;
            cx.notify();
        }
    }
}

use gpui::prelude::FluentBuilder as _;

/// Monaco's `gotoLine.useZeroBasedOffset` storage: whether `::offset`
/// counts characters from 0.
#[derive(Default)]
struct ZeroBasedOffset(bool);

impl gpui::Global for ZeroBasedOffset {}

/// Whether Go to Offset (`::`) counts from 0 (Monaco's toggle in the
/// picker, remembered for the app); `false` by default.
pub fn goto_offset_zero_based(cx: &gpui::App) -> bool {
    cx.try_global::<ZeroBasedOffset>().is_some_and(|z| z.0)
}

/// Sets whether Go to Offset (`::`) counts from 0, as the picker's
/// "Toggle Zero-Based Offset" button does.
pub fn set_goto_offset_zero_based(zero_based: bool, cx: &mut gpui::App) {
    cx.set_global(ZeroBasedOffset(zero_based));
}

impl CodeEditor {
    /// Opens quick access with `value` in the input (Monaco
    /// `quickInputService.quickAccess.show(prefix)`): `":"` goes to a line,
    /// `"@"` to a symbol, `""` shows the help.
    pub fn open_quick_access(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_quick_input(value, None, None, false, window, cx);
    }

    /// Monaco's "Go to Match..." input box (`goToMatchFindAction`).
    pub(crate) fn open_go_to_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_quick_input("", None, None, true, window, cx);
    }

    /// Shows `items` in the quick input and calls `on_pick` with the chosen
    /// one's index (Monaco `quickInputService.pick`). Typing filters them.
    pub fn show_quick_pick(
        &mut self,
        title: Option<SharedString>,
        items: Vec<SharedString>,
        on_pick: impl FnOnce(&mut CodeEditor, usize, &mut Window, &mut Context<CodeEditor>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_input(
            "",
            title,
            Some((items, Some(Box::new(on_pick)))),
            false,
            window,
            cx,
        );
    }

    fn open_quick_input(
        &mut self,
        value: &str,
        title: Option<SharedString>,
        pick: Option<(Vec<SharedString>, Option<PickHandler>)>,
        go_to_match: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            view_state: self.save_view_state(cx),
            decorations: Vec::new(),
            symbols: None,
            _symbols_task: None,
            action_keys: self.action_keybindings(window, cx),
            pick,
            go_to_match,
        });
        // The first rows come from here: the widget can't update this
        // editor while it's being updated.
        let rows = self.quick_access_rows(value, cx);
        let editor = cx.entity().downgrade();
        let theme = self.theme.clone();
        let widget = cx
            .new(|cx| QuickInput::new(editor, value, rows, width, visible_rows, theme, window, cx));
        widget.update(cx, |widget, _| widget.title = title);
        if let Some(action) = widget.read(cx).active_action() {
            self.quick_access_preview(&action, cx);
        }
        let id = self.add_overlay_widget(
            OverlayWidget::new(OverlayPosition::TopCenter, widget.clone().into()),
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
        if !accepted && !state.go_to_match {
            self.restore_view_state(&state.view_state, cx);
        }
        self.refocus = true;
        cx.notify();
    }

    /// Previews the active item: reveals it and highlights its lines.
    pub(crate) fn quick_access_preview(&mut self, action: &QuickAction, cx: &mut Context<Self>) {
        // Monaco selects the match while its number is typed, and
        // highlights its line.
        let go_to_match;
        let (selection, highlight) = match action {
            QuickAction::GoTo {
                selection,
                highlight,
            } => (selection, highlight),
            QuickAction::GoToMatch(index) => {
                self.find_go_to_match(*index, cx);
                let range = self.cursors.primary().selection().range();
                go_to_match = (range.start, range);
                (&go_to_match.0, &go_to_match.1)
            }
            _ => {
                // Other items have nothing to preview.
                let old = self
                    .quick
                    .as_mut()
                    .map(|q| std::mem::take(&mut q.decorations))
                    .unwrap_or_default();
                if !old.is_empty() {
                    let owner = Some(cx.entity_id());
                    self.model().update(cx, |model, cx| {
                        model.delta_decorations(owner, &old, Vec::new(), cx)
                    });
                }
                return;
            }
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let start = rope.point_utf16_to_offset(PointUtf16::new(highlight.start.row, 0));
        let end = rope.point_utf16_to_offset(highlight.end);
        let decorations = vec![(
            start..end.max(start),
            DecorationOptions {
                background: Some(self.theme.colors.range_highlight_background),
                border: self.theme.colors.range_highlight_border,
                is_whole_line: true,
                // Monaco `editorOverviewRuler.rangeHighlightForeground`.
                overview_ruler: Some((
                    self.theme.colors.range_highlight_overview,
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
        if let QuickAction::GoTo { .. } = action {
            let point = *selection;
            self.reveal_range(point..point, RevealType::Center, cx);
        }
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
        if self.quick.as_ref().is_some_and(|state| state.go_to_match) {
            return (vec![QuickRow::Item(self.go_to_match_item(value))], None);
        }
        if let Some((items, _)) = self.quick.as_ref().and_then(|state| state.pick.as_ref()) {
            let rows: Vec<QuickRow> = items
                .iter()
                .enumerate()
                .filter_map(|(ix, label)| {
                    let matches = if trimmed.is_empty() {
                        Vec::new()
                    } else {
                        word_filter(trimmed, label)?
                    };
                    let mut item = QuickItem::message(label.clone(), QuickAction::Pick(ix));
                    item.highlights = char_ranges_to_bytes(label, &matches);
                    Some(QuickRow::Item(item))
                })
                .collect();
            return (rows, None);
        }
        if let Some(rest) = trimmed.strip_prefix("::") {
            (vec![QuickRow::Item(self.goto_offset_item(rest, cx))], None)
        } else if let Some(rest) = trimmed.strip_prefix(':') {
            (vec![QuickRow::Item(self.goto_line_item(rest, cx))], None)
        } else if let Some(rest) = trimmed.strip_prefix('@') {
            self.goto_symbol_rows(rest, cx)
        } else if let Some(rest) = trimmed.strip_prefix('>') {
            (self.command_rows(rest.trim(), cx), None)
        } else {
            (help_rows(), None)
        }
    }

    /// The "Go to Match..." input's message: Monaco's placeholder, or its
    /// validation message for a number out of range.
    fn go_to_match_item(&self, value: &str) -> QuickItem {
        let count = self.find_matches_count();
        let placeholder =
            format!("Type a number to go to a specific match (between 1 and {count})");
        if value.is_empty() {
            return QuickItem::message(placeholder, QuickAction::None);
        }
        match self.find_match_index(value) {
            Some(index) => QuickItem::message(placeholder, QuickAction::GoToMatch(index)),
            None => QuickItem::message(
                format!("Please type a number between 1 and {count}"),
                QuickAction::None,
            ),
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

    /// Monaco's `::offset` mode: character offsets from 1, or from 0 with
    /// [`goto_offset_zero_based`].
    fn goto_offset_item(&self, query: &str, cx: &gpui::App) -> QuickItem {
        let rope = self.model().read(cx).buffer().rope().clone();
        let length = rope.len_utf16().0 as i64;
        let tab_size = self.model().read(cx).options().tab_size;
        let zero_based = goto_offset_zero_based(cx);
        let Ok(mut offset) = query.trim().parse::<i64>() else {
            return QuickItem::message(
                if zero_based {
                    format!(
                        "Type a character position to go to (from 0 to {}).",
                        length - 1
                    )
                } else {
                    format!("Type a character position to go to (from 1 to {length}).")
                },
                QuickAction::None,
            );
        };
        if !zero_based {
            offset -= offset.signum();
        }
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
                    icon: Some(symbol_icon(symbol.kind, &self.theme.colors)),
                    strikethrough: symbol.deprecated,
                    keybinding: None,
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
                    let mut row = format!("{active}{}", item.label);
                    if let Some(description) = &item.description {
                        row.push_str(&format!(" ({description})"));
                    }
                    if let Some(keys) = &item.keybinding {
                        row.push_str(&format!(" [{keys}]"));
                    }
                    row
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

impl CodeEditor {
    /// Monaco `AbstractCommandsQuickAccessProvider._getPicks` over the
    /// supported actions: matched by label or alias (or exactly by id),
    /// recently used first, then by label.
    fn command_rows(&self, filter: &str, cx: &gpui::App) -> Vec<QuickRow> {
        let history = |id: &str| crate::editor_actions::command_history(id, cx);
        let mut picks: Vec<(crate::EditorActionInfo, Vec<Range<usize>>)> = Vec::new();
        for action in self.supported_actions(cx) {
            let label = word_filter(filter, &action.label);
            let alias = action.alias.as_deref().and_then(|a| word_filter(filter, a));
            if label.is_some() || alias.is_some() || filter == action.id.as_ref() {
                let highlights = label
                    .map(|m| char_ranges_to_bytes(&action.label, &m))
                    .unwrap_or_default();
                picks.push((action, highlights));
            }
        }
        // Monaco describes commands that share a label with their ids.
        let mut label_counts: HashMap<SharedString, usize> = HashMap::new();
        for (action, _) in &picks {
            *label_counts.entry(action.label.clone()).or_default() += 1;
        }
        picks.sort_by(|(a, _), (b, _)| match (history(&a.id), history(&b.id)) {
            (Some(a), Some(b)) => b.cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.label.to_lowercase().cmp(&b.label.to_lowercase()),
        });
        if picks.is_empty() {
            return vec![QuickRow::Item(QuickItem::message(
                "No matching commands",
                QuickAction::None,
            ))];
        }
        let keys = self.quick.as_ref().map(|q| &q.action_keys);
        let mut rows = Vec::new();
        let mut add_other_separator = false;
        for (ix, (action, highlights)) in picks.into_iter().enumerate() {
            let recent = history(&action.id).is_some();
            if ix == 0 && recent {
                rows.push(QuickRow::Separator("recently used".into()));
                add_other_separator = true;
            }
            if add_other_separator && !recent {
                rows.push(QuickRow::Separator("other commands".into()));
                add_other_separator = false;
            }
            let duplicate = label_counts.get(&action.label).is_some_and(|n| *n > 1);
            rows.push(QuickRow::Item(QuickItem {
                label: action.label.clone(),
                highlights,
                description: duplicate.then(|| action.id.clone()),
                icon: None,
                strikethrough: false,
                keybinding: keys.and_then(|keys| keys.get(&action.id).cloned()),
                action: QuickAction::Run(action.id),
            }));
        }
        rows
    }
}

/// Monaco's help provider: one item per prefix.
fn help_rows() -> Vec<QuickRow> {
    [
        (":", "Go to Line/Column..."),
        ("::", "Go to Offset..."),
        ("@", "Go to Symbol..."),
        ("@:", "Go to Symbol by Category..."),
        (">", "Show And Run Commands"),
    ]
    .into_iter()
    .map(|(prefix, help)| {
        QuickRow::Item(QuickItem {
            label: prefix.into(),
            highlights: Vec::new(),
            description: Some(help.into()),
            icon: None,
            strikethrough: false,
            keybinding: None,
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

/// Monaco `AbstractCommandsQuickAccessProvider.WORD_FILTER`:
/// `or(matchesPrefix, matchesWords, matchesContiguousSubString)`, as char
/// ranges of `target`.
fn word_filter(word: &str, target: &str) -> Option<Vec<Range<usize>>> {
    let word: Vec<char> = word.chars().map(lower).collect();
    let target: Vec<char> = target.chars().map(lower).collect();
    matches_prefix(&word, &target)
        .or_else(|| matches_words(&word, &target, false))
        .or_else(|| matches_contiguous_substring(&word, &target))
}

/// Lowercases one char (Monaco's `tryNormalizeToBase`, without removing
/// accents).
fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Monaco `matchesPrefix` (case-insensitive).
#[allow(clippy::single_range_in_vec_init)]
fn matches_prefix(word: &[char], target: &[char]) -> Option<Vec<Range<usize>>> {
    if target.is_empty() || target.len() < word.len() || !target.starts_with(word) {
        return None;
    }
    Some(if word.is_empty() {
        Vec::new()
    } else {
        vec![0..word.len()]
    })
}

/// Monaco `matchesContiguousSubString`.
#[allow(clippy::single_range_in_vec_init)]
fn matches_contiguous_substring(word: &[char], target: &[char]) -> Option<Vec<Range<usize>>> {
    if word.len() > target.len() {
        return None;
    }
    let start = (0..=target.len() - word.len()).find(|&i| target[i..].starts_with(word))?;
    Some(vec![start..start + word.len()])
}

fn is_word_separator(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r') || "()[]{}<>`'\"-/;:,.?!".contains(c)
}

fn characters_match(a: char, b: char) -> bool {
    a == b || (is_word_separator(a) && is_word_separator(b))
}

/// Monaco `nextWord`: the next index that starts a word or is a separator.
fn next_word(target: &[char], start: usize) -> usize {
    (start..target.len())
        .find(|&i| is_word_separator(target[i]) || (i > 0 && is_word_separator(target[i - 1])))
        .unwrap_or(target.len())
}

/// Monaco `join`: prepends `head`, merging it with an adjacent first match.
fn join(head: Range<usize>, mut tail: Vec<Range<usize>>) -> Vec<Range<usize>> {
    match tail.first_mut() {
        None => tail.push(head),
        Some(first) if first.start == head.end => first.start = head.start,
        Some(_) => tail.insert(0, head),
    }
    tail
}

type WordsMemo = HashMap<(usize, usize), Option<Vec<Range<usize>>>>;

/// Monaco `matchesWords`: each char of `word` matches the start of a word in
/// `target` (or continues the previous match), e.g. "gp" in "Git: Pull".
fn matches_words(word: &[char], target: &[char], contiguous: bool) -> Option<Vec<Range<usize>>> {
    if target.is_empty() {
        return None;
    }
    let mut memo = WordsMemo::new();
    let mut target_index = 0;
    while target_index < target.len() {
        if let Some(result) = matches_words_at(word, target, 0, target_index, contiguous, &mut memo)
        {
            return Some(result);
        }
        target_index = next_word(target, target_index + 1);
    }
    None
}

fn matches_words_at(
    word: &[char],
    target: &[char],
    word_index: usize,
    target_index: usize,
    contiguous: bool,
    memo: &mut WordsMemo,
) -> Option<Vec<Range<usize>>> {
    if word_index == word.len() {
        return Some(Vec::new());
    }
    if target_index == target.len() {
        return None;
    }
    if let Some(cached) = memo.get(&(word_index, target_index)) {
        return cached.clone();
    }
    let computed = (|| {
        if !characters_match(word[word_index], target[target_index]) {
            return None;
        }
        let mut next = target_index + 1;
        let mut result = matches_words_at(word, target, word_index + 1, next, contiguous, memo);
        if !contiguous {
            while result.is_none() {
                next = next_word(target, next);
                if next >= target.len() {
                    break;
                }
                result = matches_words_at(word, target, word_index + 1, next, contiguous, memo);
                next += 1;
            }
        }
        let result = result?;
        // Matching separators aren't highlighted.
        if word[word_index] != target[target_index] {
            return Some(result);
        }
        Some(join(target_index..target_index + 1, result))
    })();
    memo.insert((word_index, target_index), computed.clone());
    computed
}

/// Char ranges of `text` as byte ranges.
fn char_ranges_to_bytes(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut byte_at: Vec<usize> = text.char_indices().map(|(ix, _)| ix).collect();
    byte_at.push(text.len());
    ranges
        .iter()
        .map(|r| byte_at[r.start.min(byte_at.len() - 1)]..byte_at[r.end.min(byte_at.len() - 1)])
        .collect()
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn words(word: &str, target: &str) -> Option<Vec<Range<usize>>> {
        let word: Vec<char> = word.chars().map(lower).collect();
        let target: Vec<char> = target.chars().map(lower).collect();
        matches_words(&word, &target, false)
    }

    /// From VS Code's `filters.test.ts` (`matchesWords`).
    #[test]
    fn matches_words_like_monaco() {
        assert_eq!(words("alpha", "alpha"), Some(vec![0..5]));
        assert_eq!(words("alpha", "alphasomething"), Some(vec![0..5]));
        assert_eq!(words("alpha", "alp"), None);
        assert_eq!(words("a", "alpha"), Some(vec![0..1]));
        assert_eq!(words("x", "alpha"), None);
        assert_eq!(words("AlPh", "alPHA"), Some(vec![0..4]));
        assert!(words("Debug Console", "Open: Debug Console").is_some());
        assert_eq!(words("gp", "Git: Pull"), Some(vec![0..1, 5..6]));
        assert_eq!(words("g p", "Git: Pull"), Some(vec![0..1, 5..6]));
        assert_eq!(words("gipu", "Git: Pull"), Some(vec![0..2, 5..7]));
        assert_eq!(
            words("gp", "Category: Git: Pull"),
            Some(vec![10..11, 15..16])
        );
        assert_eq!(
            words("gipu", "Category: Git: Pull"),
            Some(vec![10..12, 15..17])
        );
        assert_eq!(words("it", "Git: Pull"), None);
        assert_eq!(words("ll", "Git: Pull"), None);
        assert_eq!(words("C++", "C/C++: command"), Some(vec![2..5]));
        assert_eq!(words(".", ":"), Some(vec![]));
        assert_eq!(words(".", "."), Some(vec![0..1]));
        assert!(words("bar", "foo-bar").is_some());
        assert!(words("foo bar", "foo-bar").is_some());
        assert!(words("foo bar", "123 foo-bar 456").is_some());
        assert!(words("foo-bar", "foo bar").is_some());
        assert!(words("foo:bar", "foo:bar").is_some());
    }

    #[test]
    fn word_filter_tries_prefix_words_then_substring() {
        assert_eq!(word_filter("fold a", "Fold All"), Some(vec![0..6]));
        assert_eq!(word_filter("fa", "Fold All"), Some(vec![0..1, 5..6]));
        assert_eq!(word_filter("old", "Fold All"), Some(vec![1..4]));
        assert_eq!(word_filter("", "Fold All"), Some(vec![]));
        assert_eq!(word_filter("xyz", "Fold All"), None);
        assert_eq!(
            char_ranges_to_bytes("äb c", &[1..2, 3..4]),
            vec![2..3, 4..5]
        );
    }

    #[test]
    fn visible_columns() {
        assert_eq!(visible_column("\tab", 1, 4), 4);
        assert_eq!(visible_column("\tab", 2, 4), 5);
        assert_eq!(column_from_visible("\tab", 4, 4), 1);
        assert_eq!(column_from_visible("\tab", 5, 4), 2);
        assert_eq!(column_from_visible("ab", 1, 4), 1);
    }
}
