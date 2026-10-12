//! Cursor undo/redo (Monaco's `contrib/cursorUndo`), the selection anchor
//! (`contrib/anchorSelect`) and the editor scroll commands
//! (`scrollLineUp`, `scrollPageDown`, … from `browser/coreCommands.ts`).

use gpui::{Context, Pixels, point, px};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::decorations::{DecorationId, DecorationOptions};
use crate::editor::CodeEditor;
use crate::view::reveal::ScrollType;

/// Monaco keeps this many cursor states.
const MAX_CURSOR_UNDO: usize = 50;

/// Monaco's `.selection-anchor` color (`anchorSelect.css`), not a theme
/// color.
fn selection_anchor_color() -> gpui::Hsla {
    gpui::rgb(0x007acc).into()
}

#[derive(Clone)]
struct CursorStackElement {
    selections: Vec<Selection>,
    scroll_top: Pixels,
    scroll_left: Pixels,
}

/// Monaco `CursorUndoRedoController` state, plus the selection anchor.
#[derive(Default)]
pub(crate) struct NavigationHistory {
    undo: Vec<CursorStackElement>,
    redo: Vec<CursorStackElement>,
    /// The selections and text version after the last cursor change.
    last: Option<(Vec<Selection>, text::Version)>,
    /// Set while applying a cursor undo/redo so it isn't recorded.
    restoring: bool,
    anchor: Option<DecorationId>,
}

impl NavigationHistory {
    /// History whose current state is `selections` at `version`.
    pub(crate) fn starting_at(selections: Vec<Selection>, version: text::Version) -> Self {
        Self {
            last: Some((selections, version)),
            ..Default::default()
        }
    }
}

impl CodeEditor {
    /// Records the previous cursor state for cursor undo (Monaco's
    /// `onDidChangeCursorSelection` listener). Edits clear the history.
    pub(crate) fn record_cursor_change(&mut self) {
        let current = self.cursors.selections();
        let version = self.snapshot.version();
        let history = &mut self.navigation;
        match history.last.take() {
            Some((previous, previous_version)) if previous_version == version => {
                if !history.restoring
                    && previous != current
                    && history
                        .undo
                        .last()
                        .is_none_or(|top| top.selections != previous)
                {
                    history.undo.push(CursorStackElement {
                        selections: previous,
                        scroll_top: self.scroll_top,
                        scroll_left: self.scroll_left,
                    });
                    history.redo.clear();
                    if history.undo.len() > MAX_CURSOR_UNDO {
                        history.undo.remove(0);
                    }
                }
            }
            Some(_) => {
                history.undo.clear();
                history.redo.clear();
            }
            None => {}
        }
        history.last = Some((current, version));
    }

    /// Forgets cursor history and the anchor (for a new model).
    pub(crate) fn reset_navigation_history(&mut self) {
        self.navigation.undo.clear();
        self.navigation.redo.clear();
        self.navigation.last = None;
        self.navigation.anchor = None;
    }

    fn current_stack_element(&self) -> CursorStackElement {
        CursorStackElement {
            selections: self.cursors.selections(),
            scroll_top: self.scroll_top,
            scroll_left: self.scroll_left,
        }
    }

    fn restore_stack_element(&mut self, element: CursorStackElement, cx: &mut Context<Self>) {
        self.navigation.restoring = true;
        self.set_selections(&element.selections, cx);
        self.navigation.restoring = false;
        self.autoscroll_requested = false;
        self.scroll_top = element.scroll_top;
        self.scroll_left = element.scroll_left;
        self.emit_scroll_changed(cx);
    }

    /// Goes back to the previous cursor state (Monaco `cursorUndo`, Cmd+U).
    pub fn cursor_undo(&mut self, cx: &mut Context<Self>) {
        let Some(element) = self.navigation.undo.pop() else {
            return;
        };
        let current = self.current_stack_element();
        self.navigation.redo.push(current);
        self.restore_stack_element(element, cx);
    }

    /// Reapplies an undone cursor state (Monaco `cursorRedo`).
    pub fn cursor_redo(&mut self, cx: &mut Context<Self>) {
        let Some(element) = self.navigation.redo.pop() else {
            return;
        };
        let current = self.current_stack_element();
        self.navigation.undo.push(current);
        self.restore_stack_element(element, cx);
    }

    // ----- selection anchor -------------------------------------------------

    /// Where the selection anchor is, if set.
    pub fn selection_anchor(&self, cx: &gpui::App) -> Option<PointUtf16> {
        let id = self.navigation.anchor?;
        let model = self.model().read(cx);
        let decoration = model.decoration(id)?;
        Some(model.buffer().offset_to_point_utf16(decoration.range.start))
    }

    /// Marks the primary cursor's position as the selection anchor (Monaco
    /// `editor.action.setSelectionAnchor`, Cmd+K Cmd+B).
    pub fn set_selection_anchor(&mut self, cx: &mut Context<Self>) {
        let position = self.cursors.primary().position;
        let offset = self.snapshot.rope().point_utf16_to_offset(position);
        let old: Vec<DecorationId> = self.navigation.anchor.take().into_iter().collect();
        let owner = Some(cx.entity_id());
        let ids = self.model().update(cx, |model, cx| {
            model.delta_decorations(
                owner,
                &old,
                vec![(
                    offset..offset,
                    // Monaco's `.selection-anchor`: a 2px `#007ACC` bar
                    // (`width: 2px !important` on the empty range).
                    DecorationOptions {
                        background: Some(selection_anchor_color()),
                        min_width: px(2.),
                        // Monaco `MarkdownString.appendText`.
                        hover_message: vec![crate::providers::MarkdownString::from_plain_text(
                            "Selection Anchor",
                        )],
                        stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
                        ..Default::default()
                    },
                )],
                cx,
            )
        });
        self.navigation.anchor = ids.first().copied();
        // Monaco alerts where the anchor is (1-based).
        self.announce(
            format!("Anchor set at {}:{}", position.row + 1, position.column + 1),
            cx,
        );
    }

    /// Moves the cursor to the selection anchor (Monaco
    /// `editor.action.goToSelectionAnchor`).
    pub fn go_to_selection_anchor(&mut self, cx: &mut Context<Self>) {
        if let Some(anchor) = self.selection_anchor(cx) {
            self.set_selections(&[Selection::collapsed(anchor)], cx);
        }
    }

    /// Selects from the anchor to the primary cursor and clears the anchor
    /// (Monaco `editor.action.selectFromAnchorToCursor`, Cmd+K Cmd+K).
    pub fn select_from_anchor_to_cursor(&mut self, cx: &mut Context<Self>) {
        let Some(anchor) = self.selection_anchor(cx) else {
            return;
        };
        let head = self.cursors.primary().position;
        self.set_selections(&[Selection { anchor, head }], cx);
        self.cancel_selection_anchor(cx);
    }

    /// Removes the selection anchor (Monaco
    /// `editor.action.cancelSelectionAnchor`); returns whether there was one.
    pub fn cancel_selection_anchor(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.navigation.anchor.take() else {
            return false;
        };
        self.model().update(cx, |model, cx| {
            model.delta_decorations(None, &[id], Vec::new(), cx)
        });
        cx.notify();
        true
    }

    // ----- scrolling ----------------------------------------------------------

    fn line_height(&self) -> Pixels {
        self.layout
            .as_ref()
            .map_or(self.options.resolved_line_height(), |l| {
                l.metrics.line_height
            })
    }

    /// Scrolls by `delta_y` and `delta_x` from where the scroll is now,
    /// without moving the cursors (Monaco `editorScroll`): smoothly, when
    /// `smooth_scrolling` is on (`ScrollType.Smooth`), clamped to the last
    /// frame's scroll range.
    pub fn scroll_by(&mut self, delta_y: Pixels, delta_x: Pixels, cx: &mut Context<Self>) {
        // Monaco `getCurrentScrollTop() + delta` on the axis that moves;
        // the other keeps where it's going.
        let (current, mut position) = (self.scroll_position(), self.future_scroll_position());
        if delta_y != px(0.) {
            position.y = (current.y + delta_y).max(px(0.));
        }
        if delta_x != px(0.) {
            position.x = (current.x + delta_x).max(px(0.));
        }
        self.set_scroll_position_with_scroll_type(position, ScrollType::Smooth, cx);
    }

    /// Scrolls by `lines` lines (Monaco `scrollLineUp`/`Down`).
    pub fn scroll_lines(&mut self, lines: f32, cx: &mut Context<Self>) {
        let delta = self.line_height() * lines;
        self.scroll_by(delta, px(0.), cx);
    }

    /// Scrolls by `pages` pages (Monaco `scrollPageUp`/`Down`): Monaco's
    /// `pageSize` lines a page, two less than fit, so two lines of the
    /// last page stay in view.
    pub fn scroll_pages(&mut self, pages: f32, cx: &mut Context<Self>) {
        let rows = self.page_lines() as f32;
        self.scroll_lines(rows * pages, cx);
    }

    /// Scrolls to the top or bottom (Monaco `scrollEditorTop`/`Bottom`),
    /// smoothly when `smooth_scrolling` is on: the bottom puts line
    /// `lineCount - pageSize` (1-based) at the top, so the last line ends
    /// a page down rather than at the top of an empty viewport. Monaco
    /// passes that model line count to `getVerticalOffsetForLineNumber`,
    /// which takes a view line; so does this.
    pub fn scroll_to_edge(&mut self, bottom: bool, cx: &mut Context<Self>) {
        let line_count = self.snapshot.line_count() as i64;
        let line = if bottom {
            line_count - self.page_lines() as i64
        } else {
            0
        };
        let top = match self.layout.as_ref() {
            Some(layout) => {
                let last = self.view_lines.line_count().saturating_sub(1) as i64;
                let view_line = (line - 1).clamp(0, last) as u32;
                layout.zones.line_y(view_line, layout.metrics.line_height)
            }
            // Clamped to the content on the first frame.
            None if bottom => px(1e9),
            None => px(0.),
        };
        let left = self.future_scroll_position().x;
        self.set_scroll_position_with_scroll_type(point(left, top), ScrollType::Smooth, cx);
    }
}
