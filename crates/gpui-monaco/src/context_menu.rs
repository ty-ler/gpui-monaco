//! The context menu (Monaco `ContextMenuController` and the
//! `EditorContext` menu): right-click or Shift+F10 shows the editor's
//! actions in groups, with host actions from [`CodeEditor::add_action`]
//! that set a context menu group.
//!
//! Ports `contextmenu.ts` (MIT); the items and their groups and order come
//! from the actions' `contextMenuOpts` and `MenuRegistry` entries.

use std::collections::BTreeMap;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement,
    Styled, WeakEntity, Window, div, px,
};
use text::PointUtf16;

use crate::actions::{
    ContextMenuFirst, ContextMenuHide, ContextMenuLast, ContextMenuNext, ContextMenuPrevious,
    ContextMenuSelect, ContextMenuSubmenuClose, ContextMenuSubmenuOpen,
};
use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::widgets::{OverlayPosition, WidgetId};

/// The key context of the context menu.
pub const CONTEXT_MENU_KEY_CONTEXT: &str = "EditorContextMenu";

/// Monaco's menu row height.
const ROW_HEIGHT: f32 = 24.;
const SEPARATOR_HEIGHT: f32 = 9.;
const MENU_WIDTH: f32 = 260.;

/// Whether a menu item shows.
type When = fn(&CodeEditor, &App) -> bool;

/// What a menu item runs.
#[derive(Clone, Debug, PartialEq)]
enum Run {
    /// A GPUI action, by name.
    Action(&'static str),
    /// An editor action, by id ([`CodeEditor::run_action`]).
    EditorAction(SharedString),
}

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

/// Whether the built-in action `id` can run.
fn supported(editor: &CodeEditor, id: &str, cx: &App) -> bool {
    editor
        .supported_actions(cx)
        .iter()
        .any(|action| action.id.as_ref() == id)
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
when_supported!(can_refactor, "editor.action.refactor");
when_supported!(can_source_action, "editor.action.sourceAction");

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
    BuiltinItem("navigation", 3., "Go to Symbol...", "code_editor::QuickOutline", always),
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
    focus: FocusHandle,
    colors: crate::theme::EditorColors,
    /// The window's height: the list scrolls past it.
    max_height: Pixels,
}

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
                }
            });
        });
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            cx.defer(move |cx| editor.update(cx, |editor, cx| editor.hide_context_menu(cx)));
        }
    }

    fn render_entries(
        &self,
        entries: &[Entry],
        active: Option<usize>,
        submenu_of: Option<usize>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = &self.colors;
        div()
            .w(px(MENU_WIDTH))
            .py(px(4.))
            .flex()
            .flex_col()
            // Monaco's menu colors (`menu.*`); the border only shows in
            // high contrast.
            .bg(colors.menu_background)
            .border_1()
            .border_color(colors.menu_border.unwrap_or_else(gpui::transparent_black))
            .rounded(px(5.))
            .shadow_md()
            .text_size(px(13.))
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
                        let keybinding = match entry {
                            Entry::Item { keybinding, .. } => keybinding.clone(),
                            _ => None,
                        };
                        let is_submenu = matches!(entry, Entry::Submenu { .. });
                        let id: SharedString = match submenu_of {
                            Some(parent) => format!("context-menu-{parent}-{ix}").into(),
                            None => format!("context-menu-{ix}").into(),
                        };
                        div()
                            .id(id)
                            .h(px(ROW_HEIGHT))
                            .mx(px(4.))
                            .px(px(20.))
                            .flex()
                            .items_center()
                            .rounded(px(3.))
                            .cursor_pointer()
                            .when_active(is_active, colors)
                            .child(div().flex_1().whitespace_nowrap().child(label.clone()))
                            .children(keybinding.map(|keys| {
                                div()
                                    .pl(px(16.))
                                    .opacity(0.8)
                                    .text_size(px(12.))
                                    .child(keys)
                            }))
                            .children(is_submenu.then(|| div().pl(px(8.)).child("›")))
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if !*hovered {
                                    return;
                                }
                                match submenu_of {
                                    Some(parent) => {
                                        this.submenu = Some((parent, Some(ix)));
                                    }
                                    None => {
                                        this.active = Some(ix);
                                        this.submenu = None;
                                        if is_submenu {
                                            this.open_submenu(cx);
                                        }
                                    }
                                }
                                cx.notify();
                            }))
                            .on_mouse_down(
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.entries.clone();
        let menu = div()
            .id("context-menu-list")
            .max_h(self.max_height)
            .overflow_y_scroll()
            .child(self.render_entries(&entries, self.active, None, cx));
        let submenu = self.submenu.and_then(|(ix, active)| match &entries[ix] {
            Entry::Submenu { entries, .. } => {
                // Beside its row.
                let top = entries_offset(&self.entries, ix);
                Some(
                    div()
                        .absolute()
                        .left(px(MENU_WIDTH - 4.))
                        .top(top)
                        .child(self.render_entries(entries, active, Some(ix), cx)),
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
                if this.submenu.take().is_some() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ContextMenuHide, _, cx| this.hide(cx)))
            .child(menu)
            .children(submenu)
    }
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
}

impl CodeEditor {
    /// The menu's entries (Monaco `_getMenuActions`), with keybindings.
    fn context_menu_entries(&self, window: &Window, cx: &App) -> Vec<Entry> {
        let keys = |name: &str| -> Option<SharedString> {
            let action = cx.build_action(name, None).ok()?;
            let binding = window
                .highest_precedence_binding_for_action_in(action.as_ref(), &self.focus_handle)?;
            Some(
                binding
                    .keystrokes()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
                    .into(),
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
                        Entry::Item {
                            label: item.2.into(),
                            keybinding: keys(item.3),
                            run: Run::Action(item.3),
                        },
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
                Entry::Item {
                    label: action.label.clone(),
                    keybinding: binding.map(|b| {
                        b.keystrokes()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" ")
                            .into()
                    }),
                    run: Run::EditorAction(action.id.clone()),
                },
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
        let entries = self.context_menu_entries(window, cx);
        if entries.is_empty() {
            return;
        }
        let Some(layout) = self.layout.clone() else {
            return;
        };
        let anchor = position.unwrap_or_else(|| {
            // Below the cursor.
            let cursor = self.cursors.primary().position;
            match layout.line_for_point(cursor) {
                Some(line) => gpui::point(
                    layout.text_origin_x() + line.x_for_column(cursor.column),
                    layout.line_top(line.view_line) + layout.line_height_of(line.view_line),
                ),
                None => layout.text_bounds.origin,
            }
        });
        // Monaco's context view: in the window, not just the editor,
        // flipping up or left at its edges, and scrolling when it's taller
        // than the window.
        let viewport = window.viewport_size();
        let max_height = viewport.height;
        let height = menu_height(&entries).min(max_height);
        let width = px(MENU_WIDTH);
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
            active: None,
            submenu: None,
            focus: focus.clone(),
            colors,
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
        // Monaco hides it when it loses focus.
        let editor = cx.entity().downgrade();
        window
            .on_focus_out(&focus, cx, move |_, _, cx| {
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |editor, cx| editor.hide_context_menu(cx));
                }
            })
            .detach();
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
                Entry::Item { label, .. } => rows.push(format!("{active}{label}")),
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
        Entry::Item {
            label: label.to_string().into(),
            keybinding: None,
            run: Run::Action("code_editor::Copy"),
        }
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
