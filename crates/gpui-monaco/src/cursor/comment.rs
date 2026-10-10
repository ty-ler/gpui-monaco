//! Toggling line and block comments.
//!
//! Ports Monaco's `src/vs/editor/contrib/comment/browser/lineCommentCommand.ts`,
//! `blockCommentCommand.ts` and the actions in `comment.ts` (MIT). Columns
//! below are Monaco's 1-based UTF-16 columns unless named otherwise; rows are
//! 0-based.

use std::ops::Range;

use syntax::Comments;

use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType, TrackedId,
    execute_commands,
};
use super::strings::{first_non_whitespace_index, utf16};
use super::{CursorCollection, CursorConfig, CursorModel, Position, Selection};

/// What a line comment command does (Monaco `Type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCommentType {
    /// Removes comments if every relevant line has one, else adds them
    /// (`editor.action.commentLine`, Cmd+/).
    Toggle,
    /// Always adds (`editor.action.addCommentLine`, Cmd+K Cmd+C).
    ForceAdd,
    /// Always removes (`editor.action.removeCommentLine`, Cmd+K Cmd+U).
    ForceRemove,
}

/// Monaco's `comments` editor option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommentOptions {
    /// Insert a space after the line comment token and inside block comment
    /// tokens (Monaco `comments.insertSpace`, default `true`).
    pub insert_space: bool,
    /// Leave empty and whitespace-only lines alone when toggling line comments
    /// (Monaco `comments.ignoreEmptyLines`, default `true`).
    pub ignore_empty_lines: bool,
}

impl Default for CommentOptions {
    fn default() -> Self {
        Self {
            insert_space: true,
            ignore_empty_lines: true,
        }
    }
}

/// The comment tokens in effect at a position: the language there, which
/// can be an injected one (Monaco `getLanguageIdAtPosition`).
pub type CommentsAt<'a> = dyn Fn(Position) -> Option<Comments> + 'a;

fn pos(row: u32, column1: u32) -> Position {
    Position::new(row, column1.saturating_sub(1))
}

fn line_units(model: &dyn CursorModel, row: u32) -> Vec<u16> {
    utf16(&model.line_content(row))
}

/// Monaco `getLineFirstNonWhitespaceColumn`: 1-based, or 0 for a blank line.
fn first_non_whitespace_column(model: &dyn CursorModel, row: u32) -> u32 {
    first_non_whitespace_index(&line_units(model, row)).map_or(0, |i| i as u32 + 1)
}

fn max_column(model: &dyn CursorModel, row: u32) -> u32 {
    model.line_max_column(row) + 1
}

/// JavaScript `String.prototype.indexOf`.
fn index_of(haystack: &[u16], needle: &[u16], from: i64) -> Option<usize> {
    let from = from.clamp(0, haystack.len() as i64) as usize;
    if needle.is_empty() {
        return Some(from);
    }
    (from..=haystack.len().saturating_sub(needle.len()))
        .find(|&i| i + needle.len() <= haystack.len() && haystack[i..i + needle.len()] == *needle)
}

/// JavaScript `String.prototype.lastIndexOf`.
fn last_index_of(haystack: &[u16], needle: &[u16], from: i64) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let from = from.clamp(0, (haystack.len() - needle.len()) as i64) as usize;
    (0..=from)
        .rev()
        .find(|&i| haystack[i..i + needle.len()] == *needle)
}

/// Monaco `BlockCommentCommand._haystackHasNeedleAtOffset`: ASCII
/// case-insensitive.
fn has_needle_at(haystack: &[u16], needle: &[u16], offset: i64) -> bool {
    if offset < 0 {
        return false;
    }
    let offset = offset as usize;
    if offset + needle.len() > haystack.len() {
        return false;
    }
    needle.iter().enumerate().all(|(i, &b)| {
        let a = haystack[offset + i];
        a == b
            || ((b'A' as u16..=b'Z' as u16).contains(&a) && a + 32 == b)
            || ((b'A' as u16..=b'Z' as u16).contains(&b) && b + 32 == a)
    })
}

/// Monaco `ISingleEditOperation`.
struct Op {
    range: Range<Position>,
    text: String,
}

fn delete(row: u32, start1: u32, end_row: u32, end1: u32) -> Op {
    Op {
        range: pos(row, start1)..pos(end_row, end1),
        text: String::new(),
    }
}

/// Monaco `BlockCommentCommand._createRemoveBlockCommentOperations`; `inner`
/// is the 1-based range between the tokens.
fn remove_block_comment_ops(
    inner: (u32, u32, u32, u32),
    start_token_len: u32,
    end_token_len: u32,
) -> Vec<Op> {
    let (start_row, start_col, end_row, end_col) = inner;
    if (start_row, start_col) != (end_row, end_col) {
        vec![
            delete(start_row, start_col - start_token_len, start_row, start_col),
            delete(end_row, end_col, end_row, end_col + end_token_len),
        ]
    } else {
        vec![delete(
            start_row,
            start_col - start_token_len,
            end_row,
            end_col + end_token_len,
        )]
    }
}

/// Monaco `BlockCommentCommand._createAddBlockCommentOperations`.
fn add_block_comment_ops(
    range: (u32, u32, u32, u32),
    start_token: &str,
    end_token: &str,
    insert_space: bool,
) -> Vec<Op> {
    let (start_row, start_col, end_row, end_col) = range;
    let space = if insert_space { " " } else { "" };
    if (start_row, start_col) != (end_row, end_col) {
        vec![
            Op {
                range: pos(start_row, start_col)..pos(start_row, start_col),
                text: format!("{start_token}{space}"),
            },
            Op {
                range: pos(end_row, end_col)..pos(end_row, end_col),
                text: format!("{space}{end_token}"),
            },
        ]
    } else {
        vec![Op {
            range: pos(start_row, start_col)..pos(end_row, end_col),
            text: format!("{start_token}  {end_token}"),
        }]
    }
}

fn len16(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

// ---------------------------------------------------------------------------
// Block comments

/// Monaco `BlockCommentCommand`.
struct BlockCommentCommand {
    selection: Selection,
    insert_space: bool,
    tokens: Option<(String, String)>,
    used_end_token: Option<String>,
}

impl Command for BlockCommentCommand {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let Some((start_token, end_token)) = self.tokens.clone() else {
            return;
        };
        let s = self.selection;
        let (start_row, start_col) = (s.start().row, s.start().column + 1);
        let (end_row, end_col) = (s.end().row, s.end().column + 1);
        let start_text = line_units(model, start_row);
        let end_text = line_units(model, end_row);
        let mut start_token = start_token;
        let mut end_token = end_token;
        let start_len = len16(&start_token) as i64;
        let end_len = len16(&end_token) as i64;

        let mut start_index = last_index_of(
            &start_text,
            &utf16(&start_token),
            start_col as i64 - 1 + start_len,
        );
        let mut end_index = index_of(&end_text, &utf16(&end_token), end_col as i64 - 1 - end_len);

        if let (Some(si), Some(ei)) = (start_index, end_index) {
            let end_needle = utf16(&end_token);
            let contains_end = |slice: &[u16]| index_of(slice, &end_needle, 0).is_some();
            let force_add = if start_row == end_row {
                let from = (si + start_len as usize).min(start_text.len());
                let to = ei.max(from).min(start_text.len());
                contains_end(&start_text[from..to])
            } else {
                let after_start = &start_text[(si + start_len as usize).min(start_text.len())..];
                let before_end = &end_text[..ei.min(end_text.len())];
                contains_end(after_start) || contains_end(before_end)
            };
            if force_add {
                start_index = None;
                end_index = None;
            }
        }

        let ops = match (start_index, end_index) {
            (Some(si), Some(mut ei)) => {
                let space = b' ' as u16;
                if self.insert_space
                    && si + (start_len as usize) < start_text.len()
                    && start_text[si + start_len as usize] == space
                {
                    start_token.push(' ');
                }
                if self.insert_space && ei > 0 && end_text[ei - 1] == space {
                    end_token.insert(0, ' ');
                    ei -= 1;
                }
                remove_block_comment_ops(
                    (
                        start_row,
                        si as u32 + len16(&start_token) + 1,
                        end_row,
                        ei as u32 + 1,
                    ),
                    len16(&start_token),
                    len16(&end_token),
                )
            }
            _ => {
                let ops = add_block_comment_ops(
                    (start_row, start_col, end_row, end_col),
                    &start_token,
                    &end_token,
                    self.insert_space,
                );
                self.used_end_token = (ops.len() == 1).then(|| end_token.clone());
                ops
            }
        };
        for op in ops {
            builder.add_edit(op.range, &op.text);
        }
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let inverse = helper.inverse_edit_ranges();
        if inverse.len() == 2 {
            Selection::new(inverse[0].end, inverse[1].start)
        } else {
            let end = inverse[0].end;
            let delta = self.used_end_token.as_deref().map_or(0, |t| len16(t) + 1);
            Selection::collapsed(Position::new(end.row, end.column.saturating_sub(delta)))
        }
    }
}

/// Toggles a block comment around each selection (Monaco
/// `editor.action.blockComment`, Shift+Alt+A).
pub fn block_comment(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    comments_at: &CommentsAt,
    options: CommentOptions,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands = selections
        .iter()
        .map(|&selection| {
            let tokens = comments_at(selection.start())
                .and_then(|c| c.block_comment)
                .filter(|(open, close)| !open.is_empty() && !close.is_empty());
            Some(Box::new(BlockCommentCommand {
                selection,
                insert_space: options.insert_space,
                tokens,
                used_end_token: None,
            }) as Box<dyn Command>)
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
// Line comments

/// Monaco `ILinePreflightData`.
#[derive(Clone, Debug)]
pub(crate) struct LinePreflight {
    pub(crate) ignore: bool,
    pub(crate) comment_str: String,
    /// 0-based UTF-16 offset where the token is (or goes).
    pub(crate) comment_str_offset: u32,
    pub(crate) comment_str_length: u32,
}

/// Monaco `LineCommentCommand`.
struct LineCommentCommand<'a> {
    selection: Selection,
    indent_size: u32,
    kind: LineCommentType,
    insert_space: bool,
    ignore_empty_lines: bool,
    ignore_first_line: bool,
    comments_at: &'a CommentsAt<'a>,
    tracked: Option<TrackedId>,
    delta_column: u32,
    move_end_position_down: bool,
}

/// Monaco `LineCommentCommand._analyzeLines`: `None` would be "unsupported",
/// which `gather_preflight` decides before.
#[allow(clippy::too_many_arguments)]
pub(crate) fn analyze_lines(
    kind: LineCommentType,
    insert_space: bool,
    model: &dyn CursorModel,
    lines: &mut [LinePreflight],
    start_row: u32,
    ignore_empty_lines: bool,
    ignore_first_line: bool,
    line_comment_no_indent: bool,
) -> bool {
    let mut only_whitespace_lines = true;
    let mut should_remove = match kind {
        LineCommentType::Toggle | LineCommentType::ForceRemove => true,
        LineCommentType::ForceAdd => false,
    };
    for (i, line) in lines.iter_mut().enumerate() {
        let row = start_row + i as u32;
        if row == start_row && ignore_first_line {
            line.ignore = true;
            continue;
        }
        let content = line_units(model, row);
        let Some(start_offset) = first_non_whitespace_index(&content) else {
            line.ignore = ignore_empty_lines;
            line.comment_str_offset = if line_comment_no_indent {
                0
            } else {
                content.len() as u32
            };
            continue;
        };
        only_whitespace_lines = false;
        let offset = if line_comment_no_indent {
            0
        } else {
            start_offset as u32
        };
        line.ignore = false;
        line.comment_str_offset = offset;

        if should_remove && !has_needle_at(&content, &utf16(&line.comment_str), offset as i64) {
            match kind {
                LineCommentType::Toggle => should_remove = false,
                LineCommentType::ForceAdd => {}
                LineCommentType::ForceRemove => line.ignore = true,
            }
        }
        if should_remove && insert_space {
            // Remove a following space if present.
            let end = start_offset + line.comment_str_length as usize;
            if end < content.len() && content[end] == b' ' as u16 {
                line.comment_str_length += 1;
            }
        }
    }
    if kind == LineCommentType::Toggle && only_whitespace_lines {
        should_remove = false;
        for line in lines.iter_mut() {
            line.ignore = false;
        }
    }
    should_remove
}

fn next_visible_column(current: u32, indent_size: u32, is_tab: bool) -> u32 {
    if is_tab {
        current + (indent_size - current % indent_size)
    } else {
        current + 1
    }
}

/// Monaco `LineCommentCommand._normalizeInsertionPoint`: aligns the inserted
/// tokens vertically.
pub(crate) fn normalize_insertion_points(
    model: &dyn CursorModel,
    lines: &mut [LinePreflight],
    start_row: u32,
    indent_size: u32,
) {
    let indent_size = indent_size.max(1);
    let mut min_visible = u32::MAX;
    for (i, line) in lines.iter().enumerate() {
        if line.ignore {
            continue;
        }
        let content = line_units(model, start_row + i as u32);
        let mut visible = 0;
        let mut j = 0;
        while visible < min_visible && j < line.comment_str_offset as usize {
            visible = next_visible_column(visible, indent_size, content[j] == b'\t' as u16);
            j += 1;
        }
        min_visible = min_visible.min(visible);
    }
    min_visible = (min_visible / indent_size) * indent_size;
    for (i, line) in lines.iter_mut().enumerate() {
        if line.ignore {
            continue;
        }
        let content = line_units(model, start_row + i as u32);
        let mut visible = 0;
        let mut j = 0;
        while visible < min_visible && j < line.comment_str_offset as usize {
            visible = next_visible_column(visible, indent_size, content[j] == b'\t' as u16);
            j += 1;
        }
        line.comment_str_offset = if visible > min_visible {
            j as u32 - 1
        } else {
            j as u32
        };
    }
}

impl LineCommentCommand<'_> {
    /// Monaco `_attemptRemoveBlockComment`.
    fn attempt_remove_block_comment(
        model: &dyn CursorModel,
        s: Selection,
        start_token: &str,
        end_token: &str,
    ) -> Option<Vec<Op>> {
        let mut start_row = s.start().row;
        let mut end_row = s.end().row;
        let (start_col, end_col) = (s.start().column + 1, s.end().column + 1);
        let mut start_token = start_token.to_string();
        let mut end_token = end_token.to_string();
        let start_needle = utf16(&start_token);
        let end_needle = utf16(&end_token);
        let allowed_before = len16(&end_token) as i64
            + (first_non_whitespace_column(model, start_row) as i64).max(start_col as i64);

        let mut start_index = last_index_of(
            &line_units(model, start_row),
            &start_needle,
            allowed_before - 1,
        );
        let mut end_index = index_of(
            &line_units(model, end_row),
            &end_needle,
            end_col as i64 - 1 - start_needle.len() as i64,
        );
        if let (Some(si), None) = (start_index, end_index) {
            end_index = index_of(
                &line_units(model, start_row),
                &end_needle,
                (si + start_needle.len()) as i64,
            );
            end_row = start_row;
        }
        if let (None, Some(ei)) = (start_index, end_index) {
            start_index = last_index_of(&line_units(model, end_row), &start_needle, ei as i64);
            start_row = end_row;
        }
        if s.is_empty() && (start_index.is_none() || end_index.is_none()) {
            let line = line_units(model, start_row);
            start_index = index_of(&line, &start_needle, 0);
            if let Some(si) = start_index {
                end_index = index_of(&line, &end_needle, (si + start_needle.len()) as i64);
            }
        }
        // Spaces inside the tokens are part of the comment.
        let space = b' ' as u16;
        if let Some(si) = start_index
            && line_units(model, start_row).get(si + start_needle.len()) == Some(&space)
        {
            start_token.push(' ');
        }
        if let Some(ei) = end_index
            && ei > 0
            && line_units(model, end_row).get(ei - 1) == Some(&space)
        {
            end_token.insert(0, ' ');
            end_index = Some(ei - 1);
        }
        let (si, ei) = (start_index?, end_index?);
        Some(remove_block_comment_ops(
            (
                start_row,
                si as u32 + len16(&start_token) + 1,
                end_row,
                ei as u32 + 1,
            ),
            len16(&start_token),
            len16(&end_token),
        ))
    }

    /// Monaco `_executeBlockComment`: the language has no line comment.
    fn execute_block_comment(
        &mut self,
        model: &dyn CursorModel,
        builder: &mut EditBuilder,
        s: Selection,
    ) {
        let Some((start_token, end_token)) = (self.comments_at)(Position::new(s.start().row, 0))
            .and_then(|c| c.block_comment)
            .filter(|(open, close)| !open.is_empty() && !close.is_empty())
        else {
            return;
        };
        let ops = match Self::attempt_remove_block_comment(model, s, &start_token, &end_token) {
            Some(ops) => ops,
            None => {
                let ops = if s.is_empty() {
                    let row = s.start().row;
                    let content = line_units(model, row);
                    let first = first_non_whitespace_index(&content).unwrap_or(content.len());
                    add_block_comment_ops(
                        (row, first as u32 + 1, row, content.len() as u32 + 1),
                        &start_token,
                        &end_token,
                        self.insert_space,
                    )
                } else {
                    add_block_comment_ops(
                        (
                            s.start().row,
                            first_non_whitespace_column(model, s.start().row),
                            s.end().row,
                            max_column(model, s.end().row),
                        ),
                        &start_token,
                        &end_token,
                        self.insert_space,
                    )
                };
                if ops.len() == 1 {
                    // Leave the cursor after the token and space.
                    self.delta_column = len16(&start_token) + 1;
                }
                ops
            }
        };
        self.tracked = Some(builder.track_selection(s, None));
        for op in ops {
            builder.add_edit(op.range, &op.text);
        }
    }
}

impl Command for LineCommentCommand<'_> {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let mut s = self.selection;
        self.move_end_position_down = false;

        if s.start().row == s.end().row && self.ignore_first_line {
            let row = s.start().row;
            let last = row + 1 == model.line_count();
            let end = if last {
                Position::new(row, model.line_max_column(row))
            } else {
                Position::new(row + 1, 0)
            };
            builder.add_edit(
                Position::new(row, model.line_max_column(row))..end,
                if last { "" } else { "\n" },
            );
            self.tracked = Some(builder.track_selection(s, None));
            return;
        }

        if s.start().row < s.end().row && s.end().column == 0 {
            self.move_end_position_down = true;
            let row = s.end().row - 1;
            s = set_end_position(s, Position::new(row, model.line_max_column(row)));
        }

        // Monaco `_gatherPreflightCommentStrings`: the language at the start
        // of the first line decides.
        let comments = (self.comments_at)(Position::new(s.start().row, 0));
        let Some(comment_str) = comments
            .as_ref()
            .and_then(|c| c.line_comment.clone())
            .filter(|t| !t.is_empty())
        else {
            return self.execute_block_comment(model, builder, s);
        };
        let no_indent = comments.as_ref().is_some_and(|c| c.line_comment_no_indent);
        let mut lines = vec![
            LinePreflight {
                ignore: false,
                comment_str_length: len16(&comment_str),
                comment_str,
                comment_str_offset: 0,
            };
            (s.end().row - s.start().row + 1) as usize
        ];
        let should_remove = analyze_lines(
            self.kind,
            self.insert_space,
            model,
            &mut lines,
            s.start().row,
            self.ignore_empty_lines,
            self.ignore_first_line,
            no_indent,
        );

        // Monaco `_executeLineComments`.
        let start_row = s.start().row;
        let ops: Vec<Op> = if should_remove {
            lines
                .iter()
                .enumerate()
                .filter(|(_, line)| !line.ignore)
                .map(|(i, line)| {
                    let row = start_row + i as u32;
                    delete(
                        row,
                        line.comment_str_offset + 1,
                        row,
                        line.comment_str_offset + line.comment_str_length + 1,
                    )
                })
                .collect()
        } else {
            normalize_insertion_points(model, &mut lines, start_row, self.indent_size);
            let after = if self.insert_space { " " } else { "" };
            lines
                .iter()
                .enumerate()
                .filter(|(_, line)| !line.ignore)
                .map(|(i, line)| {
                    let at = Position::new(start_row + i as u32, line.comment_str_offset);
                    Op {
                        range: at..at,
                        text: format!("{}{after}", line.comment_str),
                    }
                })
                .collect()
        };
        let cursor = s.head;
        for op in ops {
            if op.range.start == op.range.end
                && op.range.start == cursor
                && model.line_max_column(cursor.row) == cursor.column
            {
                self.delta_column = len16(&op.text);
            }
            builder.add_edit(op.range, &op.text);
        }
        self.tracked = Some(builder.track_selection(s, None));
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let Some(tracked) = self.tracked else {
            return self.selection;
        };
        let mut result = helper.tracked_selection(tracked);
        if self.move_end_position_down {
            result = set_end_position(result, Position::new(result.end().row + 1, 0));
        }
        Selection::new(
            Position::new(result.anchor.row, result.anchor.column + self.delta_column),
            Position::new(result.head.row, result.head.column + self.delta_column),
        )
    }
}

/// Monaco `Selection.setEndPosition`, which keeps the direction.
fn set_end_position(selection: Selection, end: Position) -> Selection {
    Selection::from_range(selection.start()..end, selection.is_reversed())
}

/// Adds, removes or toggles line comments on the selected lines (Monaco
/// `CommentLineAction`). Languages without a line comment get a block
/// comment instead.
pub fn line_comment(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    comments_at: &CommentsAt,
    kind: LineCommentType,
    options: CommentOptions,
) -> EditOperation {
    let model: &dyn CursorModel = model;
    let mut selections: Vec<(Selection, usize, bool)> = cursors
        .selections()
        .into_iter()
        .enumerate()
        .map(|(index, selection)| (selection, index, false))
        .collect();
    selections.sort_by_key(|(s, _, _)| (s.start(), s.end()));
    // Selections sharing a line: only the first cursor comments it.
    let mut prev = 0;
    for curr in 1..selections.len() {
        if selections[prev].0.end().row == selections[curr].0.start().row {
            if selections[prev].1 < selections[curr].1 {
                selections[curr].2 = true;
            } else {
                selections[prev].2 = true;
                prev = curr;
            }
        }
    }
    let before: Vec<Selection> = selections.iter().map(|(s, _, _)| *s).collect();
    let commands = selections
        .iter()
        .map(|&(selection, _, ignore_first_line)| {
            Some(Box::new(LineCommentCommand {
                selection,
                indent_size: config.indent_size,
                kind,
                insert_space: options.insert_space,
                ignore_empty_lines: options.ignore_empty_lines,
                ignore_first_line,
                comments_at,
                tracked: None,
                delta_column: 0,
                move_end_position_down: false,
            }) as Box<dyn Command + '_>)
        })
        .collect();
    execute_commands(
        model,
        &before,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}
