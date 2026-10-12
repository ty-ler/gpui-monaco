//! Code actions: the lightbulb, the code action menu (Cmd/Ctrl+.) and the
//! refactor, source action, organize imports, fix all and auto fix
//! commands.
//!
//! Ports Monaco's `src/vs/editor/contrib/codeAction/` (`codeAction.ts`,
//! `codeActionModel.ts`, `codeActionController.ts`, `codeActionMenu.ts`,
//! `lightBulbWidget.ts`, `common/types.ts`) and the action list widget
//! with its action bar (documentation and "Show Disabled") and preview
//! (Cmd/Ctrl+Enter, through
//! [`set_workspace_edit_preview_handler`](crate::set_workspace_edit_preview_handler))
//! (MIT). Not ported: nearby quick fixes.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, FontWeight, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use text::PointUtf16;

use crate::actions::*;
use crate::api::DecorationsCollection;
use crate::codicons::{Codicon, icon as codicon};
use crate::decorations::DecorationOptions;
use crate::editor::CodeEditor;
use crate::keybinding_label::{KeyLabel, keycaps_width, ui_text_width};
use crate::options::LightbulbMode;
use crate::providers::{
    CodeAction, CodeActionContext, CodeActionKind, CodeActionProvider, CodeActionTrigger, Command,
    code_action_providers,
};
use crate::theme::EditorColors;
use crate::widgets::{ContentWidgetPreference, WidgetId};

/// The key context of the code action menu.
pub const CODE_ACTION_MENU_KEY_CONTEXT: &str = "CodeActionMenu";

/// Monaco `CodeActionOracle` waits this long after the cursor moves.
const AUTO_TRIGGER_DELAY: Duration = Duration::from_millis(250);

/// Where the code action menu shows (Monaco `showCodeActionList`'s
/// `IAnchor | IPosition`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MenuAnchor {
    /// Below (else above) a text position's line (Monaco `toCoords`).
    Text(PointUtf16),
    /// Below (else above) a box in the window, such as the problem hover's
    /// clicked "Quick Fix..." link; `position` is where it's about.
    Window {
        bounds: gpui::Bounds<gpui::Pixels>,
        position: PointUtf16,
    },
}

impl MenuAnchor {
    /// The text position it's about.
    fn position(self) -> PointUtf16 {
        match self {
            MenuAnchor::Text(position) | MenuAnchor::Window { position, .. } => position,
        }
    }
}

/// Which actions a request keeps (Monaco `CodeActionFilter`).
#[derive(Clone, Debug, Default)]
pub struct CodeActionFilter {
    /// Only actions of this kind (and actions need a kind then).
    pub include: Option<CodeActionKind>,
    /// Drop actions of these kinds, unless `include` is more specific.
    pub excludes: Vec<CodeActionKind>,
    /// Keep `source` actions.
    pub include_source_actions: bool,
    /// Keep only preferred actions.
    pub only_preferred: bool,
}

impl CodeActionFilter {
    fn excludes_kind(&self, kind: &CodeActionKind) -> bool {
        self.excludes.iter().any(|exclude| {
            exclude.contains(kind)
                && !self
                    .include
                    .as_ref()
                    .is_some_and(|include| exclude.contains(include))
        })
    }

    fn source_excluded(&self, kind: &CodeActionKind) -> bool {
        !self.include_source_actions && CodeActionKind::from(CodeActionKind::SOURCE).contains(kind)
    }

    /// Monaco `filtersAction`: whether `action` is kept.
    pub fn keeps(&self, action: &CodeAction) -> bool {
        if let Some(include) = &self.include {
            match &action.kind {
                Some(kind) if include.contains(kind) => {}
                _ => return false,
            }
        }
        if let Some(kind) = &action.kind
            && (self.excludes_kind(kind) || self.source_excluded(kind))
        {
            return false;
        }
        !self.only_preferred || action.is_preferred
    }

    /// Monaco `mayIncludeActionsOfKind`: whether a provider of `kind` may
    /// return wanted actions.
    fn may_include(&self, kind: &CodeActionKind) -> bool {
        if self
            .include
            .as_ref()
            .is_some_and(|include| !include.intersects(kind))
        {
            return false;
        }
        !(self.excludes_kind(kind) || self.source_excluded(kind))
    }
}

/// When a command applies an action without the menu (Monaco
/// `CodeActionAutoApply`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeActionAutoApply {
    /// When it's the only valid one.
    IfSingle,
    /// The first valid one.
    First,
    /// Always show the menu.
    Never,
}

/// An action and the provider that offered it.
#[derive(Clone)]
pub(crate) struct Item {
    pub action: CodeAction,
    provider: Arc<dyn CodeActionProvider>,
}

/// The actions a request found and the providers' documentation for them
/// (Monaco `CodeActionSet`).
#[derive(Clone, Default)]
pub(crate) struct CodeActionSet {
    /// Every action, disabled ones included (Monaco `allActions`).
    pub items: Vec<Item>,
    /// Documentation commands for the menu's action bar.
    pub documentation: Vec<Command>,
}

impl CodeActionSet {
    /// Whether some action isn't disabled (Monaco `validActions.length > 0`).
    pub(crate) fn has_valid(&self) -> bool {
        !valid(&self.items).is_empty()
    }

    /// The first valid AI action (Monaco `validActions.find(a =>
    /// a.action.isAI)`).
    pub(crate) fn valid_ai(&self) -> Option<&Item> {
        valid(&self.items)
            .into_iter()
            .find(|item| item.action.is_ai)
    }
}

/// Monaco `getDocumentationFromProvider`: the documentation entry whose
/// kind most specifically contains `only`, else the first containing one
/// of `actions`' kinds.
fn documentation_for(
    documentation: &[(CodeActionKind, Command)],
    actions: &[CodeAction],
    only: Option<&CodeActionKind>,
) -> Option<Command> {
    if documentation.is_empty() {
        return None;
    }
    if let Some(only) = only {
        let mut best: Option<&(CodeActionKind, Command)> = None;
        for entry in documentation {
            if entry.0.contains(only) {
                match best {
                    Some(current) if !current.0.contains(&entry.0) => {}
                    _ => best = Some(entry),
                }
            }
        }
        if let Some((_, command)) = best {
            return Some(command.clone());
        }
    }
    actions.iter().find_map(|action| {
        let kind = action.kind.as_ref()?;
        documentation
            .iter()
            .find(|(entry, _)| entry.contains(kind))
            .map(|(_, command)| command.clone())
    })
}

/// Monaco `CodeActionController`'s `onHover`: refactorings, source actions
/// and actions with edits can be previewed.
fn can_preview(action: &CodeAction) -> bool {
    let previewable = [
        CodeActionKind::REFACTOR_EXTRACT,
        CodeActionKind::REFACTOR_INLINE,
        CodeActionKind::REFACTOR_REWRITE,
        CodeActionKind::REFACTOR_MOVE,
        CodeActionKind::SOURCE,
    ];
    let by_kind = action.kind.as_ref().is_some_and(|kind| {
        previewable
            .iter()
            .any(|group| CodeActionKind::from(*group).contains(kind))
    });
    by_kind
        || action
            .edit
            .as_ref()
            .is_some_and(|edit| !edit.edits.is_empty())
}

/// Monaco `ManagedCodeActionSet`'s order: AI actions last, actions with
/// diagnostics first, then preferred ones; otherwise provider order.
fn sort_items(items: &mut [Item]) {
    items.sort_by_key(|item| {
        (
            item.action.is_ai,
            item.action.diagnostics.is_empty(),
            !item.action.is_preferred,
        )
    });
}

fn valid(items: &[Item]) -> Vec<&Item> {
    items
        .iter()
        .filter(|item| item.action.disabled.is_none())
        .collect()
}

/// Some valid preferred quick fix (Monaco `hasAutoFix`).
fn has_auto_fix(items: &[Item]) -> bool {
    let quick_fix = CodeActionKind::from(CodeActionKind::QUICK_FIX);
    valid(items).iter().any(|item| {
        item.action.is_preferred
            && item
                .action
                .kind
                .as_ref()
                .is_some_and(|kind| quick_fix.contains(kind))
    })
}

/// The lightbulb: shown at a row when auto-triggered actions exist.
#[derive(Clone)]
pub(crate) struct Lightbulb {
    /// The cursor when the actions were asked for.
    pub position: PointUtf16,
    pub items: std::rc::Rc<Vec<Item>>,
    /// A preferred quick fix exists (`editorLightBulbAutoFix.foreground`).
    pub auto_fix: bool,
}

#[derive(Default)]
pub(crate) struct CodeActionState {
    auto: Option<Task<()>>,
    invoke: Option<Task<()>>,
    apply: Option<Task<()>>,
    pub lightbulb: Option<Lightbulb>,
    menu: Option<(WidgetId, Entity<CodeActionMenu>)>,
    highlight: Option<DecorationsCollection>,
    /// The menu lists disabled actions too (Monaco `_showDisabled`, kept
    /// across menus).
    show_disabled: bool,
}

/// How the menu was opened (Monaco `IActionShowOptions`).
#[derive(Clone, Copy, Debug, Default)]
struct MenuOptions {
    /// Disabled actions may be listed: a command asked for a kind.
    include_disabled: bool,
    /// The lightbulb was clicked: no action bar.
    from_lightbulb: bool,
}

/// A button in the menu's action bar (Monaco `_getActionBarActions`).
#[derive(Clone)]
enum BarAction {
    /// A provider's documentation command.
    Documentation(Command),
    /// Monaco `showMoreActions` / `hideMoreActions`.
    ToggleDisabled { showing: bool },
}

impl BarAction {
    fn label(&self) -> SharedString {
        match self {
            BarAction::Documentation(command) => command.title.clone(),
            BarAction::ToggleDisabled { showing: true } => "Hide Disabled".into(),
            BarAction::ToggleDisabled { showing: false } => "Show Disabled".into(),
        }
    }
}

/// Where the lightbulb is drawn (Monaco `LightBulbWidget`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LightbulbPlacement {
    /// In the glyph margin of this row.
    Gutter(u32),
    /// In the text of this row, at this (0-based) column.
    Content(u32, u32),
}

/// Monaco `LightBulbWidget._update`'s placement rules. `lines` gives a
/// row's text, `folded` whether a row is hidden in a fold.
pub(crate) fn place_lightbulb(
    position: PointUtf16,
    line_count: u32,
    lines: &dyn Fn(u32) -> String,
    folded: &dyn Fn(u32) -> bool,
    tab_size: u32,
    space_width: f32,
    has_glyph: bool,
) -> Option<LightbulbPlacement> {
    let row = position.row;
    let line = lines(row);
    // Indentation in visible columns; whitespace-only lines have none.
    let indent = if line.trim().is_empty() {
        0
    } else {
        line.chars()
            .take_while(|c| c.is_whitespace())
            .fold(0, |col, c| {
                if c == '\t' {
                    col + tab_size - col % tab_size
                } else {
                    col + 1
                }
            })
    };
    if space_width * indent as f32 > 22. {
        return Some(LightbulbPlacement::Content(row, 0));
    }
    let empty_or_indented = |row: u32| {
        let line = lines(row);
        line.trim().is_empty() || line.starts_with(char::is_whitespace) || line.chars().count() <= 1
    };
    let content = |row: u32| {
        let line = lines(row);
        // After a lone non-space character.
        let mut chars = line.chars();
        let single =
            chars.next().is_some_and(|c| !c.is_whitespace()) && chars.all(char::is_whitespace);
        LightbulbPlacement::Content(row, if single { 1 } else { 0 })
    };
    let last = line_count.saturating_sub(1);
    if row > 0 && !folded(row - 1) {
        let prev = empty_or_indented(row - 1);
        let next = row < last && empty_or_indented(row + 1);
        // The last line has no next line to block (Monaco's `endLine`).
        if !prev && !next && !has_glyph {
            return Some(LightbulbPlacement::Gutter(row));
        }
        if prev || row == last {
            return Some(content(row - 1));
        }
        if next || empty_or_indented(row) {
            return Some(content(row + 1));
        }
        return Some(content(row));
    }
    if row == 0 && (line_count == 1 || (!empty_or_indented(0) && !empty_or_indented(1))) {
        if !has_glyph {
            return Some(LightbulbPlacement::Gutter(0));
        }
        return Some(content(0));
    }
    if row < last && !folded(row + 1) {
        return Some(content(row + 1));
    }
    // It would cover the cursor (Monaco's column is 1-based).
    if ((position.column + 1) as f32) * space_width < 22. {
        return None;
    }
    Some(content(row))
}

/// The menu's rows (Monaco `codeActionMenu.ts` with headers).
#[derive(Clone)]
enum Row {
    Header(&'static str),
    Action(usize),
}

/// Monaco's menu groups, in order; the empty kind catches the rest.
const GROUPS: [(&str, &str); 8] = [
    ("Quick Fix", CodeActionKind::QUICK_FIX),
    ("Extract", CodeActionKind::REFACTOR_EXTRACT),
    ("Inline", CodeActionKind::REFACTOR_INLINE),
    ("Rewrite", CodeActionKind::REFACTOR_REWRITE),
    ("Move", CodeActionKind::REFACTOR_MOVE),
    ("Surround With", "refactor.surround"),
    ("Source Action", CodeActionKind::SOURCE),
    ("More Actions...", ""),
];

/// The index in [`GROUPS`] of the group an action goes in.
fn group_of(action: &CodeAction) -> usize {
    match &action.kind {
        Some(kind) => GROUPS
            .iter()
            .position(|(_, group)| CodeActionKind::from(*group).contains(kind))
            .unwrap_or(GROUPS.len() - 1),
        None => GROUPS.len() - 1,
    }
}

/// A row's icon (Monaco `codeActionGroups`' icons): `sparkle` for AI
/// actions, `wrench` for refactorings, `surround-with` and `symbol-file`
/// for those groups, else `None` for the action list's default lightbulb.
fn group_icon(action: &CodeAction) -> Option<Codicon> {
    if action.is_ai {
        return Some(Codicon::Sparkle);
    }
    match group_of(action) {
        1..=4 => Some(Codicon::Wrench),
        5 => Some(Codicon::SurroundWith),
        6 => Some(Codicon::SymbolFile),
        _ => None,
    }
}

/// Monaco `toMenuItems`: grouped under headers, or without
/// (`showHeaders: false`) in the actions' order.
fn menu_rows(items: &[Item], show_headers: bool) -> Vec<Row> {
    if !show_headers {
        return (0..items.len()).map(Row::Action).collect();
    }
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); GROUPS.len()];
    for (ix, item) in items.iter().enumerate() {
        groups[group_of(&item.action)].push(ix);
    }
    let mut rows = Vec::new();
    for (group, items) in groups.into_iter().enumerate() {
        if !items.is_empty() {
            rows.push(Row::Header(GROUPS[group].0));
            rows.extend(items.into_iter().map(Row::Action));
        }
    }
    rows
}

/// Monaco's action list widget for code actions.
pub(crate) struct CodeActionMenu {
    editor: WeakEntity<CodeEditor>,
    /// The listed actions.
    items: Vec<Item>,
    rows: Vec<Row>,
    /// Grouped under headers (Monaco `editor.codeActionWidget.showHeaders`);
    /// without, rows have no group icon or keybinding, and a disabled
    /// action shows why instead of its title.
    show_headers: bool,
    /// Everything the request found, to list again with "Show Disabled".
    set: std::rc::Rc<CodeActionSet>,
    at: MenuAnchor,
    options: MenuOptions,
    bar: Vec<BarAction>,
    focused: Option<usize>,
    colors: EditorColors,
    focus: FocusHandle,
    /// The editor's focus, where the code action commands' keys are
    /// looked up.
    editor_focus: Option<FocusHandle>,
    /// The list's scrolling (Monaco's `List` reveals the focused row).
    scroll: gpui::ScrollHandle,
    /// The list's size as last rendered.
    list_height: gpui::Pixels,
    list_width: gpui::Pixels,
    /// Each row's keybinding as last rendered.
    row_keys: Vec<Option<KeyLabel>>,
    focus_pending: bool,
    /// The room it has in the window (it overflows the editor, as Monaco's
    /// action list is in the context view).
    room: crate::widgets::OverflowRoomCell,
    _subscriptions: Vec<Subscription>,
}

impl CodeActionMenu {
    fn focusable(&self, row: usize) -> bool {
        matches!(self.rows[row], Row::Action(ix) if self.items[ix].action.disabled.is_none())
    }

    /// Moves focus, wrapping and skipping headers and disabled actions.
    fn move_focus(&mut self, forward: bool, cx: &mut Context<Self>) {
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        let start = self.focused.unwrap_or(if forward { len - 1 } else { 0 });
        for step in 1..=len {
            let row = if forward {
                (start + step) % len
            } else {
                (start + len - step % len) % len
            };
            if self.focusable(row) {
                self.set_focus(Some(row), cx);
                return;
            }
        }
    }

    fn set_focus(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        if self.focused == row {
            return;
        }
        self.focused = row;
        if let Some(row) = row {
            self.scroll.scroll_to_item(row);
        }
        let action = row.and_then(|row| match self.rows[row] {
            Row::Action(ix) => Some(self.items[ix].action.clone()),
            Row::Header(_) => None,
        });
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| {
                editor.focus_code_action(action.as_ref(), cx)
            });
        }
        cx.notify();
    }

    /// Monaco `acceptSelected`: applies the action at `row`, previewing
    /// its edits first if `preview`.
    fn accept(&mut self, row: usize, preview: bool, cx: &mut Context<Self>) {
        // Headers and disabled actions can't be accepted.
        let Row::Action(ix) = self.rows[row] else {
            return;
        };
        if self.items[ix].action.disabled.is_some() {
            return;
        }
        let item = self.items[ix].clone();
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| {
                editor.hide_code_action_menu(cx);
                editor.apply_code_action(item, true, preview, cx);
            });
        }
    }

    /// Runs action bar button `ix`.
    fn run_bar_action(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(action) = self.bar.get(ix).cloned() else {
            return;
        };
        let (set, at, options) = (self.set.clone(), self.at, self.options);
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| match action {
                BarAction::Documentation(command) => {
                    editor.hide_code_action_menu(cx);
                    // Unregistered commands are the host's, as for code
                    // lenses.
                    if crate::commands::execute_command(cx, &command).is_err() {
                        cx.emit(crate::CodeEditorEvent::ExecuteCommand(command));
                    }
                }
                BarAction::ToggleDisabled { showing } => {
                    editor.code_actions.show_disabled = !showing;
                    editor.show_code_action_list(set, at, options, cx);
                }
            });
        }
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, |editor, cx| editor.hide_code_action_menu(cx));
        }
    }
}

/// Monaco `ActionList`'s row heights (`_actionLineHeight`,
/// `_headerLineHeight`).
const ACTION_ROW_HEIGHT: f32 = 24.;
const HEADER_ROW_HEIGHT: f32 = 26.;
/// `.action-widget { font-size: 13px }`; group headers are 12px.
const MENU_FONT_SIZE: f32 = 13.;
const HEADER_FONT_SIZE: f32 = 12.;
/// The row's icon box (a 16px codicon).
const ICON_WIDTH: f32 = 16.;
/// `.monaco-list-row { padding: 0 4px }` and the 4px gap between its parts.
const ROW_PADDING: f32 = 4.;
const ROW_GAP: f32 = 4.;
/// The action bar under the list: `margin-top: 2px`, a 1px border, `3px`
/// top padding and 22px labels.
const BAR_HEIGHT: f32 = 2. + 1. + 3. + 22.;

impl CodeActionMenu {
    /// A row's text: the title on one line, or without headers, why a
    /// disabled action is (Monaco `toMenuItems`' `label`).
    fn row_label(&self, action: &CodeAction) -> String {
        let label = match &action.disabled {
            Some(reason) if !self.show_headers => reason,
            _ => &action.title,
        };
        label.replace(['\r', '\n'], " ")
    }

    fn row_height(&self, row: usize) -> gpui::Pixels {
        match self.rows[row] {
            Row::Header(_) => px(HEADER_ROW_HEIGHT),
            Row::Action(_) => px(ACTION_ROW_HEIGHT),
        }
    }

    /// The keybinding of the row's action (Monaco
    /// `CodeActionKeybindingResolver`), looked up in the editor.
    fn row_keybinding(&self, ix: usize, window: &Window) -> Option<KeyLabel> {
        // Monaco's rows without headers have no keybinding.
        if !self.show_headers {
            return None;
        }
        let kind = self.items[ix].action.kind.as_ref()?;
        let editor_focus = self.editor_focus.as_ref()?;
        let organize = CodeActionKind::from(CodeActionKind::SOURCE_ORGANIZE_IMPORTS);
        let fix_all = CodeActionKind::from(CodeActionKind::SOURCE_FIX_ALL);
        let binding = if organize.contains(kind) {
            crate::keybinding_label::command_binding(&OrganizeImports, None, editor_focus, window)
        } else if fix_all.contains(kind) {
            crate::keybinding_label::command_binding(&FixAll, None, editor_focus, window)
        } else {
            None
        }?;
        Some(KeyLabel::for_binding(&binding))
    }

    /// Monaco `ActionList.layout`: the list is as wide as its widest row
    /// (at least the action bar), measured.
    fn measure_width(&self, keys: &[Option<KeyLabel>], window: &mut Window) -> gpui::Pixels {
        let mut width = px(0.);
        for (row, kind) in self.rows.iter().enumerate() {
            let row_width = match kind {
                Row::Header(title) => {
                    ui_text_width(title, px(HEADER_FONT_SIZE), FontWeight::SEMIBOLD, window)
                }
                Row::Action(ix) => {
                    let title = self.row_label(&self.items[*ix].action);
                    let mut w = px(ICON_WIDTH + ROW_GAP)
                        + ui_text_width(&title, px(MENU_FONT_SIZE), FontWeight::NORMAL, window);
                    if let Some(keys) = &keys[row] {
                        w += px(ROW_GAP * 2.) + keycaps_width(keys, window);
                    }
                    w
                }
            };
            width = width.max(row_width + px(ROW_PADDING * 2.));
        }
        if !self.bar.is_empty() {
            // `.actions-container { padding: 3px 8px 0 }`, 16px after each.
            let labels = self
                .bar
                .iter()
                .map(|action| {
                    ui_text_width(&action.label(), px(12.), FontWeight::NORMAL, window) + px(16.)
                })
                .fold(px(16.), |a, b| a + b);
            width = width.max(labels);
        }
        width.ceil()
    }
}

impl Render for CodeActionMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_pending) {
            window.focus(&self.focus, cx);
            // Monaco hides the menu when it loses focus.
            let subscription = cx.on_blur(&self.focus, window, |this, _, cx| this.hide(cx));
            self._subscriptions.push(subscription);
        }
        let colors = self.colors.clone();
        // Monaco's row titles name the accept and preview keys.
        let label = |action: &dyn gpui::Action, window: &Window| {
            window
                .highest_precedence_binding_for_action_in(action, &self.focus)
                .map(|binding| KeyLabel::for_binding(&binding).text)
        };
        let accept_keys = label(&CodeActionMenuAccept, window).unwrap_or("Enter".into());
        let preview_keys = label(&CodeActionMenuPreview, window).unwrap_or_default();
        let keys: Vec<Option<KeyLabel>> = self
            .rows
            .iter()
            .map(|row| match row {
                Row::Action(ix) => self.row_keybinding(*ix, window),
                Row::Header(_) => None,
            })
            .collect();
        // Sizes: as wide as the widest row, at most 80% of the window, and
        // as tall as the rows, at most 70% of the window (Monaco
        // `ActionList.layout`), and the room on the side it shows on.
        let viewport = window.viewport_size();
        let chrome = px(4. * 2. + 2.);
        let width = self
            .measure_width(&keys, window)
            .min(viewport.width * 0.8 - chrome)
            .min(self.room.cap_width(px(f32::MAX), px(100.)) - chrome)
            .max(px(100.));
        let content: gpui::Pixels = (0..self.rows.len())
            .map(|row| self.row_height(row))
            .fold(px(0.), |a, b| a + b);
        let bar_height = if self.bar.is_empty() {
            px(0.)
        } else {
            px(BAR_HEIGHT)
        };
        let room = self.room.cap_height(px(f32::MAX), px(ACTION_ROW_HEIGHT)) - chrome - bar_height;
        let height = content
            .min(viewport.height * 0.7)
            .min(room)
            .max(px(ACTION_ROW_HEIGHT).min(content));
        self.list_height = height;
        self.list_width = width;
        self.row_keys = keys.clone();
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(row, kind)| match kind {
                // `.group-header`: 12px, semibold, `descriptionForeground`.
                Row::Header(title) => div()
                    .id(("code-action-header", row))
                    .h(px(HEADER_ROW_HEIGHT))
                    .flex_none()
                    .px(px(ROW_PADDING))
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_size(px(HEADER_FONT_SIZE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.description_foreground)
                    .child(*title)
                    .into_any_element(),
                Row::Action(ix) => {
                    let action = &self.items[*ix].action;
                    let disabled = action.disabled.is_some();
                    let focused = self.focused == Some(row);
                    let label = self.row_label(action);
                    // Monaco's row titles: why it's disabled, or the keys.
                    let tooltip: SharedString = match &action.disabled {
                        Some(reason) => reason.clone(),
                        None if can_preview(action) && !preview_keys.is_empty() => {
                            format!("{accept_keys} to Apply, {preview_keys} to Preview").into()
                        }
                        None => format!("{accept_keys} to Apply").into(),
                    };
                    div()
                        .id(("code-action", row))
                        .h(px(ACTION_ROW_HEIGHT))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(ROW_GAP))
                        .px(px(ROW_PADDING))
                        .rounded(px(3.))
                        .whitespace_nowrap()
                        .cursor_pointer()
                        // Monaco `editorActionList.focus*`; high contrast
                        // outlines the row instead.
                        .when(focused && !disabled, |row| {
                            colors.focused_list_row(
                                row,
                                colors.action_list_focus_background,
                                colors.action_list_focus_foreground,
                            )
                        })
                        // `.option-disabled`: `disabledForeground`.
                        .when(disabled, |row| row.text_color(colors.disabled_foreground))
                        // Monaco's action list: the group's icon, or
                        // `light-bulb` in `editorLightBulb.foreground`.
                        .child(
                            div()
                                .w(px(ICON_WIDTH))
                                .flex_none()
                                .flex()
                                .justify_center()
                                .child(match group_icon(action).filter(|_| self.show_headers) {
                                    Some(icon) => codicon(icon),
                                    None if disabled => codicon(Codicon::LightBulb),
                                    None => codicon(Codicon::LightBulb)
                                        .color(colors.lightbulb_foreground),
                                }),
                        )
                        // `.title`: one line, cut with an ellipsis.
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(label),
                        )
                        .children(keys[row].as_ref().map(|keys| {
                            div()
                                .flex_none()
                                .ml(px(ROW_GAP))
                                .child(crate::keybinding_label::keycaps(keys, &colors))
                        }))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.focusable(row) {
                                this.set_focus(Some(row), cx);
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.accept(row, false, cx)))
                        .tooltip(crate::tooltip::tooltip(tooltip, &colors))
                        .into_any_element()
                }
            })
            .collect();
        div()
            .id("code-action-menu")
            .debug_selector(|| "code-action-menu".into())
            .key_context(CODE_ACTION_MENU_KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CodeActionMenuNext, _, cx| this.move_focus(true, cx)))
            .on_action(
                cx.listener(|this, _: &CodeActionMenuPrevious, _, cx| this.move_focus(false, cx)),
            )
            .on_action(cx.listener(|this, _: &CodeActionMenuAccept, _, cx| {
                if let Some(row) = this.focused {
                    this.accept(row, false, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CodeActionMenuPreview, _, cx| {
                if let Some(row) = this.focused {
                    this.accept(row, true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CodeActionMenuHide, _, cx| this.hide(cx)))
            // `.action-widget`: 4px padding, 5px corners and
            // `0 2px 8px widget.shadow`.
            .p(px(4.))
            .flex()
            .flex_col()
            .bg(colors.action_list_background)
            .border_1()
            .border_color(colors.widget_border)
            .rounded(px(5.))
            .shadow(crate::widgets::card::widget_shadow(colors.widget_shadow))
            .text_size(px(MENU_FONT_SIZE))
            .text_color(colors.action_list_foreground)
            .child(
                div()
                    .id("code-action-list")
                    .w(width)
                    .h(height)
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
            .when(!self.bar.is_empty(), |menu| {
                // Monaco's action bar: links under the list, outside its
                // scrolling.
                let buttons = self.bar.iter().enumerate().map(|(ix, action)| {
                    div()
                        .id(("code-action-bar", ix))
                        .mr(px(16.))
                        .cursor_pointer()
                        .text_color(colors.text_link_active_foreground)
                        .text_size(px(12.))
                        .line_height(px(22.))
                        .whitespace_nowrap()
                        .child(action.label())
                        .on_click(cx.listener(move |this, _, _, cx| this.run_bar_action(ix, cx)))
                });
                menu.child(
                    div()
                        .id("code-action-bar")
                        .w(width)
                        .mt(px(2.))
                        .pt(px(3.))
                        .px(px(8.))
                        .border_t_1()
                        .border_color(colors.hover_border)
                        .flex()
                        .children(buttons),
                )
            })
    }
}

impl CodeEditor {
    /// The markers intersecting or touching `range` (Monaco's standalone
    /// code action context).
    fn markers_in(&self, range: &Range<PointUtf16>, cx: &gpui::App) -> Vec<crate::Marker> {
        self.model()
            .read(cx)
            .markers()
            .into_iter()
            .filter(|marker| marker.range.start <= range.end && range.start <= marker.range.end)
            .collect()
    }

    /// Monaco `getCodeActions`: asks the providers that may have wanted
    /// actions, keeps those `filter` allows, and sorts them.
    fn request_code_actions(
        &mut self,
        range: Range<PointUtf16>,
        filter: CodeActionFilter,
        trigger: CodeActionTrigger,
        cx: &mut Context<Self>,
    ) -> Task<CodeActionSet> {
        let model = self.model().read(cx);
        let context = model.provider_context();
        let action_context = CodeActionContext {
            only: filter.include.clone(),
            trigger,
            markers: self.markers_in(&range, cx),
        };
        let tasks: Vec<_> = code_action_providers(cx, self.provider_target(cx))
            .into_iter()
            .filter(|provider| {
                provider
                    .provided_kinds()
                    .is_none_or(|kinds| kinds.iter().any(|kind| filter.may_include(kind)))
            })
            .map(|provider| {
                let task =
                    provider.provide_code_actions(&context, range.clone(), &action_context, cx);
                (provider, task)
            })
            .collect();
        // Providers aren't `Send`: wait on the foreground.
        cx.spawn(async move |_, _| {
            let mut items = Vec::new();
            let mut documentation = Vec::new();
            for (provider, task) in tasks {
                match task.await {
                    Ok(actions) => {
                        let actions: Vec<CodeAction> = actions
                            .into_iter()
                            .filter(|action| filter.keeps(action))
                            .collect();
                        documentation.extend(documentation_for(
                            &provider.documentation(),
                            &actions,
                            filter.include.as_ref(),
                        ));
                        items.extend(actions.into_iter().map(|action| Item {
                            action,
                            provider: provider.clone(),
                        }));
                    }
                    Err(err) => log::warn!("code actions failed: {err:#}"),
                }
            }
            sort_items(&mut items);
            CodeActionSet {
                items,
                documentation,
            }
        })
    }

    /// Monaco `MarkerHoverParticipant.getCodeActions`: the quick fixes for
    /// `marker`'s range (`markerCodeActionTrigger`).
    pub(crate) fn marker_quick_fixes(
        &mut self,
        marker: &crate::Marker,
        cx: &mut Context<Self>,
    ) -> Task<CodeActionSet> {
        let filter = CodeActionFilter {
            include: Some(CodeActionKind::QUICK_FIX.into()),
            ..Default::default()
        };
        self.request_code_actions(marker.range.clone(), filter, CodeActionTrigger::Invoke, cx)
    }

    /// Monaco `CodeActionController.showCodeActions`, from the problem
    /// hover's "Quick Fix...": the valid actions of `set` at `at` (the
    /// clicked link).
    pub(crate) fn show_marker_quick_fixes(
        &mut self,
        set: std::rc::Rc<CodeActionSet>,
        at: MenuAnchor,
        cx: &mut Context<Self>,
    ) {
        let options = MenuOptions {
            include_disabled: false,
            from_lightbulb: false,
        };
        self.show_code_action_list(set, at, options, cx);
    }

    /// After the cursor moves or markers change: asks for actions for the
    /// lightbulb after a pause (Monaco `CodeActionOracle`).
    pub(crate) fn schedule_auto_code_actions(&mut self, cx: &mut Context<Self>) {
        if self.options.read_only
            || self.in_peek
            || self.options.single_line
            || code_action_providers(cx, self.provider_target(cx)).is_empty()
        {
            self.code_actions.lightbulb = None;
            self.code_actions.auto = None;
            return;
        }
        let Some(range) = self.auto_trigger_range(cx) else {
            self.code_actions.lightbulb = None;
            self.code_actions.auto = None;
            cx.notify();
            return;
        };
        let position = self.cursors.primary().position;
        self.code_actions.auto = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTO_TRIGGER_DELAY).await;
            let Ok(request) = this.update(cx, |this, cx| {
                this.request_code_actions(range, Default::default(), CodeActionTrigger::Auto, cx)
            }) else {
                return;
            };
            let items = request.await.items;
            this.update(cx, |this, cx| {
                this.code_actions.auto = None;
                // Monaco drops results once the cursor left the line, and
                // doesn't touch an open menu.
                if this.cursors.primary().position.row != position.row
                    || this.code_actions.menu.is_some()
                {
                    return;
                }
                this.update_lightbulb(position, items, cx);
            })
            .ok();
        }));
    }

    /// Monaco `_getRangeOfSelectionUnlessWhitespaceEnclosed` for `onCode`.
    fn auto_trigger_range(&self, cx: &gpui::App) -> Option<Range<PointUtf16>> {
        let selection = self.cursors.selections()[0];
        match self.options.lightbulb {
            LightbulbMode::Off => return None,
            LightbulbMode::On => return Some(selection.range()),
            LightbulbMode::OnCode => {}
        }
        if !selection.is_empty() {
            return Some(selection.range());
        }
        let position = selection.head;
        let line: Vec<char> = self
            .model()
            .read(cx)
            .line_content(position.row)
            .chars()
            .collect();
        // Columns are UTF-16, so map through chars approximately by units.
        let mut units = 0;
        let mut ix = 0;
        while ix < line.len() && units < position.column {
            units += line[ix].len_utf16() as u32;
            ix += 1;
        }
        let space = |i: usize| line.get(i).is_some_and(|c| c.is_whitespace());
        let enclosed = if line.is_empty() {
            true
        } else if ix == 0 {
            space(0)
        } else if ix >= line.len() {
            space(line.len() - 1)
        } else {
            space(ix - 1) && space(ix)
        };
        (!enclosed).then(|| selection.range())
    }

    /// Monaco `LightBulbWidget.update`: no bulb without valid actions or
    /// with `lightbulb: Off`, invoked requests included.
    fn update_lightbulb(&mut self, position: PointUtf16, items: Vec<Item>, cx: &mut Context<Self>) {
        self.code_actions.lightbulb =
            if valid(&items).is_empty() || self.options.lightbulb == LightbulbMode::Off {
                None
            } else {
                Some(Lightbulb {
                    position,
                    auto_fix: has_auto_fix(&items),
                    items: std::rc::Rc::new(items),
                })
            };
        cx.notify();
    }

    /// Whether the lightbulb is showing.
    pub fn is_lightbulb_visible(&self) -> bool {
        self.code_actions.lightbulb.is_some()
    }

    /// Asks for code actions at the selection and shows or applies them
    /// (Monaco `CodeActionController.manualTriggerAtCurrentPosition`). With
    /// no actions, `not_available` is shown inline.
    pub fn trigger_code_actions(
        &mut self,
        filter: CodeActionFilter,
        auto_apply: CodeActionAutoApply,
        not_available: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if self.options.read_only {
            return;
        }
        self.close_message(cx);
        let not_available = not_available.into();
        let range = self.cursors.selections()[0].range();
        let position = self.cursors.primary().position;
        let request =
            self.request_code_actions(range, filter.clone(), CodeActionTrigger::Invoke, cx);
        self.code_actions.invoke = Some(cx.spawn(async move |this, cx| {
            let set = request.await;
            let items = set.items.clone();
            this.update(cx, |this, cx| {
                this.code_actions.invoke = None;
                if this.cursors.primary().position.row != position.row {
                    return;
                }
                this.update_lightbulb(position, items.clone(), cx);
                let valid_count = valid(&items).len();
                if filter.include.is_some() {
                    let apply = match auto_apply {
                        CodeActionAutoApply::First if valid_count >= 1 => {
                            valid(&items).first().copied()
                        }
                        CodeActionAutoApply::IfSingle if valid_count == 1 => {
                            valid(&items).first().copied()
                        }
                        _ => None,
                    };
                    if let Some(item) = apply.cloned() {
                        this.code_actions.lightbulb = None;
                        this.apply_code_action(item, false, false, cx);
                        return;
                    }
                    // What would have been applied is disabled: say why.
                    let would_apply = match auto_apply {
                        CodeActionAutoApply::First => valid_count == 0,
                        CodeActionAutoApply::IfSingle => items.len() == 1,
                        CodeActionAutoApply::Never => false,
                    };
                    if would_apply
                        && let Some(reason) =
                            items.iter().find_map(|item| item.action.disabled.clone())
                    {
                        this.show_message(reason, position, cx);
                        return;
                    }
                }
                let include_disabled = filter.include.is_some();
                if items.is_empty() || (!include_disabled && valid_count == 0) {
                    this.show_message(not_available, position, cx);
                    return;
                }
                let options = MenuOptions {
                    include_disabled,
                    from_lightbulb: false,
                };
                this.show_code_action_list(
                    std::rc::Rc::new(set),
                    MenuAnchor::Text(position),
                    options,
                    cx,
                );
            })
            .ok();
        }));
    }

    /// Cmd/Ctrl+.: every code action except source actions (Monaco
    /// `editor.action.quickFix`).
    pub fn quick_fix(&mut self, cx: &mut Context<Self>) {
        self.trigger_code_actions(
            CodeActionFilter::default(),
            CodeActionAutoApply::Never,
            "No code actions available",
            cx,
        );
    }

    /// Refactorings (Monaco `editor.action.refactor`).
    pub fn refactor(&mut self, cx: &mut Context<Self>) {
        self.trigger_code_actions(
            CodeActionFilter {
                include: Some(CodeActionKind::REFACTOR.into()),
                ..Default::default()
            },
            CodeActionAutoApply::Never,
            "No refactorings available",
            cx,
        );
    }

    /// Source actions (Monaco `editor.action.sourceAction`).
    pub fn source_action(&mut self, cx: &mut Context<Self>) {
        self.trigger_code_actions(
            CodeActionFilter {
                include: Some(CodeActionKind::SOURCE.into()),
                include_source_actions: true,
                ..Default::default()
            },
            CodeActionAutoApply::Never,
            "No source actions available",
            cx,
        );
    }

    /// Organize imports (Monaco `editor.action.organizeImports`).
    pub fn organize_imports(&mut self, cx: &mut Context<Self>) {
        self.trigger_code_actions(
            CodeActionFilter {
                include: Some(CodeActionKind::SOURCE_ORGANIZE_IMPORTS.into()),
                include_source_actions: true,
                ..Default::default()
            },
            CodeActionAutoApply::IfSingle,
            "No organize imports action available",
            cx,
        );
    }

    /// Fix all (Monaco `editor.action.fixAll`).
    pub fn fix_all(&mut self, cx: &mut Context<Self>) {
        self.trigger_code_actions(
            CodeActionFilter {
                include: Some(CodeActionKind::SOURCE_FIX_ALL.into()),
                include_source_actions: true,
                ..Default::default()
            },
            CodeActionAutoApply::IfSingle,
            "No fix all action available",
            cx,
        );
    }

    /// The preferred quick fix (Monaco `editor.action.autoFix`).
    pub fn auto_fix(&mut self, cx: &mut Context<Self>) {
        self.trigger_code_actions(
            CodeActionFilter {
                include: Some(CodeActionKind::QUICK_FIX.into()),
                only_preferred: true,
                ..Default::default()
            },
            CodeActionAutoApply::IfSingle,
            "No auto fixes available",
            cx,
        );
    }

    /// Monaco's `editor.action.codeAction` command: the code actions of
    /// `args.kind` (every action with a kind, source actions included,
    /// when unset), applied as `args.apply` says (`"ifSingle"` by default).
    pub fn code_action_with_args(&mut self, args: &TriggerCodeAction, cx: &mut Context<Self>) {
        let apply = match args.apply.as_deref().map(str::to_lowercase).as_deref() {
            Some("first") => CodeActionAutoApply::First,
            Some("never") => CodeActionAutoApply::Never,
            _ => CodeActionAutoApply::IfSingle,
        };
        let message = match (&args.kind, args.preferred) {
            (Some(kind), true) => format!("No preferred code actions for '{kind}' available"),
            (Some(kind), false) => format!("No code actions for '{kind}' available"),
            (None, true) => "No preferred code actions available".to_string(),
            (None, false) => "No code actions available".to_string(),
        };
        self.trigger_code_actions(
            CodeActionFilter {
                // Monaco's default is the empty kind, which keeps every
                // action that has a kind.
                include: Some(CodeActionKind::new(args.kind.clone().unwrap_or_default())),
                include_source_actions: true,
                only_preferred: args.preferred,
                ..Default::default()
            },
            apply,
            message,
            cx,
        );
    }

    /// The lightbulb was clicked at `click_x` (its middle when `None`):
    /// its actions in the menu, at that x and a third of a line below the
    /// bulb, a line further when the bulb is above the cursor's line, so
    /// the menu covers neither (Monaco `LightBulbWidget`'s mouse down).
    pub(crate) fn show_lightbulb_menu(
        &mut self,
        click_x: Option<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(lightbulb) = self.code_actions.lightbulb.clone() else {
            return;
        };
        // Monaco `autoRun`: a lone AI fix runs on the click.
        if let Some(item) = lightbulb_auto_run(&lightbulb).cloned() {
            self.apply_code_action(item, true, false, cx);
            return;
        }
        let set = CodeActionSet {
            items: lightbulb.items.to_vec(),
            documentation: Vec::new(),
        };
        let position = PointUtf16::new(lightbulb.position.row, 0);
        let anchor = match self.layout.as_ref().and_then(|layout| {
            let (bulb, row) = layout.lightbulb?;
            Some((bulb, row, layout.metrics.line_height))
        }) {
            Some((bulb, row, line_height)) => {
                let mut pad = (line_height / 3.).floor();
                if row < lightbulb.position.row {
                    pad += line_height;
                }
                let x = click_x.unwrap_or(bulb.center().x);
                // Below the bulb's bottom plus the pad, else above the bulb.
                MenuAnchor::Window {
                    bounds: gpui::Bounds::new(
                        gpui::point(x, bulb.top()),
                        gpui::size(px(0.), bulb.size.height + pad),
                    ),
                    position,
                }
            }
            None => MenuAnchor::Text(position),
        };
        let options = MenuOptions {
            include_disabled: false,
            from_lightbulb: true,
        };
        self.show_code_action_list(std::rc::Rc::new(set), anchor, options, cx);
    }

    /// Clicks the lightbulb (in its middle), for tests.
    #[doc(hidden)]
    pub fn show_lightbulb_menu_for_tests(&mut self, cx: &mut Context<Self>) {
        self.show_lightbulb_menu(None, cx);
    }

    /// Where the open code action menu is anchored in the window (`None`
    /// when at a text position) and the lightbulb's bounds, for tests.
    #[doc(hidden)]
    pub fn code_action_menu_anchor_for_tests(
        &self,
    ) -> (
        Option<gpui::Bounds<gpui::Pixels>>,
        Option<gpui::Bounds<gpui::Pixels>>,
    ) {
        let anchor = self
            .code_actions
            .menu
            .as_ref()
            .and_then(|(id, _)| self.widgets.content.iter().find(|w| w.id == *id))
            .and_then(|widget| widget.window_anchor);
        let bulb = self.layout.as_ref().and_then(|layout| layout.lightbulb);
        (anchor, bulb.map(|(bounds, _)| bounds))
    }

    /// Monaco `CodeActionController.showCodeActionList`: the valid actions
    /// (all of them with `include_disabled` when "Show Disabled" is on or
    /// none is valid), with an action bar unless from the lightbulb, below
    /// (else above) `at`.
    fn show_code_action_list(
        &mut self,
        set: std::rc::Rc<CodeActionSet>,
        at: MenuAnchor,
        options: MenuOptions,
        cx: &mut Context<Self>,
    ) {
        self.hide_code_action_menu(cx);
        // Monaco `toCoords`: a text position is revealed before the menu is
        // anchored there, so it doesn't open (and take the keys) off screen.
        if let MenuAnchor::Text(position) = at {
            self.reveal_range_with_scroll_type(
                position..position,
                crate::RevealType::Simple,
                crate::ScrollType::Immediate,
                cx,
            );
        }
        let valid_count = valid(&set.items).len();
        let items: Vec<Item> =
            if options.include_disabled && (self.code_actions.show_disabled || valid_count == 0) {
                set.items.clone()
            } else {
                valid(&set.items).into_iter().cloned().collect()
            };
        if items.is_empty() {
            return;
        }
        // Monaco `_getActionBarActions`.
        let mut bar = Vec::new();
        if !options.from_lightbulb {
            bar.extend(
                set.documentation
                    .iter()
                    .cloned()
                    .map(BarAction::Documentation),
            );
            if options.include_disabled && valid_count > 0 && set.items.len() != valid_count {
                bar.push(BarAction::ToggleDisabled {
                    showing: self.code_actions.show_disabled,
                });
            }
        }
        let show_headers = self.options.code_action_widget.show_headers;
        let rows = menu_rows(&items, show_headers);
        // The new menu takes the focus the old one gave back.
        self.refocus = false;
        let editor = cx.entity().downgrade();
        let colors = self.theme.colors.clone();
        let editor_focus = self.focus_handle.clone();
        let menu = cx.new(|cx| {
            let mut menu = CodeActionMenu {
                editor,
                items,
                rows,
                show_headers,
                set,
                at,
                options,
                bar,
                focused: None,
                colors,
                editor_focus: Some(editor_focus),
                scroll: gpui::ScrollHandle::new(),
                list_height: px(0.),
                list_width: px(0.),
                row_keys: Vec::new(),
                focus: cx.focus_handle(),
                focus_pending: true,
                room: Default::default(),
                _subscriptions: Vec::new(),
            };
            // The first actionable row.
            menu.focused = (0..menu.rows.len()).find(|row| menu.focusable(*row));
            menu
        });
        let (id, room) = self.add_overflow_content_widget(
            at.position(),
            vec![
                ContentWidgetPreference::Below,
                ContentWidgetPreference::Above,
            ],
            menu.clone().into(),
            crate::widgets::layer::MENU,
            cx,
        );
        if let MenuAnchor::Window { bounds, .. } = at {
            self.widgets.set_window_anchor(id, bounds);
        }
        // Monaco's list focuses its first action, which `onFocus` sees.
        let focused = menu.update(cx, |menu, _| {
            menu.room = room;
            menu.focused.and_then(|row| match menu.rows[row] {
                Row::Action(ix) => Some(menu.items[ix].action.clone()),
                Row::Header(_) => None,
            })
        });
        self.code_actions.menu = Some((id, menu));
        self.focus_code_action(focused.as_ref(), cx);
    }

    /// Whether the code action menu is open.
    pub fn is_code_action_menu_visible(&self) -> bool {
        self.code_actions.menu.is_some()
    }

    /// The rows of the open code action menu as shown: action titles (why
    /// a disabled action is, without headers), with group headers as
    /// `"# Title"`.
    pub fn code_action_menu_rows(&self, cx: &gpui::App) -> Vec<String> {
        let Some((_, menu)) = &self.code_actions.menu else {
            return Vec::new();
        };
        let menu = menu.read(cx);
        menu.rows
            .iter()
            .map(|row| match row {
                Row::Header(title) => format!("# {title}"),
                Row::Action(ix) => menu.row_label(&menu.items[*ix].action),
            })
            .collect()
    }

    /// The open menu's list size, whether its focused row is inside the
    /// list's visible part, and each row's keybinding label, as last
    /// rendered, for tests.
    #[doc(hidden)]
    pub fn code_action_menu_layout_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(gpui::Size<gpui::Pixels>, bool, Vec<Option<String>>)> {
        let menu = self.code_actions.menu.as_ref()?.1.read(cx);
        let list = menu.scroll.bounds();
        let offset = menu.scroll.offset().y;
        let visible = menu.focused.is_some_and(|row| {
            menu.scroll.bounds_for_item(row).is_some_and(|item| {
                // Item bounds are unscrolled.
                let top = item.top() + offset;
                top >= list.top() - px(0.5) && top + item.size.height <= list.bottom() + px(0.5)
            })
        });
        Some((
            gpui::size(menu.list_width, menu.list_height),
            visible,
            menu.row_keys
                .iter()
                .map(|keys| keys.as_ref().map(|k| k.text.to_string()))
                .collect(),
        ))
    }

    /// The lightbulb's title (Monaco `LightBulbWidget`'s), for tests.
    #[doc(hidden)]
    pub fn lightbulb_title_for_tests(&self, window: &Window) -> Option<String> {
        let bulb = self.code_actions.lightbulb.as_ref()?;
        let keys = |action: &dyn gpui::Action| {
            crate::keybinding_label::command_label(action, None, &self.focus_handle, window)
        };
        Some(lightbulb_title(
            bulb,
            keys(&AutoFix).as_deref(),
            keys(&QuickFix).as_deref(),
        ))
    }

    /// The labels of the open code action menu's action bar (Monaco's
    /// documentation links and "Show Disabled" / "Hide Disabled").
    pub fn code_action_menu_bar_actions(&self, cx: &gpui::App) -> Vec<SharedString> {
        let Some((_, menu)) = &self.code_actions.menu else {
            return Vec::new();
        };
        menu.read(cx).bar.iter().map(BarAction::label).collect()
    }

    /// Clicks the action bar button labelled `label` in the open code
    /// action menu. Returns whether there was one.
    pub fn run_code_action_menu_bar_action(&mut self, label: &str, cx: &mut Context<Self>) -> bool {
        let Some((_, menu)) = self.code_actions.menu.clone() else {
            return false;
        };
        let Some(ix) = menu
            .read(cx)
            .bar
            .iter()
            .position(|action| action.label() == label)
        else {
            return false;
        };
        // The menu calls back into the editor: run it after this update.
        cx.defer(move |cx| menu.update(cx, |menu, cx| menu.run_bar_action(ix, cx)));
        true
    }

    /// Closes the code action menu and focuses the editor.
    pub fn hide_code_action_menu(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.code_actions.menu.take() {
            self.remove_widget(id, cx);
            self.highlight_code_action(Vec::new(), cx);
            self.refocus = true;
        }
    }

    /// Monaco `CodeActionController`'s `onFocus`: highlights the focused
    /// action's `ranges` (its diagnostics instead when it fixes several),
    /// else its diagnostics, and then says where the first one is
    /// ("Context: {word} at line {line} and column {column}.").
    fn focus_code_action(&mut self, action: Option<&CodeAction>, cx: &mut Context<Self>) {
        let Some(action) = action else {
            self.highlight_code_action(Vec::new(), cx);
            return;
        };
        let diagnostics = || -> Vec<Range<PointUtf16>> {
            action
                .diagnostics
                .iter()
                .map(|marker| marker.range.clone())
                .collect()
        };
        if !action.ranges.is_empty() {
            // `fix all` with several diagnostics.
            let ranges = if action.diagnostics.len() > 1 {
                diagnostics()
            } else {
                action.ranges.clone()
            };
            self.highlight_code_action(ranges, cx);
        } else if let Some(first) = action.diagnostics.first() {
            self.highlight_code_action(diagnostics(), cx);
            let start = first.range.start;
            let word = self
                .model()
                .read(cx)
                .word_at_position(start)
                .map(|word| word.word)
                .unwrap_or_default();
            self.announce(
                format!(
                    "Context: {word} at line {} and column {}.",
                    start.row + 1,
                    start.column + 1
                ),
                cx,
            );
        } else {
            self.highlight_code_action(Vec::new(), cx);
        }
    }

    /// The code action highlight (Monaco's `quickfix-edit-highlight`) on
    /// `ranges`.
    fn highlight_code_action(&mut self, ranges: Vec<Range<PointUtf16>>, cx: &mut Context<Self>) {
        if let Some(mut highlight) = self.code_actions.highlight.take() {
            highlight.clear(cx);
        }
        if ranges.is_empty() {
            return;
        }
        let background = self.theme.colors.find_match_highlight_background;
        let border = self.theme.colors.find_match_highlight_border;
        let decorations = ranges
            .into_iter()
            .map(|range| {
                (
                    range,
                    DecorationOptions {
                        background: Some(background),
                        border,
                        ..Default::default()
                    },
                )
            })
            .collect();
        self.code_actions.highlight = Some(self.create_decorations_collection(decorations, cx));
    }

    /// Monaco `applyCodeAction`: resolves the action if it has no edit,
    /// applies its edit (through the preview handler if `preview`), then
    /// runs its command. `retrigger` asks for the lightbulb again
    /// afterwards.
    pub(crate) fn apply_code_action(
        &mut self,
        item: Item,
        retrigger: bool,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        let resolve = if item.action.edit.is_none() {
            item.provider.resolve_code_action(&item.action, cx)
        } else {
            None
        };
        let position = self.cursors.primary().position;
        self.code_actions.apply = Some(cx.spawn(async move |this, cx| {
            let mut action = item.action;
            if let Some(resolve) = resolve {
                match resolve.await {
                    Ok(resolved) => action.edit = resolved.edit,
                    Err(err) => log::warn!("resolving a code action failed: {err:#}"),
                }
            }
            let apply = this.update(cx, |this, cx| {
                action
                    .edit
                    .clone()
                    .filter(|edit| !edit.edits.is_empty())
                    .map(|edit| {
                        this.apply_workspace_edit_with_preview(
                            edit,
                            action.title.clone(),
                            preview,
                            cx,
                        )
                    })
            });
            if let Ok(Some(apply)) = apply {
                match apply.await {
                    Ok(true) => {}
                    // The preview was cancelled: no command either.
                    Ok(false) => return,
                    Err(err) => {
                        log::warn!("code action failed to apply: {err:#}");
                        return;
                    }
                }
            }
            this.update(cx, |this, cx| {
                this.code_actions.apply = None;
                if let Some(command) = &action.command
                    && let Err(err) = crate::commands::execute_command(cx, command)
                {
                    this.show_message(err.to_string(), position, cx);
                }
                if retrigger {
                    this.schedule_auto_code_actions(cx);
                }
            })
            .ok();
        }));
    }
}

/// The lightbulb's codicon (Monaco `computeLightBulbInfo`): `sparkle-filled`
/// when every action is an AI fix, the autofix variants when a preferred
/// quick fix exists, and the sparkle variants when some action is AI.
pub(crate) fn lightbulb_icon(bulb: &Lightbulb) -> Codicon {
    let valid = valid(&bulb.items);
    let all_ai = !valid.is_empty() && valid.iter().all(|item| item.action.is_ai);
    let has_ai = valid.iter().any(|item| item.action.is_ai);
    match (all_ai, bulb.auto_fix, has_ai) {
        (true, _, _) => Codicon::SparkleFilled,
        (false, true, true) => Codicon::LightbulbSparkleAutofix,
        (false, true, false) => Codicon::LightbulbAutofix,
        (false, false, true) => Codicon::LightbulbSparkle,
        (false, false, false) => Codicon::LightBulb,
    }
}

/// Monaco's lightbulb colors (`lightBulbWidget.css`): the AI color for
/// `sparkle-filled`, the autofix color for the autofix variants, else
/// `editorLightBulb.foreground`.
pub(crate) fn lightbulb_color(colors: &EditorColors, icon: Codicon) -> Hsla {
    match icon {
        Codicon::SparkleFilled => colors.lightbulb_ai_foreground,
        Codicon::LightbulbAutofix | Codicon::LightbulbSparkleAutofix => {
            colors.lightbulb_auto_fix_foreground
        }
        _ => colors.lightbulb_foreground,
    }
}

/// Monaco `computeLightBulbInfo`'s `autoRun`: every valid action is an AI
/// fix and there's one, so a click runs it.
fn lightbulb_auto_run(bulb: &Lightbulb) -> Option<&Item> {
    let valid = valid(&bulb.items);
    match valid.as_slice() {
        [only] if only.action.is_ai => Some(only),
        _ => None,
    }
}

/// Monaco `computeLightBulbInfo`'s title, with the Auto Fix
/// (`preferred_keys`) and Quick Fix (`quick_fix_keys`) keybindings:
/// "Run: {title}", "Show Code Actions. Preferred Quick Fix Available
/// ({keys})", "Show Code Actions ({keys})" or "Show Code Actions".
pub(crate) fn lightbulb_title(
    bulb: &Lightbulb,
    preferred_keys: Option<&str>,
    quick_fix_keys: Option<&str>,
) -> String {
    if let Some(item) = lightbulb_auto_run(bulb) {
        return format!("Run: {}", item.action.title);
    }
    match (bulb.auto_fix, preferred_keys, quick_fix_keys) {
        (true, Some(keys), _) => {
            format!("Show Code Actions. Preferred Quick Fix Available ({keys})")
        }
        (false, _, Some(keys)) => format!("Show Code Actions ({keys})"),
        _ => "Show Code Actions".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(title: &str, kind: Option<&str>, preferred: bool, diagnostics: bool) -> CodeAction {
        CodeAction {
            title: title.to_string().into(),
            kind: kind.map(CodeActionKind::from),
            is_preferred: preferred,
            diagnostics: if diagnostics {
                vec![crate::Marker {
                    severity: crate::MarkerSeverity::Error,
                    message: "x".into(),
                    range: PointUtf16::zero()..PointUtf16::zero(),
                    source: None,
                    code: None,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            },
            ..Default::default()
        }
    }

    #[test]
    fn filters() {
        let refactor = CodeActionFilter {
            include: Some("refactor".into()),
            ..Default::default()
        };
        assert!(refactor.keeps(&action("a", Some("refactor.extract"), false, false)));
        assert!(!refactor.keeps(&action("a", None, false, false)));
        assert!(!refactor.keeps(&action("a", Some("quickfix"), false, false)));
        // Source actions need opting in.
        let any = CodeActionFilter::default();
        assert!(!any.keeps(&action("a", Some("source.organizeImports"), false, false)));
        assert!(any.keeps(&action("a", None, false, false)));
        // An include more specific than an exclude wins.
        let filter = CodeActionFilter {
            include: Some("refactor.extract".into()),
            excludes: vec!["refactor".into()],
            ..Default::default()
        };
        assert!(filter.keeps(&action(
            "a",
            Some("refactor.extract.function"),
            false,
            false
        )));
        assert!(filter.may_include(&"refactor".into()));
        assert!(!refactor.may_include(&"quickfix".into()));
    }

    #[test]
    fn order_and_groups() {
        let provider: Arc<dyn CodeActionProvider> = Arc::new(Nothing);
        let mut items: Vec<Item> = [
            action("plain", None, false, false),
            action("extract", Some("refactor.extract"), false, false),
            action("fix", Some("quickfix"), false, true),
            action("best fix", Some("quickfix"), true, true),
            action("organize", Some("source.organizeImports"), false, false),
            CodeAction {
                is_ai: true,
                ..action("ai fix", Some("quickfix"), true, true)
            },
        ]
        .into_iter()
        .map(|action| Item {
            action,
            provider: provider.clone(),
        })
        .collect();
        sort_items(&mut items);
        let titles: Vec<_> = items.iter().map(|i| i.action.title.to_string()).collect();
        // AI actions last, even preferred fixes (Monaco `isAI`).
        assert_eq!(
            titles,
            ["best fix", "fix", "plain", "extract", "organize", "ai fix"]
        );
        assert!(has_auto_fix(&items));
        let rows: Vec<String> = menu_rows(&items, true)
            .iter()
            .map(|row| match row {
                Row::Header(title) => format!("# {title}"),
                Row::Action(ix) => items[*ix].action.title.to_string(),
            })
            .collect();
        assert_eq!(
            rows,
            [
                "# Quick Fix",
                "best fix",
                "fix",
                "ai fix",
                "# Extract",
                "extract",
                "# Source Action",
                "organize",
                "# More Actions...",
                "plain"
            ]
        );
        // Without headers, in order (Monaco `showHeaders: false`).
        let rows: Vec<String> = menu_rows(&items, false)
            .iter()
            .map(|row| match row {
                Row::Header(title) => format!("# {title}"),
                Row::Action(ix) => items[*ix].action.title.to_string(),
            })
            .collect();
        assert_eq!(rows, titles);
    }

    #[test]
    fn documentation_and_previews() {
        let command = |id: &str| Command {
            id: id.to_string().into(),
            title: id.to_string().into(),
            arguments: Vec::new(),
            ..Default::default()
        };
        let docs = vec![
            (CodeActionKind::from("refactor"), command("refactor")),
            ("refactor.extract".into(), command("extract")),
            ("source".into(), command("source")),
        ];
        let id = |found: Option<Command>| found.map(|c| c.id.to_string());
        let extract = [action("a", Some("refactor.extract.constant"), false, false)];
        // Monaco `getDocumentationFromProvider`: the most specific entry
        // containing what was asked for.
        let only = CodeActionKind::from("refactor.extract.constant");
        assert_eq!(
            id(documentation_for(&docs, &[], Some(&only))),
            Some("extract".into())
        );
        let only = CodeActionKind::from("refactor");
        assert_eq!(
            id(documentation_for(&docs, &extract, Some(&only))),
            Some("refactor".into())
        );
        // Else the first entry containing an action's kind.
        assert_eq!(
            id(documentation_for(&docs, &extract, None)),
            Some("refactor".into())
        );
        let fix = [action("a", Some("quickfix"), false, false)];
        assert_eq!(id(documentation_for(&docs, &fix, None)), None);
        assert_eq!(id(documentation_for(&[], &extract, None)), None);

        // Refactorings and source actions can be previewed, as can edits.
        assert!(can_preview(&extract[0]));
        assert!(can_preview(&action(
            "a",
            Some("source.organizeImports"),
            false,
            false
        )));
        assert!(!can_preview(&fix[0]));
        let mut with_edit = fix[0].clone();
        with_edit.edit = Some(crate::providers::WorkspaceEdit {
            edits: vec![(
                None,
                vec![(PointUtf16::zero()..PointUtf16::zero(), "x".into())],
            )],
            ..Default::default()
        });
        assert!(can_preview(&with_edit));
    }

    struct Nothing;
    impl CodeActionProvider for Nothing {
        fn provide_code_actions(
            &self,
            _: &crate::providers::ProviderContext,
            _: Range<PointUtf16>,
            _: &CodeActionContext,
            _: &mut gpui::App,
        ) -> Task<anyhow::Result<Vec<CodeAction>>> {
            Task::ready(Ok(Vec::new()))
        }
    }

    /// Monaco `computeLightBulbInfo` and `lightBulbWidget.css`: the title
    /// and color for each kind of bulb.
    #[test]
    fn lightbulb_titles_and_colors() {
        let bulb = |actions: Vec<CodeAction>, auto_fix: bool| Lightbulb {
            position: PointUtf16::zero(),
            items: std::rc::Rc::new(
                actions
                    .into_iter()
                    .map(|action| Item {
                        action,
                        provider: Arc::new(Nothing),
                    })
                    .collect(),
            ),
            auto_fix,
        };
        let plain = bulb(
            vec![action("Inline", Some("refactor"), false, false)],
            false,
        );
        assert_eq!(
            lightbulb_title(&plain, Some("⌥⌘."), Some("⌘.")),
            "Show Code Actions (⌘.)"
        );
        assert_eq!(lightbulb_title(&plain, None, None), "Show Code Actions");
        let fix = bulb(vec![action("Fix", Some("quickfix"), true, false)], true);
        assert_eq!(
            lightbulb_title(&fix, Some("⌥⌘."), Some("⌘.")),
            "Show Code Actions. Preferred Quick Fix Available (⌥⌘.)"
        );
        // A lone AI fix runs on the click.
        let mut ai = action("Ask AI", Some("quickfix"), false, false);
        ai.is_ai = true;
        let ai = bulb(vec![ai], false);
        assert_eq!(lightbulb_title(&ai, None, Some("⌘.")), "Run: Ask AI");
        assert!(lightbulb_auto_run(&ai).is_some());
        assert!(lightbulb_auto_run(&plain).is_none());
        let colors = crate::theme::Theme::vs_dark().colors;
        assert_eq!(
            lightbulb_color(&colors, lightbulb_icon(&ai)),
            colors.lightbulb_ai_foreground
        );
        assert_eq!(
            lightbulb_color(&colors, lightbulb_icon(&fix)),
            colors.lightbulb_auto_fix_foreground
        );
        assert_eq!(
            lightbulb_color(&colors, lightbulb_icon(&plain)),
            colors.lightbulb_foreground
        );
    }

    #[test]
    fn lightbulb_placement() {
        let text = ["fn main() {", "    let x = 1;", "    x;", "}", ""];
        let lines = |row: u32| text.get(row as usize).unwrap_or(&"").to_string();
        let place = |row, column| {
            place_lightbulb(
                PointUtf16::new(row, column),
                text.len() as u32,
                &lines,
                &|_| false,
                4,
                7.,
                false,
            )
        };
        // Four spaces of indentation (28px) fit the bulb at the line start.
        assert_eq!(place(1, 6), Some(LightbulbPlacement::Content(1, 0)));
        // Line 0 before an indented line: inside that line's indentation.
        assert_eq!(place(0, 3), Some(LightbulbPlacement::Content(1, 0)));
        // `}` after an indented line: on the line above.
        assert_eq!(place(3, 0), Some(LightbulbPlacement::Content(2, 0)));

        // Code all around: the gutter, unless a glyph is there.
        let text = ["use a;", "use b;", "use c;"];
        let lines = |row: u32| text[row as usize].to_string();
        let place = |row, has_glyph| {
            place_lightbulb(
                PointUtf16::new(row, 3),
                3,
                &lines,
                &|_| false,
                4,
                7.,
                has_glyph,
            )
        };
        assert_eq!(place(0, false), Some(LightbulbPlacement::Gutter(0)));
        assert_eq!(place(1, false), Some(LightbulbPlacement::Gutter(1)));
        assert_eq!(place(1, true), Some(LightbulbPlacement::Content(1, 0)));
        // The last line too (Monaco's `endLine` has no next line to
        // block); with a glyph there, the line above.
        assert_eq!(place(2, false), Some(LightbulbPlacement::Gutter(2)));
        assert_eq!(place(2, true), Some(LightbulbPlacement::Content(1, 0)));

        // Nowhere above or below: inline unless over the cursor, with
        // Monaco's 1-based column (3 * 7px < 22px, 4 * 7px isn't).
        let text = ["x", "  y"];
        let lines = |row: u32| text[row as usize].to_string();
        let place = |column| {
            place_lightbulb(
                PointUtf16::new(0, column),
                2,
                &lines,
                &|row| row == 1,
                4,
                7.,
                false,
            )
        };
        assert_eq!(place(2), None);
        assert_eq!(place(3), Some(LightbulbPlacement::Content(0, 1)));
    }
}
