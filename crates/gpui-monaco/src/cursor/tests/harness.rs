//! A tiny stand-in for Monaco's `withTestCodeEditor` plus the
//! `CoreNavigationCommands`/`CoreEditingCommands` the tests call. All
//! coordinates are Monaco's 1-based (line, column) so ported assertions read
//! one to one.

use std::sync::Arc;

use text::Buffer;

use crate::cursor::{
    self, CursorCollection, CursorConfig, EditOperation, EditOperationType, MoveCommand, Position,
    Selection, WordNavigationType,
};

/// Monaco `Position(lineNumber, column)`.
pub fn pos(line: u32, column: u32) -> Position {
    Position::from_monaco(line, column)
}

/// Monaco `new Selection(selectionStartLineNumber, selectionStartColumn,
/// positionLineNumber, positionColumn)`.
pub fn sel(start_line: u32, start_column: u32, line: u32, column: u32) -> Selection {
    Selection::new(pos(start_line, start_column), pos(line, column))
}

/// Monaco's `Selection.toString()`, for readable assertion failures.
pub fn fmt_sel(s: &Selection) -> String {
    let (al, ac) = s.anchor.to_monaco();
    let (hl, hc) = s.head.to_monaco();
    format!("[{al},{ac} -> {hl},{hc}]")
}

pub struct TestEditor {
    pub buffer: Buffer,
    pub cursors: CursorCollection,
    pub config: CursorConfig,
    pub prev: Option<EditOperationType>,
    pub page_lines: u32,
}

impl TestEditor {
    pub fn new(text: &str) -> Self {
        Self::with_config(text, CursorConfig::default())
    }

    pub fn lines(lines: &[&str]) -> Self {
        Self::new(&lines.join("\n"))
    }

    pub fn with_config(text: &str, config: CursorConfig) -> Self {
        Self {
            buffer: Buffer::new(text),
            cursors: CursorCollection::default(),
            config,
            prev: None,
            page_lines: 10,
        }
    }

    pub fn lines_with_config(lines: &[&str], config: CursorConfig) -> Self {
        Self::with_config(&lines.join("\n"), config)
    }

    // --- model

    pub fn value(&self) -> String {
        self.buffer.rope().to_string()
    }

    pub fn line(&self, line: u32) -> String {
        self.buffer.rope().line(line - 1)
    }

    pub fn line_count(&self) -> u32 {
        self.buffer.rope().line_count()
    }

    /// An edit made outside the editor (Monaco `model.applyEdits`): cursors
    /// are validated afterwards.
    pub fn apply_external_edit(&mut self, range: (u32, u32, u32, u32), text: &str) {
        let rope = self.buffer.rope();
        let start = rope.point_utf16_to_offset(pos(range.0, range.1));
        let end = rope.point_utf16_to_offset(pos(range.2, range.3));
        self.buffer.edit([(start..end, text)]).unwrap();
        self.prev = None;
        let buffer = &self.buffer;
        self.cursors.validate(buffer);
    }

    // --- cursors

    pub fn selections(&self) -> Vec<Selection> {
        self.cursors.selections()
    }

    pub fn set_selections(&mut self, selections: &[Selection]) {
        self.cursors.set_selections(selections);
        let buffer = &self.buffer;
        self.cursors.validate(buffer);
    }

    /// Monaco `editor.setPosition`.
    pub fn set_position(&mut self, line: u32, column: u32) {
        self.set_selections(&[Selection::collapsed(pos(line, column))]);
    }

    pub fn set_selection(&mut self, s: Selection) {
        self.set_selections(&[s]);
    }

    pub fn position(&self) -> Position {
        self.cursors.primary().position
    }

    #[track_caller]
    pub fn assert_cursor(&self, line: u32, column: u32) {
        self.assert_selections(&[sel(line, column, line, column)]);
    }

    #[track_caller]
    pub fn assert_selection(&self, s: Selection) {
        self.assert_selections(&[s]);
    }

    #[track_caller]
    pub fn assert_selections(&self, expected: &[Selection]) {
        let actual: Vec<String> = self.selections().iter().map(fmt_sel).collect();
        let expected: Vec<String> = expected.iter().map(fmt_sel).collect();
        assert_eq!(actual, expected);
    }

    // --- navigation (each pushes an undo stop, like Monaco's commands)

    pub fn move_cursors(&mut self, command: MoveCommand, select: bool) {
        self.buffer.push_stack_element();
        let buffer = &self.buffer;
        cursor::move_cursors(
            &self.config,
            buffer,
            &mut self.cursors,
            command,
            select,
            self.page_lines,
        );
    }

    /// Monaco `CoreNavigationCommands.MoveTo` / `MoveToSelect`.
    pub fn move_to(&mut self, line: u32, column: u32, select: bool) {
        self.buffer.push_stack_element();
        let buffer = &self.buffer;
        cursor::move_to(
            &self.config,
            buffer,
            &mut self.cursors,
            pos(line, column),
            select,
        );
    }

    pub fn move_left(&mut self, select: bool) {
        self.move_cursors(MoveCommand::Left, select);
    }

    pub fn move_right(&mut self, select: bool) {
        self.move_cursors(MoveCommand::Right, select);
    }

    pub fn move_up(&mut self, select: bool) {
        self.move_cursors(MoveCommand::Up, select);
    }

    pub fn move_down(&mut self, select: bool) {
        self.move_cursors(MoveCommand::Down, select);
    }

    pub fn move_to_beginning_of_line(&mut self, select: bool) {
        self.move_cursors(MoveCommand::Home, select);
    }

    pub fn move_to_end_of_line(&mut self, select: bool) {
        self.move_cursors(MoveCommand::End, select);
    }

    pub fn move_to_beginning_of_buffer(&mut self, select: bool) {
        self.move_cursors(MoveCommand::Top, select);
    }

    pub fn move_to_end_of_buffer(&mut self, select: bool) {
        self.move_cursors(MoveCommand::Bottom, select);
    }

    pub fn move_word(&mut self, left: bool, nav: WordNavigationType, select: bool) {
        self.buffer.push_stack_element();
        let buffer = &self.buffer;
        cursor::move_word(&self.config, buffer, &mut self.cursors, left, nav, select);
    }

    pub fn select_all(&mut self) {
        let buffer = &self.buffer;
        cursor::select_all(buffer, &mut self.cursors);
    }

    pub fn word_select(&mut self, line: u32, column: u32, select: bool) {
        self.buffer.push_stack_element();
        let buffer = &self.buffer;
        cursor::select_word(
            &self.config,
            buffer,
            &mut self.cursors,
            pos(line, column),
            select,
        );
    }

    pub fn line_select(&mut self, line: u32, column: u32, select: bool) {
        self.buffer.push_stack_element();
        let buffer = &self.buffer;
        cursor::select_line(buffer, &mut self.cursors, pos(line, column), select);
    }

    pub fn add_cursor_below(&mut self) {
        self.buffer.push_stack_element();
        let buffer = &self.buffer;
        cursor::add_cursor_below(&self.config, buffer, &mut self.cursors);
    }

    pub fn add_cursor_above(&mut self) {
        self.buffer.push_stack_element();
        let buffer = &self.buffer;
        cursor::add_cursor_above(&self.config, buffer, &mut self.cursors);
    }

    // --- editing

    /// Applies an operation the way Monaco's cursor controller does: undo
    /// stops, one atomic batch of edits, then the new selections.
    pub fn apply(&mut self, op: EditOperation) {
        if op.push_stack_element_before {
            self.buffer.push_stack_element();
        }
        if !op.edits.is_empty() {
            let rope = self.buffer.rope();
            let edits: Vec<_> = op
                .edits
                .iter()
                .map(|(range, text)| {
                    (
                        rope.point_utf16_to_offset(range.start)
                            ..rope.point_utf16_to_offset(range.end),
                        text.clone(),
                    )
                })
                .collect();
            let before: Vec<Selection> = self.selections();
            let previous_transaction = self.buffer.last_transaction_id();
            self.buffer.edit_grouped(edits, 0).unwrap();
            let new_transaction = self.buffer.last_transaction_id() != previous_transaction;
            self.buffer.set_last_transaction_metadata(
                new_transaction.then(|| Arc::new(before) as text::TransactionMetadata),
                Some(Arc::new(op.selections_after.clone())),
            );
        }
        if !op.edits.is_empty()
            || matches!(
                op.kind,
                EditOperationType::DeletingLeft | EditOperationType::DeletingRight
            )
        {
            self.prev = Some(op.kind);
        }
        self.cursors.set_selections(&op.selections_after);
        let buffer = &self.buffer;
        self.cursors.validate(buffer);
        if op.push_stack_element_after {
            self.buffer.push_stack_element();
        }
    }

    /// Monaco `viewModel.type(text, 'keyboard')`: one character (grapheme) at
    /// a time.
    pub fn type_keyboard(&mut self, text: &str) {
        use unicode_segmentation::UnicodeSegmentation;
        for g in text.graphemes(true) {
            self.type_raw(g);
        }
    }

    /// Monaco `viewModel.type(text)` from a non-keyboard source.
    pub fn type_raw(&mut self, text: &str) {
        let buffer = &self.buffer;
        let op = cursor::type_text(&self.config, buffer, &self.cursors, text, self.prev);
        self.apply(op);
    }

    pub fn line_break(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::line_break(&self.config, buffer, &self.cursors);
        self.apply(op);
    }

    pub fn delete_left(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::delete_left(&self.config, buffer, &self.cursors, self.prev);
        self.apply(op);
    }

    pub fn delete_right(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::delete_right(&self.config, buffer, &self.cursors, self.prev);
        self.apply(op);
    }

    pub fn tab(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::tab(&self.config, buffer, &self.cursors);
        self.apply(op);
    }

    pub fn outdent(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::outdent(&self.config, buffer, &self.cursors);
        self.apply(op);
    }

    pub fn indent_lines(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::indent_lines(&self.config, buffer, &self.cursors);
        self.apply(op);
    }

    pub fn cut(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::cut(&self.config, buffer, &self.cursors);
        self.apply(op);
    }

    pub fn copy(&self) -> (String, bool, Vec<String>) {
        cursor::copy_text(&self.config, &self.buffer, &self.cursors)
    }

    pub fn paste(
        &mut self,
        text: &str,
        pasted_from_empty_selection: bool,
        multicursor_text: Option<&[String]>,
    ) {
        let buffer = &self.buffer;
        let op = cursor::paste(
            &self.config,
            buffer,
            &self.cursors,
            text,
            pasted_from_empty_selection,
            multicursor_text,
        );
        self.apply(op);
    }

    pub fn run(
        &mut self,
        f: impl FnOnce(&CursorConfig, &Buffer, &CursorCollection) -> EditOperation,
    ) {
        let op = f(&self.config, &self.buffer, &self.cursors);
        self.apply(op);
    }

    /// Monaco `model.undo()`: reverts the last undo step and restores the
    /// selections from before it.
    pub fn undo(&mut self) {
        if let Some(outcome) = self.buffer.undo()
            && let Some(meta) = outcome.metadata
            && let Ok(selections) = meta.downcast::<Vec<Selection>>()
        {
            self.cursors.set_selections(&selections);
        }
        self.prev = None;
        let buffer = &self.buffer;
        self.cursors.validate(buffer);
    }

    pub fn redo(&mut self) {
        if let Some(outcome) = self.buffer.redo()
            && let Some(meta) = outcome.metadata
            && let Ok(selections) = meta.downcast::<Vec<Selection>>()
        {
            self.cursors.set_selections(&selections);
        }
        self.prev = None;
        let buffer = &self.buffer;
        self.cursors.validate(buffer);
    }
}
