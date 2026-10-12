//! Indentation commands (Monaco `contrib/indentation/browser/indentation.ts`,
//! MIT): converting indentation between tabs and spaces, choosing the
//! indent size, and detecting it from the text.

use std::ops::Range;

use gpui::{Context, SharedString, Window};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::model::ModelOptions;

/// Monaco `getIndentationEditOperations`: each line's indentation with tabs
/// turned into `tab_size` spaces, or runs of `tab_size` spaces into tabs.
fn indentation_edits(
    rope: &text::Rope,
    tab_size: u32,
    tabs_to_spaces: bool,
) -> Vec<(Range<PointUtf16>, String)> {
    // An empty model has nothing to convert.
    if rope.line_count() == 1 && rope.line_len_utf16(0) == 0 {
        return Vec::new();
    }
    let spaces = " ".repeat(tab_size as usize);
    let mut edits = Vec::new();
    for row in 0..rope.line_count() {
        let line = rope.line(row);
        let indentation: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if indentation.is_empty() {
            continue;
        }
        let end = indentation.encode_utf16().count() as u32;
        let new = if tabs_to_spaces {
            indentation.replace('\t', &spaces)
        } else if spaces.is_empty() {
            indentation.clone()
        } else {
            indentation.replace(&spaces, "\t")
        };
        edits.push((PointUtf16::new(row, 0)..PointUtf16::new(row, end), new));
    }
    edits
}

impl CodeEditor {
    /// Converts the indentation of every line to spaces and indents with
    /// spaces from now on (Monaco `editor.action.indentationToSpaces`).
    pub fn indentation_to_spaces(&mut self, cx: &mut Context<Self>) {
        self.convert_indentation(true, cx);
    }

    /// Converts the indentation of every line to tabs and indents with tabs
    /// from now on (Monaco `editor.action.indentationToTabs`).
    pub fn indentation_to_tabs(&mut self, cx: &mut Context<Self>) {
        self.convert_indentation(false, cx);
    }

    fn convert_indentation(&mut self, to_spaces: bool, cx: &mut Context<Self>) {
        if self.options.read_only {
            return;
        }
        let model = self.model().clone();
        let tab_size = model.read(cx).options().tab_size;
        let edits = indentation_edits(self.snapshot.rope(), tab_size, to_spaces);
        // Monaco tracks the selections like decorations: a point inside a
        // line's replaced indentation keeps its offset (clamped to the new
        // indentation), and one after it moves with the change in length.
        let map = |p: PointUtf16| {
            let Some((range, new)) = edits.iter().find(|(range, _)| range.start.row == p.row)
            else {
                return p;
            };
            let old_len = range.end.column;
            let new_len = new.encode_utf16().count() as u32;
            let column = if p.column <= old_len {
                p.column.min(new_len)
            } else {
                p.column - old_len + new_len
            };
            PointUtf16::new(p.row, column)
        };
        let selections: Vec<_> = self
            .selections()
            .iter()
            .map(|s| crate::cursor::Selection::new(map(s.anchor), map(s.head)))
            .collect();
        self.push_undo_stop(cx);
        if !edits.is_empty() {
            self.execute_edits(edits, Some(selections), cx);
        }
        self.push_undo_stop(cx);
        model.update(cx, |model, cx| {
            let options = ModelOptions {
                insert_spaces: to_spaces,
                ..model.options().clone()
            };
            model.update_options(options, cx);
        });
    }

    /// Asks for a tab size, 1 to 8, and indents with it using spaces (Monaco
    /// `editor.action.indentUsingSpaces`), tabs (`indentUsingTabs`), or, with
    /// `insert_spaces: None`, only changes how wide tabs show
    /// (`changeTabDisplaySize`).
    pub fn change_indentation_size(
        &mut self,
        insert_spaces: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.model().read(cx).options().tab_size;
        let default = ModelOptions::default().tab_size;
        // Monaco's descriptions of the configured and current sizes.
        let items = (1..=8u32)
            .map(|n| {
                let note = match (n == default, n == current) {
                    (true, true) => "  Configured Tab Size",
                    (true, false) => "  Default Tab Size",
                    (false, true) => "  Current Tab Size",
                    (false, false) => "",
                };
                SharedString::from(format!("{n}{note}"))
            })
            .collect();
        // Monaco focuses the current size (`activeItem`).
        let active = current.clamp(1, 8) as usize - 1;
        self.show_quick_pick_with_active_item(
            Some("Select Tab Size for Current File".into()),
            items,
            Some(active),
            move |editor, index, _, cx| {
                let size = index as u32 + 1;
                editor.model().clone().update(cx, |model, cx| {
                    let mut options = model.options().clone();
                    options.tab_size = size;
                    // `changeTabDisplaySize` keeps the indent size, which
                    // may follow the tab size.
                    if let Some(insert_spaces) = insert_spaces {
                        options.indent_size = size;
                        options.indent_size_is_tab_size = false;
                        options.insert_spaces = insert_spaces;
                    }
                    model.update_options(options, cx);
                });
            },
            window,
            cx,
        );
    }

    /// Guesses the indentation from the text (Monaco
    /// `editor.action.detectIndentation`), falling back to the default
    /// model options.
    pub fn detect_indentation(&mut self, cx: &mut Context<Self>) {
        let defaults = ModelOptions::default();
        self.model().clone().update(cx, |model, cx| {
            model.detect_indentation(defaults.insert_spaces, defaults.tab_size, cx)
        });
    }
}
