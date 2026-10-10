//! Re-indenting pasted text with the language's indentation rules.
//!
//! Ports `AutoIndentOnPaste.trigger`, `AutoIndentOnPasteCommand` and
//! `isStartOrEndInString` from Monaco's
//! `src/vs/editor/contrib/indentation/browser/indentation.ts`, and
//! `getSpaceCnt` / `generateIndent` from
//! `src/vs/editor/contrib/indentation/common/indentUtils.ts` (MIT). Rows are
//! 0-based; the comments use Monaco's 1-based line numbers where the port
//! keeps its arithmetic.
//!
//! Not ported: Monaco also gives up when the first pasted line is not cheap
//! to tokenize (`isCheapToTokenize`); here the token types always come from
//! the [`LanguageContext`].

use std::ops::Range;

use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType, TrackedId,
    execute_commands,
};
use super::language::{
    IndentConsts, LanguageContext, LanguageSupport, VirtualModel, indent_metadata, is_blank,
    rules_good_indent_for_line,
};
use super::{
    AutoIndent, CursorCollection, CursorConfig, CursorModel, Position, Selection,
    StandardTokenType, strings,
};

/// Monaco `indentUtils.getSpaceCnt`: the width of `indentation` with tabs
/// counted as `tab_size` columns.
fn space_count(indentation: &str, tab_size: u32) -> i64 {
    indentation
        .encode_utf16()
        .map(|c| {
            if c == u16::from(b'\t') {
                tab_size as i64
            } else {
                1
            }
        })
        .sum()
}

/// Monaco `indentUtils.generateIndent`.
fn generate_indent(spaces: i64, tab_size: u32, insert_spaces: bool) -> String {
    let mut spaces = spaces.max(0) as usize;
    let mut result = String::new();
    if !insert_spaces {
        let tab_size = tab_size.max(1) as usize;
        result.push_str(&"\t".repeat(spaces / tab_size));
        spaces %= tab_size;
    }
    result.push_str(&" ".repeat(spaces));
    result
}

/// Monaco `AutoIndentOnPaste.rangeContainsOnlyWhitespaceCharacters`.
fn range_contains_only_whitespace(model: &dyn CursorModel, range: &Range<Position>) -> bool {
    let (start, end) = (range.start, range.end);
    if start.row == end.row {
        let line = model.line_content(start.row);
        return is_blank(&strings::utf16_slice(&line, start.column, end.column));
    }
    (start.row..=end.row).all(|row| {
        let line = model.line_content(row);
        if row == start.row {
            is_blank(&strings::utf16_slice(&line, start.column, u32::MAX))
        } else if row == end.row {
            is_blank(strings::utf16_prefix(&line, end.column))
        } else {
            is_blank(&line)
        }
    })
}

/// Monaco `isStartOrEndInString`: whether the token at either end of `range`
/// (Monaco `getStandardTokenTypeAtPosition`: the character after the
/// position, or the last one at the end of the line) is a string.
fn is_start_or_end_in_string(
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    range: &Range<Position>,
) -> bool {
    [range.start, range.end]
        .into_iter()
        .any(|p| lang.token_at(model, p.row, p.column) == StandardTokenType::String)
}

/// The indentation edits Monaco's `AutoIndentOnPaste.trigger` makes after
/// text was pasted at `range` (in the text after the paste): the first
/// pasted line gets the indentation the indentation rules expect when only
/// whitespace precedes the paste on it, and the following pasted lines are
/// all shifted by the amount the first non-blank one of them is off from
/// its expected indentation, so their relative indentation is kept.
///
/// Empty when `config.auto_indent` is below [`AutoIndent::Full`], the
/// language has no indentation rules, the pasted text is only whitespace,
/// or (unless `within_string`, Monaco `autoIndentOnPasteWithinString`)
/// either end of `range` is in a string token.
///
/// Each edit replaces the leading whitespace of one line, in the
/// coordinates of the text after the paste.
pub(crate) fn auto_indent_on_paste_edits(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    range: Range<Position>,
    within_string: bool,
) -> Vec<(Range<Position>, String)> {
    if config.auto_indent < AutoIndent::Full {
        return Vec::new();
    }
    if range_contains_only_whitespace(model, &range) {
        return Vec::new();
    }
    if !within_string && is_start_or_end_in_string(lang, model, &range) {
        return Vec::new();
    }
    let tab_size = config.tab_size;
    let insert_spaces = config.insert_spaces;
    let mut edits = Vec::new();

    let mut start_row = range.start.row;
    let mut first_line_text = model.line_content(start_row);
    if is_blank(strings::utf16_prefix(&first_line_text, range.start.column)) {
        let vm = VirtualModel::new(model, lang);
        if let Some(indent_of_first_line) = rules_good_indent_for_line(config, &vm, start_row + 1) {
            let old_indentation = strings::leading_whitespace(&first_line_text).to_string();
            let new_space_count = space_count(&indent_of_first_line, tab_size);
            let old_space_count = space_count(&old_indentation, tab_size);
            if new_space_count != old_space_count {
                let new_indent = generate_indent(new_space_count, tab_size, insert_spaces);
                edits.push((
                    Position::new(start_row, 0)
                        ..Position::new(start_row, strings::utf16_len(&old_indentation)),
                    new_indent.clone(),
                ));
                first_line_text =
                    format!("{new_indent}{}", &first_line_text[old_indentation.len()..]);
            } else {
                let metadata = indent_metadata(lang, model, start_row);
                if metadata == Some(0) || metadata == Some(IndentConsts::UNINDENT_MASK) {
                    // The paste went into a whitespace-only line, the first
                    // line's indentation is already right and it matches no
                    // indentation rule: leave the rest alone.
                    return Vec::new();
                }
            }
        }
    }

    let first_row = start_row;
    // Skip empty lines.
    while start_row < range.end.row && is_blank(&model.line_content(start_row + 1)) {
        start_row += 1;
    }

    if start_row != range.end.row {
        let vm = VirtualModel::new(model, lang).with_line_content(first_row + 1, first_line_text);
        let second_row = start_row + 1;
        if let Some(indent_of_second_line) = rules_good_indent_for_line(config, &vm, second_row + 1)
        {
            let new_space_count = space_count(&indent_of_second_line, tab_size);
            let old_space_count = space_count(
                strings::leading_whitespace(&model.line_content(second_row)),
                tab_size,
            );
            if new_space_count != old_space_count {
                let offset = new_space_count - old_space_count;
                for row in second_row..=range.end.row {
                    let line = model.line_content(row);
                    let original_indent = strings::leading_whitespace(&line);
                    let new_indent = generate_indent(
                        space_count(original_indent, tab_size) + offset,
                        tab_size,
                        insert_spaces,
                    );
                    if new_indent != original_indent {
                        edits.push((
                            Position::new(row, 0)
                                ..Position::new(row, strings::utf16_len(original_indent)),
                            new_indent,
                        ));
                    }
                }
            }
        }
    }
    edits
}

/// Monaco `AutoIndentOnPasteCommand`: applies the edits and keeps the
/// selection on the same text. When there is a single edit next to an empty
/// selection, the selection sticks to the side away from the edit.
struct AutoIndentOnPasteCommand {
    edits: Vec<(Range<Position>, String)>,
    initial_selection: Selection,
    tracked: Option<TrackedId>,
}

impl Command for AutoIndentOnPasteCommand {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        for (range, text) in &self.edits {
            builder.add_edit(range.clone(), text);
        }
        let selection = self.initial_selection;
        let mut track_previous = None;
        if let [(range, _)] = self.edits.as_slice()
            && selection.is_empty()
        {
            if range.start == selection.end() {
                track_previous = Some(true);
            } else if range.end == selection.start() {
                track_previous = Some(false);
            }
        }
        self.tracked = Some(builder.track_selection(selection, track_previous));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        helper.tracked_selection(self.tracked.expect("tracked in edit_operations"))
    }
}

/// Re-indents text that was just pasted at `range` (Monaco
/// `AutoIndentOnPaste`, `editor.autoIndentOnPaste`): when only whitespace
/// precedes the paste on its first line, that line gets the indentation the
/// language's indentation rules expect; the following pasted lines are all
/// shifted by the amount the first non-blank one of them is off from its
/// expected indentation, so their relative indentation is kept. Nothing
/// happens when the pasted text is only whitespace, the language has no
/// indentation rules, or (unless `within_string`) either end of `range` is
/// in a string token.
///
/// Call it after the paste has been applied, with `model` and `cursors` as
/// they are after the paste, and `range` from the primary selection's start
/// before the paste to its start after it (like Monaco's `onDidPaste`
/// range). Only call it when `editor.autoIndentOnPaste` is on;
/// `within_string` is `editor.autoIndentOnPasteWithinString`.
///
/// `None` when nothing changes, `config.auto_indent` is below
/// [`AutoIndent::Full`] or there are several cursors. Otherwise the
/// operation is its own undo step (it has undo stops before and after),
/// with the selection kept on the same text.
pub fn auto_indent_on_paste<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    range: Range<Position>,
    lang: &LanguageContext,
    within_string: bool,
) -> Option<EditOperation> {
    let selections = cursors.selections();
    let [selection] = selections.as_slice() else {
        return None;
    };
    let model: &dyn CursorModel = model;
    let lang = LanguageSupport::new(lang);
    let edits = auto_indent_on_paste_edits(config, &lang, model, range, within_string);
    if edits.is_empty() {
        return None;
    }
    let command = AutoIndentOnPasteCommand {
        edits,
        initial_selection: *selection,
        tracked: None,
    };
    Some(execute_commands(
        model,
        &selections,
        vec![Some(Box::new(command))],
        EditOperationType::Other,
        true,
        true,
    ))
}
