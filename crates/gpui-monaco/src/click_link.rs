//! The Cmd/Ctrl+click gesture shared by links, go to definition and inlay
//! hint label parts (Monaco's `ClickLinkGesture`): the modifier is noted
//! when the mouse goes down, over a row, and the gesture runs when the
//! mouse comes up on the same row. The ordinary mouse-down still runs, so
//! Cmd/Ctrl+drag selects; a drag, a selection, a scroll or an edit in
//! between cancels it.
//!
//! Ports Monaco's `contrib/gotoSymbol/browser/link/clickLinkGesture.ts`
//! (MIT).

use gpui::{Context, MouseButton};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::mouse_target::{EditorMouseEvent, MouseTargetKind};
use crate::options::MouseMiddleClickAction;

/// What the last mouse-down armed (Monaco `ClickLinkGesture`'s
/// `_hasTriggerKeyOnMouseDown` and `_lineNumberOnMouseDown`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClickLinkGesture {
    /// The trigger modifier was held when the mouse went down, or a middle
    /// click acts as a Cmd/Ctrl+click (`mouseMiddleClickAction:
    /// ctrlLeftClick`). Pressing it after the mouse went down (to copy a
    /// selection, say) doesn't count.
    has_trigger_key_on_mouse_down: bool,
    /// The row the mouse went down on; `None` off the text and gutter.
    row_on_mouse_down: Option<u32>,
}

/// Monaco `ClickLinkMouseEvent`: a mouse event's buttons and modifiers as
/// the gesture reads them, with a middle click redirected to a Cmd/Ctrl+
/// left click under `mouseMiddleClickAction: ctrlLeftClick`.
struct ClickLinkMouseEvent {
    has_trigger_modifier: bool,
    has_side_by_side_modifier: bool,
    is_left_click: bool,
    /// A middle click that opens links (`mouseMiddleClickAction:
    /// openLink`).
    is_open_link_middle_click: bool,
    is_none_or_single_mouse_down: bool,
}

impl CodeEditor {
    fn click_link_mouse_event(&self, event: &EditorMouseEvent) -> ClickLinkMouseEvent {
        let middle = event.button == Some(MouseButton::Middle);
        let action = self.options.mouse_middle_click_action;
        let ctrl_left_click = middle && action == MouseMiddleClickAction::CtrlLeftClick;
        ClickLinkMouseEvent {
            has_trigger_modifier: ctrl_left_click || self.is_definition_modifier(&event.modifiers),
            has_side_by_side_modifier: self.is_side_by_side_modifier(&event.modifiers),
            is_left_click: event.button == Some(MouseButton::Left) || ctrl_left_click,
            is_open_link_middle_click: middle && action == MouseMiddleClickAction::OpenLink,
            is_none_or_single_mouse_down: event.click_count <= 1,
        }
    }

    /// Monaco `_onEditorMouseDown`: notes the modifier and row.
    pub(crate) fn click_link_mouse_down(&mut self, event: &EditorMouseEvent) {
        self.click_link = ClickLinkGesture {
            has_trigger_key_on_mouse_down: self.click_link_mouse_event(event).has_trigger_modifier,
            row_on_mouse_down: event.target.position.map(|position| position.row),
        };
    }

    /// Monaco `_resetHandler`: a drag, a selection, a scroll, an edit or a
    /// new model cancels the gesture, and the Cmd/Ctrl+hover links go
    /// (`onCancel`).
    pub(crate) fn reset_click_link(&mut self, cx: &mut Context<Self>) {
        self.click_link = ClickLinkGesture::default();
        if self.has_definition_link() {
            self.clear_definition_link(cx);
        }
        if self.has_active_link() {
            self.links_mouse_move(None, false, cx);
        }
        self.cancel_inlay_hint_link(cx);
    }

    /// Monaco `_onDidChangeCursorSelection`: selecting cancels the gesture
    /// (Monaco only looks at the primary selection's columns).
    pub(crate) fn click_link_selection_changed(
        &mut self,
        selection: std::ops::Range<PointUtf16>,
        cx: &mut Context<Self>,
    ) {
        if selection.start.column != selection.end.column {
            self.reset_click_link(cx);
        }
    }

    /// Monaco `_onEditorMouseUp`: on the row the mouse went down on, with
    /// the trigger modifier held then (or a middle click that opens
    /// links), runs the gesture there (`onExecute`). Returns whether it
    /// ran.
    pub(crate) fn click_link_mouse_up(
        &mut self,
        event: &EditorMouseEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let gesture = std::mem::take(&mut self.click_link);
        let mouse = self.click_link_mouse_event(event);
        let same_row = gesture.row_on_mouse_down.is_some()
            && gesture.row_on_mouse_down == event.target.position.map(|position| position.row);
        if !same_row || !(gesture.has_trigger_key_on_mouse_down || mouse.is_open_link_middle_click)
        {
            return false;
        }
        // Monaco `InlayHintsController`: a label part opens its location or
        // runs its command.
        if let Some(part) = self.inlay_hint_part_at(event.target.point) {
            return self.inlay_hint_link_click(part, cx);
        }
        let (MouseTargetKind::ContentText, Some(position)) =
            (event.target.kind, event.target.position)
        else {
            return false;
        };
        // Monaco `LinkDetector.onEditorMouseUp`.
        if (mouse.has_trigger_modifier || mouse.is_open_link_middle_click)
            && self.follow_link_at(position, cx)
        {
            return true;
        }
        // Monaco `GotoDefinitionAtPositionEditorContribution.isEnabled`.
        if mouse.is_left_click
            && mouse.is_none_or_single_mouse_down
            && mouse.has_trigger_modifier
            && event.target.injected_text.is_none()
            && self.has_definition_providers(cx)
        {
            self.go_to_definition_from_mouse(position, mouse.has_side_by_side_modifier, cx);
            return true;
        }
        false
    }
}
