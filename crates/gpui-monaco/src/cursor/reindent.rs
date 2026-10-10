//! Reindenting lines with the language's indentation rules.
//!
//! Ports `getReindentEditOperations` from Monaco's
//! `src/vs/editor/contrib/indentation/common/indentation.ts` and the
//! `ReindentLinesAction` / `ReindentSelectedLinesAction` from
//! `contrib/indentation/browser/indentation.ts` (MIT). Rows are 0-based;
//! the comments use Monaco's 1-based line numbers where the port keeps its
//! arithmetic.

use std::ops::Range;

use super::edit::{EditOperation, execute_edits};
use super::language::{LanguageContext, LanguageSupport, shift_indent, unshift_indent};
use super::{CursorCollection, CursorConfig, CursorModel, Position, StandardTokenType, strings};

/// Monaco `getReindentEditOperations` for rows `start_row..=end_row`.
fn reindent_edits(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    mut start_row: u32,
    end_row: u32,
) -> Vec<(Range<Position>, String)> {
    if model.line_count() == 1 && model.line_max_column(0) == 0 {
        return Vec::new();
    }
    if !lang.has_indentation_rules() {
        return Vec::new();
    }
    let end_row = end_row.min(model.line_count() - 1);
    let processed = |row: u32| lang.processed_line(model, row);
    // Skip `unIndentedLinePattern` lines.
    while start_row <= end_row && lang.should_ignore(&processed(start_row)) {
        start_row += 1;
    }
    if start_row + 1 > end_row {
        return Vec::new();
    }

    // The first line's indentation is the base.
    let first = model.line_content(start_row);
    let mut global_indent = strings::leading_whitespace(&first).to_string();
    let mut ideal_for_next_line = global_indent.clone();
    if lang.should_increase(&processed(start_row)) {
        ideal_for_next_line = shift_indent(config, &ideal_for_next_line);
        global_indent = shift_indent(config, &global_indent);
    } else if lang.should_indent_next_line(&processed(start_row)) {
        ideal_for_next_line = shift_indent(config, &ideal_for_next_line);
    }

    let mut edits = Vec::new();
    for row in start_row + 1..=end_row {
        // Monaco `doesLineStartWithString`: leave multi-line strings alone.
        if lang.token_at(model, row, 0) == StandardTokenType::String {
            continue;
        }
        let text = model.line_content(row);
        let old_indentation = strings::leading_whitespace(&text);
        let line = processed(row);
        if lang.should_decrease(&line) {
            ideal_for_next_line = unshift_indent(config, &ideal_for_next_line);
            global_indent = unshift_indent(config, &global_indent);
        }
        if old_indentation != ideal_for_next_line {
            edits.push((
                Position::new(row, 0)..Position::new(row, strings::utf16_len(old_indentation)),
                config.normalize_indentation(&ideal_for_next_line),
            ));
        }
        if lang.should_ignore(&line) {
            // Inherit from the lines above without changing the indents.
            continue;
        } else if lang.should_increase(&line) {
            global_indent = shift_indent(config, &global_indent);
            ideal_for_next_line = global_indent.clone();
        } else if lang.should_indent_next_line(&line) {
            ideal_for_next_line = shift_indent(config, &ideal_for_next_line);
        } else {
            ideal_for_next_line = global_indent.clone();
        }
    }
    edits
}

/// Reindents the whole document with the language's indentation rules
/// (Monaco `editor.action.reindentlines`). `None` if nothing changes.
pub fn reindent_lines(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    lang: &LanguageContext,
) -> Option<EditOperation> {
    let model: &dyn CursorModel = model;
    let lang = LanguageSupport::new(lang);
    let edits = reindent_edits(config, &lang, model, 0, model.line_count() - 1);
    (!edits.is_empty()).then(|| execute_edits(model, &cursors.selections(), edits, None))
}

/// Reindents the selected lines, using the line above each selection as the
/// base (Monaco `editor.action.reindentselectedlines`). `None` if nothing
/// changes.
pub fn reindent_selected_lines(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    lang: &LanguageContext,
) -> Option<EditOperation> {
    let model: &dyn CursorModel = model;
    let lang = LanguageSupport::new(lang);
    let selections = cursors.selections();
    let mut edits = Vec::new();
    for selection in &selections {
        let mut start = selection.start().row;
        let mut end = selection.end().row;
        if start != end && selection.end().column == 0 {
            end -= 1;
        }
        if start == 0 {
            if start == end {
                continue;
            }
        } else {
            start -= 1;
        }
        edits.extend(reindent_edits(config, &lang, model, start, end));
    }
    (!edits.is_empty()).then(|| execute_edits(model, &selections, edits, None))
}
