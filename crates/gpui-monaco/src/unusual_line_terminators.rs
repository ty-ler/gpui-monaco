//! Unusual line terminators (Monaco `UnusualLineTerminatorsDetector`, MIT):
//! the Line Separator and Paragraph Separator characters, which some tools
//! treat as line breaks. They are removed, or the user is asked.

use gpui::{Context, Window};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::options::UnusualLineTerminators;

/// Monaco `strings.UNUSUAL_LINE_TERMINATORS`: LS and PS.
fn is_unusual_line_terminator(ch: char) -> bool {
    matches!(ch, '\u{2028}' | '\u{2029}')
}

/// Whether `text` has an unusual line terminator.
pub(crate) fn contains_unusual_line_terminators(text: &str) -> bool {
    text.contains(is_unusual_line_terminator)
}

impl CodeEditor {
    /// Checks again before the next frame (after a model change, an edit
    /// inserting one, or an option change).
    pub(crate) fn request_unusual_line_terminators_check(&mut self) {
        self.unusual_line_terminators_pending = true;
    }

    /// Monaco `_checkForUnusualLineTerminators`. Called before painting.
    pub(crate) fn check_unusual_line_terminators(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !std::mem::take(&mut self.unusual_line_terminators_pending)
            || self.options.unusual_line_terminators == UnusualLineTerminators::Off
            || self.options.read_only
        {
            return;
        }
        let model = self.model().read(cx);
        // This model was ignored.
        if model.ignore_unusual_line_terminators
            || !model
                .buffer()
                .rope()
                .chunks()
                .any(contains_unusual_line_terminators)
        {
            return;
        }
        if self.options.unusual_line_terminators == UnusualLineTerminators::Auto {
            self.remove_unusual_line_terminators(cx);
            return;
        }
        // Monaco asks in a dialog; dismissing it ignores this model, like
        // its Ignore button.
        self.model()
            .update(cx, |model, _| model.ignore_unusual_line_terminators = true);
        let editor = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            let Some(editor) = editor.upgrade() else {
                return;
            };
            editor.update(cx, |editor, cx| {
                editor.show_quick_pick(
                    Some(
                        "Detected unusual line terminators: Line Separator (LS) or Paragraph \
                         Separator (PS) characters, which can break other tools"
                            .into(),
                    ),
                    vec!["Remove Unusual Line Terminators".into(), "Ignore".into()],
                    |editor, ix, _, cx| {
                        if ix == 0 {
                            editor.model().update(cx, |model, _| {
                                model.ignore_unusual_line_terminators = false
                            });
                            editor.remove_unusual_line_terminators(cx);
                        }
                    },
                    window,
                    cx,
                );
            });
        });
    }

    /// Monaco `removeUnusualLineTerminators`: deletes every LS and PS
    /// character, as one undoable edit.
    pub fn remove_unusual_line_terminators(&mut self, cx: &mut Context<Self>) {
        let rope = self.model().read(cx).buffer().rope().clone();
        let mut edits = Vec::new();
        let mut offset = 0;
        for chunk in rope.chunks() {
            for (ix, ch) in chunk.char_indices() {
                if is_unusual_line_terminator(ch) {
                    let start = rope.offset_to_point_utf16(offset + ix);
                    let end = PointUtf16::new(start.row, start.column + 1);
                    edits.push((start..end, String::new()));
                }
            }
            offset += chunk.len();
        }
        if !edits.is_empty() {
            self.execute_edits(edits, None, cx);
        }
    }
}
