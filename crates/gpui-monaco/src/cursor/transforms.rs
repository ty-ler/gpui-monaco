//! One-shot text transforms.
//!
//! Ports the rest of Monaco's
//! `src/vs/editor/contrib/linesOperations/browser/linesOperations.ts`
//! (sort, delete duplicates, reverse, trim trailing whitespace, delete all
//! right, join lines, transpose, case transforms, duplicate selection) with
//! `sortLinesCommand.ts` and `trimTrailingWhitespaceCommand.ts`, plus
//! `contrib/caretOperations` (transpose letters, move caret) (MIT). Columns
//! are UTF-16 and 0-based; comments mention Monaco's 1-based columns where
//! the port keeps its arithmetic.

use std::ops::Range;

use regex::Regex;

use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType, ReplaceCommand,
    ReplaceCommandThatPreservesSelection, TrackedId, execute_commands, execute_edits,
};
use super::lines::copy_lines_command;
use super::movement::{left_position, right_position};
use super::strings::{self, utf16};
use super::word::{WordCharClass, WordClassifier};
use super::{CursorCollection, CursorConfig, CursorModel, Position, Selection};

type Commands<'a> = Vec<Option<Box<dyn Command + 'a>>>;

fn last_position(model: &dyn CursorModel) -> Position {
    let row = model.line_count() - 1;
    Position::new(row, model.line_max_column(row))
}

/// Monaco's "apply to the whole document when there's one single-line
/// selection" rule for sort, delete duplicates and reverse.
fn whole_document_if_single_line(
    model: &dyn CursorModel,
    selections: Vec<Selection>,
) -> (Vec<Selection>, bool) {
    if selections.len() == 1 && selections[0].start().row == selections[0].end().row {
        (
            vec![Selection::new(Position::zero(), last_position(model))],
            true,
        )
    } else {
        (selections, false)
    }
}

fn value_in_range(model: &dyn CursorModel, range: Range<Position>) -> String {
    if range.start.row == range.end.row {
        return strings::utf16_slice(
            &model.line_content(range.start.row),
            range.start.column,
            range.end.column,
        );
    }
    let mut text = strings::utf16_slice(
        &model.line_content(range.start.row),
        range.start.column,
        u32::MAX,
    );
    for row in range.start.row + 1..range.end.row {
        text.push('\n');
        text.push_str(&model.line_content(row));
    }
    text.push('\n');
    text.push_str(&strings::utf16_slice(
        &model.line_content(range.end.row),
        0,
        range.end.column,
    ));
    text
}

fn len16(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// Monaco `Range.intersectRanges(a, b) !== null` (touching ranges count).
fn intersects(a: &Range<Position>, b: &Range<Position>) -> bool {
    a.start.max(b.start) <= a.end.min(b.end)
}

// ---------------------------------------------------------------------------
// Sort

/// Unicode-aware, case-insensitive-first ordering, like the default
/// `Intl.Collator` Monaco sorts with: base letters first, then lowercase
/// before uppercase.
fn collate(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| {
        let flip = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    if c.is_lowercase() {
                        c.to_uppercase().next().unwrap_or(c)
                    } else {
                        c.to_lowercase().next().unwrap_or(c)
                    }
                })
                .collect()
        };
        flip(a).cmp(&flip(b))
    })
}

/// Monaco `getSortData`: the rows to sort and their sorted text, or `None`
/// when there's nothing to sort.
fn sort_data(
    model: &dyn CursorModel,
    selection: Selection,
    descending: bool,
) -> Option<(u32, u32, Vec<String>, Vec<String>)> {
    let start = selection.start().row;
    let mut end = selection.end().row;
    if selection.end().column == 0 {
        end = end.saturating_sub(1);
    }
    if start >= end {
        return None;
    }
    let before: Vec<String> = (start..=end).map(|row| model.line_content(row)).collect();
    let mut after = before.clone();
    after.sort_by(|a, b| collate(a, b));
    if descending {
        after.reverse();
    }
    Some((start, end, before, after))
}

/// Monaco `SortLinesCommand`.
struct SortLinesCommand {
    selection: Selection,
    descending: bool,
    tracked: Option<TrackedId>,
}

impl Command for SortLinesCommand {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        if let Some((start, end, _, after)) = sort_data(model, self.selection, self.descending) {
            builder.add_edit(
                Position::new(start, 0)..Position::new(end, model.line_max_column(end)),
                &after.join("\n"),
            );
        }
        self.tracked = Some(builder.track_selection(self.selection, None));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        helper.tracked_selection(self.tracked.expect("tracked"))
    }
}

/// Sorts the selected lines (Monaco `editor.action.sortLinesAscending` /
/// `sortLinesDescending`); with one single-line selection, the whole
/// document. `None` when a selection has nothing to change.
pub fn sort_lines(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    descending: bool,
) -> Option<EditOperation> {
    let model: &dyn CursorModel = model;
    let (selections, _) = whole_document_if_single_line(model, cursors.selections());
    for &selection in &selections {
        let (_, _, before, after) = sort_data(model, selection, descending)?;
        if before == after {
            return None;
        }
    }
    let commands: Commands = selections
        .iter()
        .map(|&selection| {
            Some(Box::new(SortLinesCommand {
                selection,
                descending,
                tracked: None,
            }) as Box<dyn Command>)
        })
        .collect();
    Some(execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    ))
}

// ---------------------------------------------------------------------------
// Delete duplicates, reverse

/// Removes repeated lines within each selection, keeping the first of each
/// (Monaco `editor.action.removeDuplicateLines`); with one single-line
/// selection, in the whole document.
pub fn delete_duplicate_lines(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> Option<EditOperation> {
    let model: &dyn CursorModel = model;
    if model.line_count() == 1 && model.line_max_column(0) == 0 {
        return None;
    }
    let before = cursors.selections();
    let (selections, whole_document) = whole_document_if_single_line(model, before.clone());
    let mut edits = Vec::new();
    let mut end_state = Vec::new();
    let mut lines_deleted = 0;
    for selection in selections {
        let (start, end) = (selection.start().row, selection.end().row);
        let mut seen = std::collections::HashSet::new();
        let mut lines = Vec::new();
        for row in start..=end {
            let line = model.line_content(row);
            if seen.insert(line.clone()) {
                lines.push(line);
            }
        }
        let adjusted = start - lines_deleted;
        let last_len = lines.last().map_or(0, |l| len16(l));
        end_state.push(Selection::new(
            Position::new(adjusted, 0),
            Position::new(adjusted + lines.len() as u32 - 1, last_len),
        ));
        lines_deleted += (end - start + 1) - lines.len() as u32;
        edits.push((
            Position::new(start, 0)..Position::new(end, model.line_max_column(end)),
            lines.join("\n"),
        ));
    }
    Some(execute_edits(
        model,
        &before,
        edits,
        (!whole_document).then_some(end_state),
    ))
}

/// Reverses the order of the selected lines (Monaco
/// `editor.action.reverseLines`); with one single-line selection, of the
/// whole document. Cursors move with their lines.
pub fn reverse_lines(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let original = cursors.selections();
    let (selections, _) = whole_document_if_single_line(model, original.clone());
    let mut edits = Vec::new();
    let mut resulting = Vec::new();
    for (selection, original) in selections.iter().zip(&original) {
        let start = selection.start().row;
        let mut end = selection.end().row;
        if start < end && selection.end().column == 0 {
            end -= 1;
        }
        let mut range_end = Position::new(end, model.line_max_column(end));
        // Exclude the last line if it's empty and ends the document.
        if end + 1 == model.line_count() && model.line_content(end).is_empty() && end > 0 {
            range_end = Position::new(end - 1, model.line_max_column(end - 1));
        }
        let lines: Vec<String> = (start..=range_end.row)
            .rev()
            .map(|row| model.line_content(row))
            .collect();
        edits.push((Position::new(start, 0)..range_end, lines.join("\n")));
        let flip = |row: u32| {
            if row <= range_end.row {
                range_end.row - row + start
            } else {
                row
            }
        };
        resulting.push(if original.is_empty() {
            let head = Position::new(flip(original.head.row), original.head.column);
            Selection::collapsed(head)
        } else {
            Selection::new(
                Position::new(flip(original.anchor.row), original.anchor.column),
                Position::new(flip(original.head.row), original.head.column),
            )
        });
    }
    execute_edits(model, &original, edits, Some(resulting))
}

// ---------------------------------------------------------------------------
// Trim trailing whitespace

/// Monaco `TrimTrailingWhitespaceCommand` for the primary selection.
struct TrimTrailingWhitespaceCommand {
    selection: Selection,
    tracked: Option<TrackedId>,
}

impl Command for TrimTrailingWhitespaceCommand {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        for row in 0..model.line_count() {
            let line = utf16(&model.line_content(row));
            if line.is_empty() {
                continue;
            }
            let from = match strings::last_non_whitespace_index(&line, None) {
                None => 0,
                Some(last) if last + 1 != line.len() => last as u32 + 1,
                Some(_) => continue,
            };
            builder.add_edit(
                Position::new(row, from)..Position::new(row, line.len() as u32),
                "",
            );
        }
        self.tracked = Some(builder.track_selection(self.selection, None));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        helper.tracked_selection(self.tracked.expect("tracked"))
    }
}

/// Removes trailing whitespace from every line (Monaco
/// `editor.action.trimTrailingWhitespace`, Cmd/Ctrl+K Cmd/Ctrl+X).
pub fn trim_trailing_whitespace(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let mut commands: Commands = vec![Some(Box::new(TrimTrailingWhitespaceCommand {
        selection: selections[0],
        tracked: None,
    }))];
    commands.resize_with(selections.len(), || None);
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}

// ---------------------------------------------------------------------------
// Delete all right

/// Deletes from each cursor to the end of its line, or joins the next line
/// when already there (Monaco `deleteAllRight`, Ctrl+K / Cmd+Delete on
/// macOS).
pub fn delete_all_right(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let primary = selections[0].range();
    let mut ranges: Vec<Range<Position>> = selections
        .iter()
        .map(|s| {
            if !s.is_empty() {
                return s.range();
            }
            let p = s.start();
            let max = model.line_max_column(p.row);
            if p.column == max {
                p..Position::new(p.row + 1, 0)
            } else {
                p..Position::new(p.row, max)
            }
        })
        .collect();
    ranges.sort_by_key(|r| (r.start, r.end));
    // Monaco `AbstractDeleteAllToBoundaryAction`: merge overlapping ranges.
    let mut effective: Vec<Range<Position>> = Vec::new();
    for i in 0..ranges.len().saturating_sub(1) {
        let (range, next) = (ranges[i].clone(), ranges[i + 1].clone());
        if intersects(&range, &next) {
            ranges[i + 1] = range.start.min(next.start)..range.end.max(next.end);
        } else {
            effective.push(range);
        }
    }
    if let Some(last) = ranges.last() {
        effective.push(last.clone());
    }
    // Monaco's end positions are validated against the model.
    let last = last_position(model);
    let effective: Vec<Range<Position>> = effective
        .into_iter()
        .map(|r| r.start.min(last)..r.end.min(last))
        .collect();

    let mut end_primary = None;
    let mut end_state = Vec::new();
    for range in &effective {
        let cursor = Selection::collapsed(range.start);
        if intersects(range, &primary) {
            end_primary = Some(cursor);
        } else {
            end_state.push(cursor);
        }
    }
    if let Some(primary) = end_primary {
        end_state.insert(0, primary);
    }
    let edits = effective.into_iter().map(|r| (r, String::new())).collect();
    execute_edits(model, &selections, edits, Some(end_state))
}

// ---------------------------------------------------------------------------
// Join lines

/// Joins each selection's lines (or the cursor's line and the next) with
/// single spaces, dropping the joined lines' indentation (Monaco
/// `editor.action.joinLines`, Ctrl+J on macOS).
pub fn join_lines(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let original = cursors.selections();
    let mut primary = original[0];
    let mut selections = original.clone();
    selections.sort_by_key(|s| (s.start(), s.end()));

    // Merge selections on adjacent lines (Monaco's `reduce`).
    let mut reduced: Vec<Selection> = Vec::new();
    let mut previous = selections[0];
    for &current in &selections[1..] {
        let span = |prev: Selection, cur: Selection| Selection::new(prev.start(), cur.end());
        if previous.is_empty() {
            if previous.end().row == current.start().row {
                if primary == previous {
                    primary = current;
                }
                previous = current;
            } else if current.start().row > previous.end().row + 1 {
                reduced.push(previous);
                previous = current;
            } else {
                previous = span(previous, current);
            }
        } else if current.start().row > previous.end().row {
            reduced.push(previous);
            previous = current;
        } else {
            previous = span(previous, current);
        }
    }
    reduced.push(previous);

    let mut edits = Vec::new();
    let mut end_state = Vec::new();
    let mut end_primary = primary;
    let mut line_offset = 0;
    for selection in reduced {
        let start_row = selection.start().row;
        let mut column_delta = 0u32;
        let end_len = model.line_max_column(selection.end().row) as i64;
        let selection_end_offset = end_len - (selection.end().column as i64 + 1);
        let end_row = if selection.is_empty() || selection.start().row == selection.end().row {
            if selection.start().row + 1 < model.line_count() {
                start_row + 1
            } else {
                selection.start().row
            }
        } else {
            selection.end().row
        };
        let end = Position::new(end_row, model.line_max_column(end_row));

        let mut joined = model.line_content(start_row);
        for row in start_row + 1..=end_row {
            let text = model.line_content(row);
            let units = utf16(&text);
            match strings::first_non_whitespace_index(&units) {
                Some(first) => {
                    let mut insert_space = !joined.is_empty();
                    if insert_space && (joined.ends_with(' ') || joined.ends_with('\t')) {
                        insert_space = false;
                        let trimmed_len = joined
                            .trim_end_matches(|c: char| {
                                c.is_whitespace() || c == '\u{feff}' || c == '\u{a0}'
                            })
                            .len();
                        joined.truncate(trimmed_len);
                        joined.push(' ');
                    }
                    let rest = String::from_utf16_lossy(&units[first..]);
                    if insert_space {
                        joined.push(' ');
                    }
                    column_delta = len16(&rest) + u32::from(insert_space);
                    joined.push_str(&rest);
                }
                None => column_delta = 0,
            }
        }

        let delete = Position::new(start_row, 0)..end;
        if delete.start != delete.end {
            let joined_len = len16(&joined);
            let result = if selection.is_empty() {
                let column = joined_len - column_delta;
                Selection::collapsed(Position::new(start_row - line_offset, column))
            } else if selection.start().row == selection.end().row {
                Selection::new(
                    Position::new(
                        selection.start().row - line_offset,
                        selection.start().column,
                    ),
                    Position::new(selection.end().row - line_offset, selection.end().column),
                )
            } else {
                let column = (joined_len as i64 - selection_end_offset - 1).max(0) as u32;
                Selection::new(
                    Position::new(
                        selection.start().row - line_offset,
                        selection.start().column,
                    ),
                    Position::new(selection.start().row - line_offset, column),
                )
            };
            edits.push((delete.clone(), joined));
            if intersects(&delete, &primary.range()) {
                end_primary = result;
            } else {
                end_state.push(result);
            }
        }
        line_offset += delete.end.row - delete.start.row;
    }
    end_state.insert(0, end_primary);
    execute_edits(model, &original, edits, Some(end_state))
}

// ---------------------------------------------------------------------------
// Transpose

/// Swaps the characters around each cursor, moving it forward (Monaco
/// `editor.action.transpose`). At a line end, swaps the last character and
/// the line break.
pub fn transpose(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|s| {
            if !s.is_empty() {
                return None;
            }
            let p = s.start();
            let max = model.line_max_column(p.row);
            if p.column >= max {
                if p.row + 1 == model.line_count() {
                    return None;
                }
                let range =
                    Position::new(p.row, p.column.saturating_sub(1))..Position::new(p.row + 1, 0);
                let chars: String = value_in_range(model, range.clone()).chars().rev().collect();
                ReplaceCommand::boxed(range, chars)
            } else {
                let range = Position::new(p.row, p.column.saturating_sub(1))
                    ..Position::new(p.row, p.column + 1);
                let chars: String = value_in_range(model, range.clone()).chars().rev().collect();
                Some(Box::new(ReplaceCommandThatPreservesSelection {
                    range,
                    text: chars,
                    selection: Selection::collapsed(Position::new(p.row, p.column + 1)),
                    force_move_markers: false,
                    tracked: None,
                }) as Box<dyn Command>)
            }
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}

/// Swaps the characters before and after each cursor, or the two before it
/// at a line end (Monaco `editor.action.transposeLetters`, Ctrl+T on
/// macOS). `None` when no cursor can transpose.
pub fn transpose_letters(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> Option<EditOperation> {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|s| {
            if !s.is_empty() {
                return None;
            }
            let p = s.start();
            let last = model.line_max_column(p.row);
            if p.row == 0 && (p.column == 0 || (p.column == 1 && last == 1)) {
                return None;
            }
            let end = if p.column == last {
                p
            } else {
                right_position(model, p.row, p.column)
            };
            let middle = left_position(model, end);
            let begin = left_position(model, middle);
            let left = value_in_range(model, begin..middle);
            let right = value_in_range(model, middle..end);
            ReplaceCommand::boxed(begin..end, right + &left)
        })
        .collect();
    if commands.iter().all(Option::is_none) {
        return None;
    }
    Some(execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    ))
}

// ---------------------------------------------------------------------------
// Move caret

/// Monaco `MoveCaretCommand`.
struct MoveCaretCommand {
    selection: Selection,
    left: bool,
}

impl Command for MoveCaretCommand {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let s = self.selection;
        if s.start().row != s.end().row || s.is_empty() {
            return;
        }
        let row = s.start().row;
        let (start, end) = (s.start().column, s.end().column);
        if self.left && start == 0 {
            return;
        }
        if !self.left && end == model.line_max_column(row) {
            return;
        }
        if self.left {
            let before = Position::new(row, start - 1)..Position::new(row, start);
            let ch = value_in_range(model, before.clone());
            builder.add_edit(before, "");
            builder.add_edit(Position::new(row, end)..Position::new(row, end), &ch);
        } else {
            let after = Position::new(row, end)..Position::new(row, end + 1);
            let ch = value_in_range(model, after.clone());
            builder.add_edit(after, "");
            builder.add_edit(Position::new(row, start)..Position::new(row, start), &ch);
        }
    }

    fn compute_cursor_state(&self, _: &CursorStateHelper) -> Selection {
        let s = self.selection;
        let shift = |p: Position| {
            if self.left {
                Position::new(p.row, p.column - 1)
            } else {
                Position::new(p.row, p.column + 1)
            }
        };
        Selection::new(shift(s.start()), shift(s.end()))
    }
}

/// Moves the selected text one character left or right (Monaco
/// `editor.action.moveCarretLeftAction` / `moveCarretRightAction`).
pub fn move_caret(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    left: bool,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|&selection| Some(Box::new(MoveCaretCommand { selection, left }) as Box<dyn Command>))
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}

// ---------------------------------------------------------------------------
// Duplicate selection

/// Monaco `ReplaceCommandThatSelectsText`; `reversed` keeps a selection's
/// direction where the port also uses it for the case transforms.
struct ReplaceCommandThatSelectsText {
    range: Range<Position>,
    text: String,
    reversed: bool,
}

impl Command for ReplaceCommandThatSelectsText {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit(self.range.clone(), &self.text);
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let range = helper.inverse_edit_ranges()[0].clone();
        Selection::from_range(range, self.reversed)
    }
}

/// Inserts a copy of each selection after it and selects the copy; empty
/// selections duplicate their line (Monaco `editor.action.duplicateSelection`).
pub fn duplicate_selection(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|&s| {
            if s.is_empty() {
                Some(copy_lines_command(s, true))
            } else {
                let end = s.end();
                Some(Box::new(ReplaceCommandThatSelectsText {
                    range: end..end,
                    text: value_in_range(model, s.range()),
                    reversed: false,
                }) as Box<dyn Command>)
            }
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}

// ---------------------------------------------------------------------------
// Case transforms

/// A case transform (Monaco's `editor.action.transformTo*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseTransform {
    /// `transformToUppercase`.
    Upper,
    /// `transformToLowercase`.
    Lower,
    /// `transformToTitlecase`: the first letter of every word uppercase.
    Title,
    /// `transformToSnakecase`: `camelCase` → `camel_case`.
    Snake,
    /// `transformToCamelcase`: `snake_case` → `snakeCase`.
    Camel,
    /// `transformToPascalcase`: `snake_case` → `SnakeCase`.
    Pascal,
    /// `transformToKebabcase`: `camelCase` → `camel-case`.
    Kebab,
}

struct CaseRegexes {
    title_boundary: Regex,
    snake_case_boundary: Regex,
    snake_single_letters: Regex,
    camel_single_line: Regex,
    camel_multi_line: Regex,
    camel_word_start: Regex,
    pascal_boundary: Regex,
    pascal_all_caps: Regex,
    kebab_single_letters: Regex,
    kebab_underscore: Regex,
}

fn regexes() -> &'static CaseRegexes {
    static REGEXES: std::sync::OnceLock<CaseRegexes> = std::sync::OnceLock::new();
    REGEXES.get_or_init(|| {
        let re = |pattern: &str| Regex::new(pattern).expect("valid case regex");
        CaseRegexes {
            title_boundary: re(r"(?m)(^|[^\p{L}\p{N}']|((^|\P{L})'))\p{L}"),
            snake_case_boundary: re(r"(\p{Ll})(\p{Lu})"),
            snake_single_letters: re(r"(\p{Lu}|\p{N})(\p{Lu})(\p{Ll})"),
            camel_single_line: re(r"[_\s-]+"),
            camel_multi_line: re(r"[_-]+"),
            camel_word_start: re(r"(?m)^(\p{Lu}[^\p{Lu}])"),
            pascal_boundary: re(r"[_ \t-]"),
            pascal_all_caps: re(r"(?m)^\p{Lu}+$"),
            kebab_single_letters: re(r"(\p{Lu}|\p{N})(\p{Lu}\p{Ll})"),
            kebab_underscore: re(r"(\S)(_)(\S)"),
        }
    })
}

fn capitalize_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Applies a case transform to `text` (Monaco's `_modifyText`).
pub fn transform_text_case(text: &str, transform: CaseTransform) -> String {
    let r = regexes();
    match transform {
        CaseTransform::Upper => text.to_uppercase(),
        CaseTransform::Lower => text.to_lowercase(),
        CaseTransform::Title => r
            .title_boundary
            .replace_all(&text.to_lowercase(), |c: &regex::Captures| {
                c[0].to_uppercase()
            })
            .into_owned(),
        CaseTransform::Snake => {
            let text = r.snake_case_boundary.replace_all(text, "${1}_${2}");
            r.snake_single_letters
                .replace_all(&text, "${1}_${2}${3}")
                .to_lowercase()
        }
        CaseTransform::Camel => {
            let boundary = if text.contains(['\n', '\r']) {
                &r.camel_multi_line
            } else {
                &r.camel_single_line
            };
            let mut words = boundary.split(text);
            let first = words.next().unwrap_or("");
            let first = r
                .camel_word_start
                .replace_all(first, |c: &regex::Captures| c[0].to_lowercase());
            let rest: String = words.map(capitalize_first).collect();
            format!("{first}{rest}")
        }
        CaseTransform::Pascal => {
            // Split after every `.` (keeping it), then at the boundaries.
            let mut pieces = Vec::new();
            let mut start = 0;
            for (i, _) in text.match_indices('.') {
                pieces.push(&text[start..i + 1]);
                start = i + 1;
            }
            pieces.push(&text[start..]);
            pieces
                .into_iter()
                .flat_map(|piece| r.pascal_boundary.split(piece))
                .map(|word| {
                    let normalized = capitalize_first(word);
                    let all_caps =
                        normalized.chars().count() > 1 && r.pascal_all_caps.is_match(&normalized);
                    if all_caps {
                        let mut chars = normalized.chars();
                        let first = chars.next().unwrap_or_default();
                        format!("{first}{}", chars.as_str().to_lowercase())
                    } else {
                        normalized
                    }
                })
                .collect()
        }
        CaseTransform::Kebab => {
            let text = r.kebab_underscore.replace_all(text, "${1}-${3}");
            let text = r.snake_case_boundary.replace_all(&text, "${1}-${2}");
            r.kebab_single_letters
                .replace_all(&text, "${1}-${2}")
                .to_lowercase()
        }
    }
}

/// The word (not separators or whitespace) at `p` (Monaco
/// `getConfiguredWordAtPosition`).
fn word_range_at(
    config: &CursorConfig,
    model: &dyn CursorModel,
    p: Position,
) -> Option<Range<Position>> {
    let classifier = WordClassifier::new(&config.word_separators);
    let line = utf16(&model.line_content(p.row));
    let is_word = |i: usize| classifier.get(line[i]) == WordCharClass::Regular;
    let column = (p.column as usize).min(line.len());
    let anchor = if column < line.len() && is_word(column) {
        column
    } else if column > 0 && is_word(column - 1) {
        column - 1
    } else {
        return None;
    };
    let start = (0..anchor)
        .rev()
        .take_while(|&i| is_word(i))
        .last()
        .unwrap_or(anchor);
    let end = (anchor..line.len())
        .take_while(|&i| is_word(i))
        .last()
        .unwrap_or(anchor)
        + 1;
    Some(Position::new(p.row, start as u32)..Position::new(p.row, end as u32))
}

/// Applies a case transform to each selection, or to the word at each
/// cursor (Monaco `AbstractCaseAction`).
pub fn transform_case(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    transform: CaseTransform,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    // Monaco runs `editor.executeEdits` without an end state, so the cursors
    // are recovered from their markers, which are
    // `TrackedRangeStickiness.AlwaysGrowsWhenTypingAtEdges`: a selection that
    // is replaced selects the replacement (`parseHTMLString` → the whole
    // `parse_html_string`), and a cursor inside a word stays put.
    let commands: Commands = selections
        .iter()
        .map(|&s| {
            let range = if s.is_empty() {
                word_range_at(config, model, s.start())?
            } else {
                s.range()
            };
            let text = transform_text_case(&value_in_range(model, range.clone()), transform);
            Some(if s.is_empty() {
                Box::new(ReplaceCommandThatPreservesSelection {
                    range,
                    text,
                    selection: s,
                    force_move_markers: false,
                    tracked: None,
                }) as Box<dyn Command>
            } else {
                Box::new(ReplaceCommandThatSelectsText {
                    range,
                    text,
                    reversed: s.is_reversed(),
                })
            })
        })
        .collect();
    execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}
