//! A tiny stand-in for Monaco's `withTestCodeEditor` plus the
//! `CoreNavigationCommands`/`CoreEditingCommands` the tests call. All
//! coordinates are Monaco's 1-based (line, column) so ported assertions read
//! one to one.

use std::sync::Arc;

use syntax::LanguageConfiguration;
use text::Buffer;

use super::tokenizer::Tokenizer;
use crate::cursor::{
    self, CursorCollection, CursorConfig, EditOperation, EditOperationType, LanguageContext,
    MoveCommand, Position, Selection, StandardTokenType, WordNavigationType,
};
use crate::model::{AutoWhitespaceLines, shift_past_trimmed};

/// Monaco `new Range(startLineNumber, startColumn, endLineNumber,
/// endColumn)`.
pub type MonacoRange = (u32, u32, u32, u32);

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
    /// The language configuration, like Monaco's `languageId` option.
    pub language: Option<LanguageConfiguration>,
    /// Where token types come from (only used with a `language`).
    pub tokenizer: Tokenizer,
    /// Monaco's `trimAutoWhitespace` model option (default `true`).
    pub trim_auto_whitespace: bool,
    auto_whitespace: AutoWhitespaceLines,
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
            language: None,
            tokenizer: Tokenizer::None,
            trim_auto_whitespace: true,
            auto_whitespace: AutoWhitespaceLines::default(),
        }
    }

    /// Sets Monaco's `trimAutoWhitespace` model option.
    pub fn with_trim_auto_whitespace(mut self, trim: bool) -> Self {
        self.trim_auto_whitespace = trim;
        self
    }

    /// Sets the language configuration (Monaco's `languageId`).
    pub fn with_language(mut self, config: LanguageConfiguration) -> Self {
        self.language = Some(config);
        self
    }

    pub fn with_tokenizer(mut self, tokenizer: Tokenizer) -> Self {
        self.tokenizer = tokenizer;
        self
    }

    fn model_lines(&self) -> Vec<String> {
        let rope = self.buffer.rope();
        (0..rope.line_count()).map(|row| rope.line(row)).collect()
    }

    /// Runs `f` with the language context for the current text (`None`
    /// without a language).
    pub fn with_lang<R>(&self, f: impl FnOnce(Option<&LanguageContext>) -> R) -> R {
        let Some(config) = &self.language else {
            return f(None);
        };
        let lines = self.model_lines();
        let tokens = self.tokenizer.tokenize(&lines);
        let at = |p: Position| -> StandardTokenType {
            if p.column == 0 {
                return StandardTokenType::Other;
            }
            tokens
                .get(p.row as usize)
                .and_then(|line| line.get(p.column as usize - 1))
                .copied()
                .unwrap_or_default()
        };
        let inserting = |p: Position, ch: char| -> StandardTokenType {
            let mut lines = lines.clone();
            let line = &mut lines[p.row as usize];
            let mut units: Vec<u16> = line.encode_utf16().collect();
            let column = (p.column as usize).min(units.len());
            let mut buf = [0u16; 2];
            let inserted = ch.encode_utf16(&mut buf);
            units.splice(column..column, inserted.iter().copied());
            *line = String::from_utf16_lossy(&units);
            self.tokenizer.tokenize(&lines)[p.row as usize]
                .get(column)
                .copied()
                .unwrap_or_default()
        };
        let ctx = LanguageContext::new(config, &at).with_token_type_if_inserting(&inserting);
        f(Some(&ctx))
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
        self.auto_whitespace.clear();
        self.prev = None;
        self.cursors.auto_closed_ranges_mut().clear();
        let buffer = &self.buffer;
        self.cursors.validate(buffer);
    }

    /// Monaco `model.setValue`: replaces the text and resets the cursors.
    pub fn set_value(&mut self, text: &str) {
        let len = self.buffer.rope().len();
        self.buffer.edit([(0..len, text)]).unwrap();
        self.auto_whitespace.clear();
        self.prev = None;
        self.cursors = CursorCollection::default();
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
    pub fn apply(&mut self, mut op: EditOperation) {
        if op.push_stack_element_before {
            self.buffer.push_stack_element();
        }
        let follow_up = op.follow_up.clone();
        self.cursors
            .track_auto_closed(&self.buffer, &op.edits, &op.auto_closed);
        if !op.edits.is_empty() {
            let rope = self.buffer.rope();
            let edits: Vec<_> = op
                .edits
                .iter()
                .enumerate()
                .map(|(i, (range, text))| {
                    (
                        rope.point_utf16_to_offset(range.start)
                            ..rope.point_utf16_to_offset(range.end),
                        text.clone(),
                        op.auto_whitespace.contains(&i),
                    )
                })
                .collect();
            let before: Vec<Selection> = self.selections();
            let cursor_rows: Vec<_> = before.iter().map(|s| s.start().row..=s.end().row).collect();
            let previous_transaction = self.buffer.last_transaction_id();
            let applied = self
                .auto_whitespace
                .edit(
                    &mut self.buffer,
                    self.trim_auto_whitespace,
                    Some(&cursor_rows),
                    edits,
                    Some(0),
                )
                .unwrap();
            shift_past_trimmed(&mut op.selections_after, &applied.trimmed);
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
        if let Some(follow_up) = follow_up {
            self.apply(*follow_up);
        }
    }

    /// Monaco `viewModel.type(text, 'keyboard')`: one character (grapheme) at
    /// a time.
    pub fn type_keyboard(&mut self, text: &str) {
        use unicode_segmentation::UnicodeSegmentation;
        for g in text.graphemes(true) {
            let op = self.with_lang(|lang| {
                cursor::type_with_language(
                    &self.config,
                    &self.buffer,
                    &self.cursors,
                    g,
                    self.prev,
                    lang,
                )
            });
            self.apply(op);
        }
    }

    /// Monaco `viewModel.type(text)` from a non-keyboard source.
    pub fn type_raw(&mut self, text: &str) {
        let buffer = &self.buffer;
        let op = cursor::type_text(&self.config, buffer, &self.cursors, text, self.prev);
        self.apply(op);
    }

    pub fn line_break(&mut self) {
        let op = self.with_lang(|lang| {
            cursor::line_break_with_language(&self.config, &self.buffer, &self.cursors, lang)
        });
        self.apply(op);
    }

    pub fn delete_left(&mut self) {
        let op = self.with_lang(|lang| {
            cursor::delete_left_with_language(
                &self.config,
                &self.buffer,
                &self.cursors,
                self.prev,
                lang,
            )
        });
        self.apply(op);
    }

    pub fn delete_right(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::delete_right(&self.config, buffer, &self.cursors, self.prev);
        self.apply(op);
    }

    pub fn tab(&mut self) {
        let op = self.with_lang(|lang| {
            cursor::tab_with_language(&self.config, &self.buffer, &self.cursors, lang)
        });
        self.apply(op);
    }

    pub fn outdent(&mut self) {
        let op = self.with_lang(|lang| {
            cursor::outdent_with_language(&self.config, &self.buffer, &self.cursors, lang)
        });
        self.apply(op);
    }

    pub fn indent_lines(&mut self) {
        let op = self.with_lang(|lang| {
            cursor::indent_lines_with_language(&self.config, &self.buffer, &self.cursors, lang)
        });
        self.apply(op);
    }

    pub fn outdent_lines(&mut self) {
        let op = self.with_lang(|lang| {
            cursor::outdent_lines_with_language(&self.config, &self.buffer, &self.cursors, lang)
        });
        self.apply(op);
    }

    /// Monaco `editor.action.insertLineAfter`.
    pub fn insert_line_after(&mut self) {
        let op = self.with_lang(|lang| {
            cursor::insert_line_below_with_language(&self.config, &self.buffer, &self.cursors, lang)
        });
        self.apply(op);
    }

    /// Monaco `editor.action.insertLineBefore`.
    pub fn insert_line_before(&mut self) {
        let op = self.with_lang(|lang| {
            cursor::insert_line_above_with_language(&self.config, &self.buffer, &self.cursors, lang)
        });
        self.apply(op);
    }

    // --- IME composition

    /// Monaco `viewModel.startComposition()`.
    pub fn start_composition(&mut self) {
        let buffer = &self.buffer;
        self.cursors.start_composition(buffer);
    }

    /// Monaco `viewModel.compositionType(text, replacePrevCharCnt,
    /// replaceNextCharCnt, positionDelta)`.
    pub fn composition_type(&mut self, text: &str, prev: u32, next: u32, delta: i32) {
        let buffer = &self.buffer;
        let op =
            cursor::composition_type(buffer, &self.cursors, text, prev, next, delta, self.prev);
        self.apply(op);
    }

    /// Monaco `viewModel.endComposition('keyboard')`.
    pub fn end_composition(&mut self) {
        let Some(composition) = self.cursors.end_composition() else {
            return;
        };
        let op = self.with_lang(|lang| {
            cursor::composition_end_with_language(
                &self.config,
                &self.buffer,
                &self.cursors,
                &composition,
                lang,
            )
        });
        if let Some(op) = op {
            self.apply(op);
        }
    }

    /// Monaco `viewModel.executeEdits('snippet', edits, () => selections)`:
    /// the edits, then the closing characters they end with tracked as
    /// auto-closed.
    pub fn execute_snippet_edits(
        &mut self,
        edits: &[(MonacoRange, &str)],
        selections: &[Selection],
    ) {
        let edits: Vec<_> = edits
            .iter()
            .map(|(r, text)| (pos(r.0, r.1)..pos(r.2, r.3), text.to_string()))
            .collect();
        let op = EditOperation {
            selections_after: selections.to_vec(),
            ..EditOperation::from_edits(&self.buffer, &self.selections(), edits.clone())
        };
        // Where each edit's text starts once applied (the edits are sorted
        // and on one line each here).
        let mut inserted = Vec::new();
        let mut shift: (u32, i64) = (u32::MAX, 0);
        for (range, text) in &edits {
            if shift.0 != range.start.row {
                shift = (range.start.row, 0);
            }
            let start = Position::new(
                range.start.row,
                (range.start.column as i64 + shift.1) as u32,
            );
            shift.1 += text.encode_utf16().count() as i64
                - (range.end.column as i64 - range.start.column as i64);
            inserted.push((start, text.as_str()));
        }
        let pairs = self.with_lang(|lang| {
            lang.map(|lang| cursor::snippet_auto_closed_pairs(lang, &inserted))
                .unwrap_or_default()
        });
        self.apply(op);
        self.cursors.auto_closed_ranges_mut().push(&pairs);
    }

    pub fn cut(&mut self) {
        let buffer = &self.buffer;
        let op = cursor::cut(&self.config, buffer, &self.cursors);
        self.apply(op);
    }

    pub fn copy(&self) -> (String, bool, Vec<String>) {
        self.copy_with_eol(text::LineEnding::Lf, false)
    }

    pub fn copy_with_eol(
        &self,
        eol: text::LineEnding,
        force_crlf: bool,
    ) -> (String, bool, Vec<String>) {
        cursor::copy_text(&self.config, &self.buffer, &self.cursors, eol, force_crlf)
    }

    pub fn paste(
        &mut self,
        text: &str,
        pasted_from_empty_selection: bool,
        multicursor_text: Option<&[String]>,
    ) {
        self.paste_block(text, pasted_from_empty_selection, multicursor_text, false);
    }

    /// [`Self::paste`] with Monaco's `isBlock` paste payload.
    pub fn paste_block(
        &mut self,
        text: &str,
        pasted_from_empty_selection: bool,
        multicursor_text: Option<&[String]>,
        is_block: bool,
    ) {
        let buffer = &self.buffer;
        let op = cursor::paste(
            &self.config,
            buffer,
            &self.cursors,
            text,
            pasted_from_empty_selection,
            multicursor_text,
            is_block,
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
        self.auto_whitespace.clear();
        if let Some(outcome) = self.buffer.undo()
            && let Some(meta) = outcome.metadata
            && let Ok(selections) = meta.downcast::<Vec<Selection>>()
        {
            self.cursors.set_selections(&selections);
        }
        self.prev = None;
        self.cursors.auto_closed_ranges_mut().clear();
        let buffer = &self.buffer;
        self.cursors.validate(buffer);
    }

    pub fn redo(&mut self) {
        self.auto_whitespace.clear();
        if let Some(outcome) = self.buffer.redo()
            && let Some(meta) = outcome.metadata
            && let Ok(selections) = meta.downcast::<Vec<Selection>>()
        {
            self.cursors.set_selections(&selections);
        }
        self.prev = None;
        self.cursors.auto_closed_ranges_mut().clear();
        let buffer = &self.buffer;
        self.cursors.validate(buffer);
    }
}
