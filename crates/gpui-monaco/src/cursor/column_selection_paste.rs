//! Pasting a column selection's rows as a block.
//!
//! Ports Monaco's
//! `src/vs/editor/common/commands/columnSelectionPasteCommand.ts` (MIT).

use super::edit::{Command, CursorStateHelper, EditBuilder, ReplaceOvertypeCommand};
use super::{CursorModel, Position, Selection, strings};

/// Monaco `ColumnSelectionPasteCommand`: puts the rows of a copied column
/// selection on successive lines at the visible column where `selection`
/// starts. The first row replaces `selection`; the rest are inserted on the
/// lines after its end, padding short lines with spaces, and rows past the
/// last line are appended as new lines. Every row is padded to the widest
/// so the block stays rectangular.
pub(super) struct ColumnSelectionPasteCommand {
    pub selection: Selection,
    pub text: Vec<String>,
    pub tab_size: u32,
    pub overtype: bool,
}

impl ColumnSelectionPasteCommand {
    /// Monaco `_padTextToBlockWidth`.
    fn pad_text_to_block_width(&self, visible_column: u32) -> Vec<String> {
        let prefix = " ".repeat(visible_column as usize);
        let end_visible_columns: Vec<u32> = self
            .text
            .iter()
            .map(|text| {
                let with_prefix = format!("{prefix}{text}");
                strings::visible_column_from_column(
                    &with_prefix,
                    strings::utf16_len(&with_prefix),
                    self.tab_size,
                )
            })
            .collect();
        let block_end = end_visible_columns
            .iter()
            .copied()
            .fold(visible_column, u32::max);
        self.text
            .iter()
            .zip(end_visible_columns)
            .map(|(text, end)| format!("{text}{}", " ".repeat((block_end - end) as usize)))
            .collect()
    }

    fn replace(
        &self,
        model: &dyn CursorModel,
        builder: &mut EditBuilder,
        range: std::ops::Range<Position>,
        text: String,
    ) {
        if self.overtype {
            ReplaceOvertypeCommand { range, text }.edit_operations(model, builder);
        } else {
            builder.add_edit(range, &text);
        }
    }
}

impl Command for ColumnSelectionPasteCommand {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let start = self.selection.start();
        let visible_column = strings::visible_column_from_column(
            &model.line_content(start.row),
            start.column,
            self.tab_size,
        );
        let text = self.pad_text_to_block_width(visible_column);
        let line_count = model.line_count();
        let end_row = self.selection.end().row;
        for (i, row_text) in text.iter().enumerate() {
            let row = end_row + i as u32;
            if row >= line_count {
                let last = line_count - 1;
                let end = Position::new(last, model.line_max_column(last));
                let padding = " ".repeat(visible_column as usize);
                let separator = format!("\n{padding}");
                let appended = format!("{separator}{}", text[i..].join(&separator));
                builder.add_edit(end..end, &appended);
                break;
            }
            if i == 0 {
                self.replace(model, builder, self.selection.range(), row_text.clone());
                continue;
            }
            let line = model.line_content(row);
            let column =
                strings::column_from_visible_column(&line, visible_column as i64, self.tab_size);
            let at = Position::new(row, column);
            let mut row_text = row_text.clone();
            if column == model.line_max_column(row) {
                let end_visible_column =
                    strings::visible_column_from_column(&line, column, self.tab_size);
                let padding = visible_column.saturating_sub(end_visible_column);
                row_text.insert_str(0, &" ".repeat(padding as usize));
            }
            self.replace(model, builder, at..at, row_text);
        }
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        // After the last row.
        helper
            .inverse_edit_ranges()
            .last()
            .map_or(self.selection, |range| Selection::collapsed(range.end))
    }
}
