//! The context menu (Monaco `ContextMenuController` and the
//! `EditorContext` menu): right-click or Shift+F10 shows the editor's
//! actions in groups, with host actions from [`CodeEditor::add_action`]
//! that set a context menu group.
//!
//! Ports `contextmenu.ts` (MIT); the items and their groups and order come
//! from the actions' `contextMenuOpts` and `MenuRegistry` entries.

use std::collections::BTreeMap;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, WeakEntity, Window, div, px,
};
use text::PointUtf16;

use crate::actions::{
    ContextMenuFirst, ContextMenuHide, ContextMenuLast, ContextMenuNext, ContextMenuPrevious,
    ContextMenuSelect, ContextMenuSubmenuClose, ContextMenuSubmenuOpen,
};
use crate::codicons::{Codicon, icon as codicon};
use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::keybinding_label::ui_text_width;
use crate::options::LocationKind;
use crate::providers::{Command, Location};
use crate::widgets::{OverlayPosition, WidgetId};

/// The key context of the context menu.
pub const CONTEXT_MENU_KEY_CONTEXT: &str = "EditorContextMenu";

/// Monaco's menu row height.
const ROW_HEIGHT: f32 = 24.;
const SEPARATOR_HEIGHT: f32 = 9.;
/// The menu's font size, and its keybindings'.
const MENU_FONT_SIZE: f32 = 13.;
const KEYBINDING_FONT_SIZE: f32 = 12.;
/// The label's padding on each side (room for the check mark on the left)
/// and the keybinding's on its left.
const LABEL_PADDING: f32 = 20.;
const KEYBINDING_PADDING: f32 = 16.;
/// `.context-view .monaco-menu { min-width: 130px }`; at most the window
/// less this on each side.
const MENU_MIN_WIDTH: f32 = 130.;
const MENU_WINDOW_MARGIN: f32 = 8.;

/// Whether a menu item shows.
type When = fn(&CodeEditor, &App) -> bool;

/// What a menu item runs.
#[derive(Clone, Debug, PartialEq)]
enum Run {
    /// A GPUI action, by name.
    Action(&'static str),
    /// An editor action, by id ([`CodeEditor::run_action`]).
    EditorAction(SharedString),
    /// A navigation command from an inlay hint part's location.
    Navigate(LocationKind, Location),
    /// A command (an inlay hint part's).
    Command(Command),
}

/// The navigation commands of Monaco's inlay hint location menu: the
/// `SymbolNavigationAction`s in the `EditorContext` menu, in the order
/// they're registered (`showGoToContextMenu`).
const LOCATION_ITEMS: &[(&str, LocationKind)] = &[
    ("Go to Definition", LocationKind::Definition),
    ("Go to Declaration", LocationKind::Declaration),
    ("Go to Type Definition", LocationKind::TypeDefinition),
    ("Go to Implementations", LocationKind::Implementation),
    ("Go to References", LocationKind::References),
];

/// One of Monaco's `EditorContext` menu items: group, order, label, what
/// it runs, and when it shows.
struct BuiltinItem(&'static str, f32, &'static str, &'static str, When);

fn always(_: &CodeEditor, _: &App) -> bool {
    true
}

fn writable(editor: &CodeEditor, _: &App) -> bool {
    !editor.options.read_only
}

fn has_selection(editor: &CodeEditor, cx: &App) -> bool {
    writable(editor, cx) && editor.selections().iter().any(|s| !s.is_empty())
}

/// Whether the built-in action `id` can run (its precondition holds, which
/// Monaco folds into the menu item's `when`).
fn supported(editor: &CodeEditor, id: &str, cx: &App) -> bool {
    editor.action_enabled(id, cx)
}

macro_rules! when_supported {
    ($name:ident, $id:literal) => {
        fn $name(editor: &CodeEditor, cx: &App) -> bool {
            supported(editor, $id, cx)
        }
    };
}

when_supported!(can_go_to_definition, "editor.action.revealDefinition");
when_supported!(can_go_to_declaration, "editor.action.revealDeclaration");
when_supported!(
    can_go_to_type_definition,
    "editor.action.goToTypeDefinition"
);
when_supported!(can_go_to_implementation, "editor.action.goToImplementation");
when_supported!(can_go_to_references, "editor.action.goToReferences");
when_supported!(can_rename, "editor.action.rename");
when_supported!(can_format, "editor.action.formatDocument");
when_supported!(can_go_to_symbol, "editor.action.quickOutline");

/// Monaco's Refactor... item: a provider declares refactorings
/// (`contextKeyForSupportedActions(Refactor)`).
fn can_refactor(editor: &CodeEditor, cx: &App) -> bool {
    supported(editor, "editor.action.refactor", cx)
        && editor.when("supportedCodeAction_refactor", cx)
}

/// Monaco's Source Action... item: a provider declares source actions.
fn can_source_action(editor: &CodeEditor, cx: &App) -> bool {
    supported(editor, "editor.action.sourceAction", cx)
        && editor.when("supportedCodeAction_source", cx)
}

fn can_format_selection(editor: &CodeEditor, cx: &App) -> bool {
    has_selection(editor, cx) && supported(editor, "editor.action.formatSelection", cx)
}

/// The `EditorContext` menu.
#[rustfmt::skip]
static ITEMS: &[BuiltinItem] = &[
    BuiltinItem("navigation", 1.1, "Go to Definition", "code_editor::GoToDefinition", can_go_to_definition),
    BuiltinItem("navigation", 1.3, "Go to Declaration", "code_editor::GoToDeclaration", can_go_to_declaration),
    BuiltinItem("navigation", 1.4, "Go to Type Definition", "code_editor::GoToTypeDefinition", can_go_to_type_definition),
    BuiltinItem("navigation", 1.45, "Go to Implementations", "code_editor::GoToImplementation", can_go_to_implementation),
    BuiltinItem("navigation", 1.45, "Go to References", "code_editor::GoToReferences", can_go_to_references),
    BuiltinItem("navigation", 3., "Go to Symbol...", "code_editor::QuickOutline", can_go_to_symbol),
    BuiltinItem("1_modification", 1.1, "Rename Symbol", "code_editor::Rename", can_rename),
    BuiltinItem("1_modification", 1.2, "Change All Occurrences", "code_editor::SelectAllOccurrences", writable),
    BuiltinItem("1_modification", 1.3, "Format Document", "code_editor::FormatDocument", can_format),
    BuiltinItem("1_modification", 1.31, "Format Selection", "code_editor::FormatSelection", can_format_selection),
    BuiltinItem("1_modification", 2., "Refactor...", "code_editor::Refactor", can_refactor),
    BuiltinItem("1_modification", 2.1, "Source Action...", "code_editor::SourceAction", can_source_action),
    BuiltinItem("9_cutcopypaste", 1., "Cut", "code_editor::Cut", writable),
    BuiltinItem("9_cutcopypaste", 2., "Copy", "code_editor::Copy", always),
    BuiltinItem("9_cutcopypaste", 4., "Paste", "code_editor::Paste", writable),
    BuiltinItem("z_commands", 1., "Command Palette", "code_editor::QuickCommand", always),
];

/// The "Peek" submenu (Monaco `EditorContextPeek`, in `navigation` at 100).
#[rustfmt::skip]
static PEEK_ITEMS: &[BuiltinItem] = &[
    BuiltinItem("peek", 2., "Peek Definition", "code_editor::PeekDefinition", can_go_to_definition),
    BuiltinItem("peek", 3., "Peek Declaration", "code_editor::PeekDeclaration", can_go_to_declaration),
    BuiltinItem("peek", 4., "Peek Type Definition", "code_editor::PeekTypeDefinition", can_go_to_type_definition),
    BuiltinItem("peek", 5., "Peek Implementations", "code_editor::PeekImplementation", can_go_to_implementation),
    BuiltinItem("peek", 6., "Peek References", "code_editor::PeekReferences", can_go_to_references),
];

/// A row of the menu.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Item {
        label: SharedString,
        keybinding: Option<SharedString>,
        run: Run,
        /// Monaco's `checked` actions (`menu-selection` before the label).
        checked: bool,
    },
    Submenu {
        label: SharedString,
        entries: Vec<Entry>,
    },
    Separator,
}

impl Entry {
    fn label(&self) -> Option<&SharedString> {
        match self {
            Entry::Item { label, .. } | Entry::Submenu { label, .. } => Some(label),
            Entry::Separator => None,
        }
    }

    fn item(label: SharedString, keybinding: Option<SharedString>, run: Run) -> Self {
        Entry::Item {
            label,
            keybinding,
            run,
            checked: false,
        }
    }
}

/// An item before grouping: group, order, entry.
type Grouped = (SharedString, f32, Entry);

/// Monaco `_getMenuActions`: groups in order (`navigation` first, then by
/// name), items by order then label, separators between groups.
fn build_menu(mut items: Vec<Grouped>) -> Vec<Entry> {
    let mut groups: BTreeMap<(bool, SharedString), Vec<(f32, Entry)>> = BTreeMap::new();
    for (group, order, entry) in items.drain(..) {
        groups
            .entry((group.as_ref() != "navigation", group))
            .or_default()
            .push((order, entry));
    }
    let mut entries = Vec::new();
    for (_, mut group) in groups {
        group.sort_by(|(a, ea), (b, eb)| a.total_cmp(b).then_with(|| ea.label().cmp(&eb.label())));
        if !entries.is_empty() {
            entries.push(Entry::Separator);
        }
        entries.extend(group.into_iter().map(|(_, entry)| entry));
    }
    entries
}

/// The context menu.
pub struct ContextMenu {
    editor: WeakEntity<CodeEditor>,
    entries: Vec<Entry>,
    active: Option<usize>,
    /// The open submenu and its active row.
    submenu: Option<(usize, Option<usize>)>,
    /// Monaco's submenu show delay: a hovered submenu row opens after
    /// 250ms.
    submenu_timer: Option<Task<()>>,
    focus: FocusHandle,
    colors: crate::theme::EditorColors,
    /// Where the menu is in the window.
    origin: Point<Pixels>,
    /// Its measured width.
    width: Pixels,
    /// The window's height: the list scrolls past it.
    max_height: Pixels,
}

/// Monaco's submenu `showDelayer`.
const SUBMENU_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

impl ContextMenu {
    fn items(entries: &[Entry]) -> Vec<usize> {
        entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !matches!(e, Entry::Separator))
            .map(|(ix, _)| ix)
            .collect()
    }

    fn move_active(&mut self, delta: isize, cx: &mut Context<Self>) {
        let step = |entries: &[Entry], active: Option<usize>| {
            let items = Self::items(entries);
            if items.is_empty() {
                return None;
            }
            let count = items.len() as isize;
            let current = active.and_then(|a| items.iter().position(|ix| *ix == a));
            let next = match (current, delta.abs()) {
                (Some(current), 1) => (current as isize + delta).rem_euclid(count),
                (None, 1) if delta > 0 => 0,
                (None, 1) => count - 1,
                (_, _) if delta > 0 => count - 1,
                (_, _) => 0,
            };
            Some(items[next as usize])
        };
        match &mut self.submenu {
            Some((ix, active)) => {
                if let Entry::Submenu { entries, .. } = &self.entries[*ix] {
                    *active = step(entries, *active);
                }
            }
            None => self.active = step(&self.entries, self.active),
        }
        cx.notify();
    }

    fn open_submenu(&mut self, cx: &mut Context<Self>) {
        self.submenu_timer = None;
        if let Some(ix) = self.active
            && let Entry::Submenu { entries, .. } = &self.entries[ix]
        {
            self.submenu = Some((ix, Self::items(entries).first().copied()));
            cx.notify();
        }
    }

    fn select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = match self.submenu {
            Some((ix, Some(sub))) => match &self.entries[ix] {
                Entry::Submenu { entries, .. } => entries.get(sub).cloned(),
                _ => None,
            },
            Some((_, None)) => None,
            None => self.active.and_then(|ix| self.entries.get(ix).cloned()),
        };
        match entry {
            Some(Entry::Item { run, .. }) => self.run(run, window, cx),
            Some(Entry::Submenu { .. }) => self.open_submenu(cx),
            _ => {}
        }
    }

    fn run(&mut self, run: Run, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        // Monaco hides the menu first.
        window.defer(cx, move |window, cx| {
            editor.update(cx, |editor, cx| {
                editor.hide_context_menu(cx);
                window.focus(&editor.focus_handle, cx);
                match run {
                    Run::Action(name) => {
                        if let Ok(action) = cx.build_action(name, None) {
                            window.dispatch_action(action, cx);
                        }
                    }
                    Run::EditorAction(id) => {
                        editor.run_action(&id, window, cx);
                    }
                    Run::Navigate(kind, location) => {
                        editor.run_inlay_hint_location(kind, &location, cx);
                    }
                    Run::Command(command) => {
                        editor.run_inlay_hint_command(&command, cx);
                    }
                }
            });
        });
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            cx.defer(move |cx| editor.update(cx, |editor, cx| editor.hide_context_menu(cx)));
        }
    }

    /// The mouse is over row `ix` of the top menu: it becomes active, and
    /// a submenu row opens its submenu after Monaco's delay.
    fn hover_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.active == Some(ix) && (self.submenu_timer.is_some() || self.submenu.is_some()) {
            return;
        }
        self.active = Some(ix);
        self.submenu = None;
        self.submenu_timer = None;
        if matches!(self.entries[ix], Entry::Submenu { .. }) {
            self.submenu_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SUBMENU_DELAY).await;
                this.update(cx, |this, cx| {
                    if this.active == Some(ix) {
                        this.open_submenu(cx);
                    }
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn render_entries(
        &self,
        entries: &[Entry],
        active: Option<usize>,
        submenu_of: Option<usize>,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = &self.colors;
        div()
            .w(width)
            .py(px(4.))
            .flex()
            .flex_col()
            // Monaco's menu colors (`menu.*`); the border only shows in
            // high contrast.
            .bg(colors.menu_background)
            .border_1()
            .border_color(colors.menu_border.unwrap_or_else(gpui::transparent_black))
            .rounded(px(5.))
            // VS Code's menu shadow, `0 2px 8px widget.shadow` (none in
            // high contrast).
            .shadow(crate::widgets::card::widget_shadow(colors.widget_shadow))
            // The wheel over the menu scrolls it, never the text: Monaco's
            // menu is outside the editor, so its wheel doesn't hide it.
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .text_size(px(MENU_FONT_SIZE))
            .text_color(colors.menu_foreground)
            .children(entries.iter().enumerate().map(|(ix, entry)| {
                let is_active = active == Some(ix);
                match entry {
                    Entry::Separator => div()
                        .h(px(SEPARATOR_HEIGHT))
                        .flex()
                        .items_center()
                        .px(px(8.))
                        .child(
                            div()
                                .w_full()
                                .h(px(1.))
                                .bg(colors.menu_separator_background),
                        )
                        .into_any_element(),
                    Entry::Item { label, .. } | Entry::Submenu { label, .. } => {
                        let (keybinding, checked) = match entry {
                            Entry::Item {
                                keybinding,
                                checked,
                                ..
                            } => (keybinding.clone(), *checked),
                            _ => (None, false),
                        };
                        let is_submenu = matches!(entry, Entry::Submenu { .. });
                        let id: SharedString = match submenu_of {
                            Some(parent) => format!("context-menu-{parent}-{ix}").into(),
                            None => format!("context-menu-{ix}").into(),
                        };
                        let selector = id.to_string();
                        div()
                            .id(id)
                            .debug_selector(move || selector)
                            .relative()
                            .h(px(ROW_HEIGHT))
                            .mx(px(4.))
                            .px(px(LABEL_PADDING))
                            .flex()
                            .items_center()
                            .rounded(px(3.))
                            .cursor_pointer()
                            .whitespace_nowrap()
                            .when_active(is_active, colors)
                            // `.menu-item-check`: `menu-selection` in the
                            // label's left padding.
                            .children(checked.then(|| {
                                div()
                                    .absolute()
                                    .left_0()
                                    .w(px(LABEL_PADDING))
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(codicon(Codicon::MenuSelection))
                            }))
                            // One line, cut with an ellipsis when the menu
                            // is at its widest.
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(label.clone()),
                            )
                            .children(keybinding.map(|keys| {
                                div()
                                    .flex_none()
                                    .pl(px(KEYBINDING_PADDING))
                                    .opacity(0.8)
                                    .text_size(px(KEYBINDING_FONT_SIZE))
                                    .child(keys)
                            }))
                            .children(is_submenu.then(|| {
                                // `menu-submenu`: `chevron-right`.
                                div()
                                    .flex_none()
                                    .pl(px(8.))
                                    .child(codicon(Codicon::MenuSubmenu))
                            }))
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if !*hovered {
                                    return;
                                }
                                match submenu_of {
                                    Some(parent) => {
                                        this.submenu = Some((parent, Some(ix)));
                                        cx.notify();
                                    }
                                    None => this.hover_row(ix, cx),
                                }
                            }))
                            // VS Code's menu items run on mouse up, so a
                            // press can be dragged off the item to cancel.
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    match submenu_of {
                                        Some(parent) => this.submenu = Some((parent, Some(ix))),
                                        None => {
                                            this.active = Some(ix);
                                            this.submenu = None;
                                        }
                                    }
                                    this.select(window, cx);
                                }),
                            )
                            .into_any_element()
                    }
                }
            }))
    }
}

trait ActiveRow {
    fn when_active(self, active: bool, colors: &crate::theme::EditorColors) -> Self;
}

impl<E: Styled> ActiveRow for E {
    fn when_active(self, active: bool, colors: &crate::theme::EditorColors) -> Self {
        if active {
            let row = self
                .bg(colors.menu_selection_background)
                .text_color(colors.menu_selection_foreground);
            // High contrast outlines the row (`menu.selectionBorder`).
            match colors.menu_selection_border {
                Some(border) => row.border_1().border_color(border),
                None => row,
            }
        } else {
            self
        }
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.entries.clone();
        let viewport = window.viewport_size();
        let menu = div()
            .id("context-menu-list")
            .max_h(self.max_height)
            .overflow_y_scroll()
            .child(self.render_entries(&entries, self.active, None, self.width, cx));
        let submenu = self.submenu.and_then(|(ix, active)| match &entries[ix] {
            Entry::Submenu { entries, .. } => {
                // Beside its row, on the side with room for it (Monaco
                // `layout` with `LayoutAnchorPosition.Before` when it
                // doesn't fit after), kept in the window vertically.
                let width = menu_width(entries, viewport.width, window);
                let height = menu_height(entries);
                let row_top = self.origin.y + entries_offset(&self.entries, ix) - px(4.);
                let top = row_top.min(viewport.height - height).max(px(0.)) - self.origin.y;
                let left =
                    submenu_left(self.origin.x, self.width, width, viewport.width) - self.origin.x;
                Some(
                    div()
                        .absolute()
                        .left(left)
                        .top(top)
                        .child(self.render_entries(entries, active, Some(ix), width, cx)),
                )
            }
            _ => None,
        });
        div()
            .relative()
            .track_focus(&self.focus)
            .key_context(CONTEXT_MENU_KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &ContextMenuNext, _, cx| this.move_active(1, cx)))
            .on_action(cx.listener(|this, _: &ContextMenuPrevious, _, cx| this.move_active(-1, cx)))
            .on_action(cx.listener(|this, _: &ContextMenuFirst, _, cx| {
                this.move_active(-(isize::MAX / 2), cx)
            }))
            .on_action(
                cx.listener(|this, _: &ContextMenuLast, _, cx| {
                    this.move_active(isize::MAX / 2, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ContextMenuSelect, window, cx| this.select(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ContextMenuSubmenuOpen, _, cx| this.open_submenu(cx)))
            .on_action(cx.listener(|this, _: &ContextMenuSubmenuClose, _, cx| {
                this.submenu_timer = None;
                if this.submenu.take().is_some() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ContextMenuHide, _, cx| this.hide(cx)))
            .child(menu)
            .children(submenu)
    }
}

/// The submenu's left edge in the window: after its parent menu when it
/// fits, else before it (Monaco's submenu `layout`), and in the window.
fn submenu_left(
    parent_left: Pixels,
    parent_width: Pixels,
    width: Pixels,
    window: Pixels,
) -> Pixels {
    let after = parent_left + parent_width - px(4.);
    if after + width <= window {
        after
    } else {
        (parent_left - width + px(4.)).max(px(0.))
    }
}

/// The menu's measured width (Monaco's menu sizes to its widest row): the
/// row margins and label padding, the label, the keybinding and the
/// submenu indicator, at least Monaco's `min-width: 130px` and at most the
/// window less a margin on each side.
fn menu_width(entries: &[Entry], window_width: Pixels, window: &mut Window) -> Pixels {
    let mut widest = px(0.);
    for entry in entries {
        let width = match entry {
            Entry::Separator => continue,
            Entry::Item {
                label, keybinding, ..
            } => {
                let mut width =
                    ui_text_width(label, px(MENU_FONT_SIZE), FontWeight::NORMAL, window);
                if let Some(keys) = keybinding {
                    width += px(KEYBINDING_PADDING)
                        + ui_text_width(keys, px(KEYBINDING_FONT_SIZE), FontWeight::NORMAL, window);
                }
                width
            }
            Entry::Submenu { label, .. } => {
                ui_text_width(label, px(MENU_FONT_SIZE), FontWeight::NORMAL, window) + px(8. + 16.)
            }
        };
        widest = widest.max(width);
    }
    // Row margins, label padding and the border.
    let width = (widest + px(4. * 2. + LABEL_PADDING * 2. + 2.)).ceil();
    width
        .max(px(MENU_MIN_WIDTH))
        .min((window_width - px(MENU_WINDOW_MARGIN * 2.)).max(px(MENU_MIN_WIDTH)))
}

/// The top of entry `ix` within the menu.
fn entries_offset(entries: &[Entry], ix: usize) -> Pixels {
    px(4.)
        + entries[..ix]
            .iter()
            .map(|e| match e {
                Entry::Separator => px(SEPARATOR_HEIGHT),
                _ => px(ROW_HEIGHT),
            })
            .fold(px(0.), |a, b| a + b)
}

fn menu_height(entries: &[Entry]) -> Pixels {
    entries_offset(entries, entries.len()) + px(4.)
}

/// The editor's context menu state.
#[derive(Default)]
pub(crate) struct ContextMenuState {
    menu: Option<(Entity<ContextMenu>, WidgetId)>,
    /// Shift+F10 revealed the cursor: show the menu below it once laid out.
    pub(crate) show_at_cursor: bool,
}

impl CodeEditor {
    /// The menu's entries (Monaco `_getMenuActions`), with keybindings.
    fn context_menu_entries(&self, window: &Window, cx: &App) -> Vec<Entry> {
        let keys = |name: &str| -> Option<SharedString> {
            let action = cx.build_action(name, None).ok()?;
            crate::keybinding_label::command_label(
                action.as_ref(),
                None,
                &self.focus_handle,
                window,
            )
        };
        let builtin = |items: &[BuiltinItem]| -> Vec<Grouped> {
            items
                .iter()
                .filter(|item| (item.4)(self, cx))
                .map(|item| {
                    (
                        item.0.into(),
                        item.1,
                        Entry::item(item.2.into(), keys(item.3), Run::Action(item.3)),
                    )
                })
                .collect()
        };
        let mut items = builtin(ITEMS);
        let peek: Vec<Entry> = build_menu(builtin(PEEK_ITEMS));
        if !peek.is_empty() {
            items.push((
                "navigation".into(),
                100.,
                Entry::Submenu {
                    label: "Peek".into(),
                    entries: peek,
                },
            ));
        }
        // Host actions with a group (Monaco `contextMenuGroupId`), this
        // editor's then the global ones it doesn't shadow.
        let global = crate::editor_actions::global_actions(cx);
        let global = global
            .iter()
            .filter(|g| !self.added_actions.iter().any(|a| a.id == g.id));
        for action in self.added_actions.iter().chain(global) {
            let Some(group) = &action.context_menu_group_id else {
                continue;
            };
            if !action.precondition.as_ref().is_none_or(|p| p(self, cx)) {
                continue;
            }
            let binding = window.highest_precedence_binding_for_action_in(
                &crate::actions::RunAction {
                    id: action.id.to_string(),
                },
                &self.focus_handle,
            );
            items.push((
                group.clone(),
                action.context_menu_order,
                Entry::item(
                    action.label.clone(),
                    binding.map(|b| crate::keybinding_label::binding_label(&b)),
                    Run::EditorAction(action.id.clone()),
                ),
            ));
        }
        build_menu(items)
    }

    /// Shows the context menu at `position` (window coordinates), or at the
    /// cursor (Monaco `showContextMenu`, Shift+F10).
    pub fn show_context_menu(
        &mut self,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_context_menu(cx);
        if !self.options.context_menu {
            return;
        }
        // Monaco `_doShowContextMenu` without a mouse event: reveals the
        // cursor, renders, then anchors below it. A cursor out of view shows
        // the menu once the next frame has laid it out.
        if position.is_none() && !self.cursor_fully_visible() {
            let cursor = self.cursors.primary().position;
            self.reveal_range_with_scroll_type(
                cursor..cursor,
                crate::view::reveal::RevealType::Simple,
                crate::view::reveal::ScrollType::Immediate,
                cx,
            );
            self.context_menu.show_at_cursor = true;
            cx.notify();
            return;
        }
        let entries = self.context_menu_entries(window, cx);
        self.show_menu(entries, position, None, window, cx);
    }

    /// After a frame laid out the cursor Shift+F10 revealed: the menu below
    /// it.
    pub(crate) fn show_pending_context_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if std::mem::take(&mut self.context_menu.show_at_cursor) && self.options.context_menu {
            let entries = self.context_menu_entries(window, cx);
            self.show_menu(entries, None, None, window, cx);
        }
    }

    /// Whether the primary cursor's line is laid out inside the text area.
    fn cursor_fully_visible(&self) -> bool {
        let Some(layout) = self.layout.as_ref() else {
            return false;
        };
        let cursor = self.cursors.primary().position;
        layout.line_for_point(cursor).is_some_and(|line| {
            let top = layout.line_top(line.view_line);
            top >= layout.text_bounds.top()
                && top + layout.line_height_of(line.view_line) <= layout.text_bounds.bottom()
        })
    }

    /// Monaco's sticky scroll context menu (`MenuId.StickyScrollContext`):
    /// the Toggle Editor Sticky Scroll action, rendered with its toggled
    /// title, "Sticky Scroll", checked.
    pub(crate) fn show_sticky_scroll_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_context_menu(cx);
        let entries = vec![Entry::Item {
            label: "Sticky Scroll".into(),
            keybinding: None,
            run: Run::Action("code_editor::ToggleStickyScroll"),
            checked: self.options.sticky_scroll.enabled,
        }];
        self.show_menu(entries, Some(position), None, window, cx);
    }

    /// Monaco `showGoToContextMenu`: the navigation commands for an inlay
    /// hint part's `location`, then the part's command, at `anchor`, with
    /// the first item active (`autoSelectFirstItem`).
    pub(crate) fn show_inlay_hint_location_menu(
        &mut self,
        location: Location,
        command: Option<Command>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_context_menu(cx);
        let mut entries: Vec<Entry> = LOCATION_ITEMS
            .iter()
            .map(|(label, kind)| {
                Entry::item(
                    (*label).into(),
                    None,
                    Run::Navigate(*kind, location.clone()),
                )
            })
            .collect();
        if let Some(command) = command {
            entries.push(Entry::Separator);
            entries.push(Entry::item(
                command.title.clone(),
                None,
                Run::Command(command),
            ));
        }
        self.show_menu(entries, Some(anchor), Some(0), window, cx);
    }

    /// Shows a menu of `entries` at `position` (or below the cursor), with
    /// `active` the active row: measured, in the window, flipped up or left
    /// at its edges and scrolling when it's taller than the window (Monaco's
    /// context view).
    fn show_menu(
        &mut self,
        entries: Vec<Entry>,
        position: Option<Point<Pixels>>,
        active: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if entries.is_empty() {
            return;
        }
        let Some(layout) = self.layout.clone() else {
            return;
        };
        let anchor = position.unwrap_or_else(|| {
            // Below the cursor (after any injected text before it).
            let cursor = self.cursors.primary().position;
            match layout.line_for_point(cursor) {
                Some(line) => gpui::point(
                    layout.text_origin_x() + line.x_for_column(cursor.column),
                    layout.line_top(line.view_line) + layout.line_height_of(line.view_line),
                ),
                None => layout.text_bounds.origin,
            }
        });
        let viewport = window.viewport_size();
        let max_height = viewport.height;
        let height = menu_height(&entries).min(max_height);
        let width = menu_width(&entries, viewport.width, window);
        let mut origin = anchor;
        if anchor.y + height > viewport.height {
            origin.y = (anchor.y - height).max(px(0.));
        }
        if anchor.x + width > viewport.width {
            origin.x = (anchor.x - width).max(px(0.));
        }
        let offset = origin - layout.bounds.origin;
        let focus = cx.focus_handle();
        let colors = self.theme.colors.clone();
        let editor = cx.entity().downgrade();
        let menu = cx.new(|_| ContextMenu {
            editor,
            entries,
            active,
            submenu: None,
            submenu_timer: None,
            focus: focus.clone(),
            colors,
            origin,
            width,
            max_height,
        });
        let id = self.add_overflow_overlay_widget(
            OverlayPosition::Custom(offset),
            menu.clone().into(),
            crate::widgets::layer::MENU,
            cx,
        );
        self.context_menu.menu = Some((menu, id));
        window.focus(&focus, cx);
        // It takes the focus from a menu it replaces.
        self.refocus = false;
        // Monaco hides it when it loses focus (unless another menu already
        // replaced it).
        let editor = cx.entity().downgrade();
        window
            .on_focus_out(&focus, cx, move |_, _, cx| {
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |editor, cx| {
                        if editor
                            .context_menu
                            .menu
                            .as_ref()
                            .is_some_and(|(_, m)| *m == id)
                        {
                            editor.hide_context_menu(cx);
                        }
                    });
                }
            })
            .detach();
    }

    /// Whether `point` (window coordinates) is over injected text, such as
    /// an inlay hint (Monaco's mouse target `detail.injectedText`).
    pub(crate) fn injected_text_at(&self, point: Point<Pixels>) -> bool {
        let Some(layout) = self.layout.as_ref() else {
            return false;
        };
        if !layout.text_bounds.contains(&point) {
            return false;
        }
        let Some(line) = layout.lines.iter().find(|line| {
            let top = layout.line_top(line.view_line);
            top <= point.y && point.y < top + layout.line_height_of(line.view_line)
        }) else {
            return false;
        };
        let x = point.x - layout.text_origin_x() - line.indent_x;
        line.injected.iter().any(|injected| {
            line.shaped.x_for_index(injected.range.start) <= x
                && x < line.shaped.x_for_index(injected.range.end)
        })
    }

    /// Hides the context menu.
    pub fn hide_context_menu(&mut self, cx: &mut Context<Self>) {
        if let Some((_, id)) = self.context_menu.menu.take() {
            self.remove_widget(id, cx);
            self.refocus = true;
        }
    }

    /// Whether the context menu shows.
    pub fn is_context_menu_visible(&self) -> bool {
        self.context_menu.menu.is_some()
    }

    /// Monaco `_onContextMenu`: focuses the editor, moves the cursor to
    /// `position` unless a selection contains it, and shows the menu at
    /// `point` if the option is on.
    pub(crate) fn context_menu_mouse_down(
        &mut self,
        position: Option<PointUtf16>,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(position) = position {
            let inside = self.selections().iter().any(|s| {
                let (start, end) = (s.start(), s.end());
                start <= position && position <= end
            });
            if !inside {
                self.set_selections(&[Selection::collapsed(position)], cx);
            }
        }
        if self.options.context_menu {
            self.show_context_menu(Some(point), window, cx);
        }
    }

    /// The open menu's measured width, for tests.
    #[doc(hidden)]
    pub fn context_menu_width_for_tests(&self, cx: &App) -> Option<Pixels> {
        Some(self.context_menu.menu.as_ref()?.0.read(cx).width)
    }

    /// The menu's rows as text (`---` for separators, `>` before the active
    /// row, submenus nested with two spaces), for tests.
    #[doc(hidden)]
    pub fn context_menu_rows_for_tests(&self, cx: &App) -> Vec<String> {
        let Some((menu, _)) = &self.context_menu.menu else {
            return Vec::new();
        };
        let menu = menu.read(cx);
        let mut rows = Vec::new();
        for (ix, entry) in menu.entries.iter().enumerate() {
            let active = if menu.active == Some(ix) { "> " } else { "" };
            match entry {
                Entry::Separator => rows.push("---".to_string()),
                Entry::Item { label, checked, .. } => {
                    let check = if *checked { "✓ " } else { "" };
                    rows.push(format!("{active}{check}{label}"))
                }
                Entry::Submenu { label, entries } => {
                    rows.push(format!("{active}{label} ›"));
                    for sub in entries {
                        if let Some(label) = sub.label() {
                            rows.push(format!("  {label}"));
                        }
                    }
                }
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str) -> Entry {
        Entry::item(
            label.to_string().into(),
            None,
            Run::Action("code_editor::Copy"),
        )
    }

    /// Monaco's submenu `layout`: after its parent when it fits in the
    /// window, else before it.
    #[test]
    fn submenus_flip_at_the_window_edge() {
        assert_eq!(
            submenu_left(px(100.), px(200.), px(150.), px(1000.)),
            px(296.)
        );
        assert_eq!(
            submenu_left(px(700.), px(200.), px(150.), px(1000.)),
            px(554.)
        );
        assert_eq!(submenu_left(px(50.), px(200.), px(400.), px(500.)), px(0.));
    }

    #[test]
    fn groups_navigation_first_then_by_name() {
        let menu = build_menu(vec![
            ("z_commands".into(), 1., item("Command Palette")),
            ("9_cutcopypaste".into(), 2., item("Copy")),
            ("navigation".into(), 1.1, item("Go to Definition")),
            ("9_cutcopypaste".into(), 1., item("Cut")),
            ("1_modification".into(), 1.3, item("Format Document")),
        ]);
        let labels: Vec<_> = menu
            .iter()
            .map(|e| e.label().map_or("---".to_string(), |l| l.to_string()))
            .collect();
        assert_eq!(
            labels,
            vec![
                "Go to Definition",
                "---",
                "Format Document",
                "---",
                "Cut",
                "Copy",
                "---",
                "Command Palette"
            ]
        );
    }
}
