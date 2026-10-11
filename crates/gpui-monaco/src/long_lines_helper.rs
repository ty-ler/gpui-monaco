//! Clicking past where a long line stops rendering turns the limit off
//! (Monaco `contrib/longLinesHelper/browser/longLinesHelper.ts`, MIT).

use gpui::Context;

use crate::editor::CodeEditor;
use crate::mouse_target::{EditorMouseEvent, MouseTargetKind};

impl CodeEditor {
    /// Monaco `LongLinesHelper`'s `onMouseDown`: a press on text at or past
    /// column `stop_rendering_line_after` (1-based, as Monaco counts) sets
    /// it to `None`, so the whole line draws.
    pub(crate) fn long_lines_mouse_down(
        &mut self,
        mouse: &EditorMouseEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(limit) = self.options.stop_rendering_line_after else {
            return;
        };
        let Some(position) = mouse.target.position else {
            return;
        };
        if mouse.target.kind == MouseTargetKind::ContentText && position.column + 1 >= limit {
            let mut options = self.options().clone();
            options.stop_rendering_line_after = None;
            self.update_options(options, cx);
        }
    }
}
