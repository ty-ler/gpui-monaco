//! Language-aware typing: auto-closing pairs, overtyping and deleting them,
//! surrounding selections, electric characters, onEnter rules and
//! indentation rules.
//!
//! Ports `TypeOperations.typeWithInterceptors` from Monaco's
//! `src/vs/editor/common/cursor/cursorTypeOperations.ts`;
//! `AutoIndentOperation`, `AutoClosingOvertypeOperation`,
//! `AutoClosingOpenCharTypeOperation`, `SurroundSelectionOperation`,
//! `InterceptorElectricCharOperation`, `EnterOperation._enter`,
//! `TabOperation._goodIndentForLine`, `TypeWithAutoClosingCommand`,
//! `TypeWithIndentationAndAutoClosingCommand`, `isAutoClosingOvertype` and
//! `shouldSurroundChar` from
//! `src/vs/editor/common/cursor/cursorTypeEditOperations.ts`;
//! `DeleteOperations.isAutoClosingPairDelete` from
//! `src/vs/editor/common/cursor/cursorDeleteOperations.ts`; and
//! `SurroundSelectionCommand` from
//! `src/vs/editor/common/commands/surroundSelectionCommand.ts` (MIT).
//!
//! The interceptors that run when an IME composition ends are in
//! `composition.rs`, and the auto-closed pairs of snippet edits in
//! [`snippet_auto_closed_pairs`].

use std::ops::Range;

use syntax::IndentAction;
use unicode_segmentation::UnicodeSegmentation;

use super::brackets::{Brackets, find_matching_bracket_up, has_unmatched_closing_bracket_after};
use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType, ReplaceCommand,
    ReplaceCommandWithOffsetCursorState, ReplaceCommandWithoutChangingPosition, auto_whitespace,
    execute_commands, should_push_stack_element_between, typing_operation,
};
use super::language::{
    AutoClosingPair, LanguageContext, LanguageSupport, enter_action, good_indent_for_line,
    indent_action_for_type, indent_for_enter, indentation_at_position, is_electric_character,
    is_quote, on_electric_character, scoped_support, unshift_indent,
};
use super::typing::{
    delete_left, delete_word_part_with, delete_word_with, enter_text, line_break, shift, tab,
    tab_with_good_indent, type_simple, type_text,
};
use super::word::{WordCharClass, WordClassifier, WordNavigationType};
use super::{
    AutoClosedPair, AutoClosingEditStrategy, AutoClosingStrategy, AutoIndent, AutoSurroundStrategy,
    CursorCollection, CursorConfig, CursorModel, Position, Selection, line_units, strings,
};

type Commands = Vec<Option<Box<dyn Command>>>;

/// Types `text` at every cursor like [`type_text`], but first gives the
/// language a chance to intercept it, in Monaco's order: Enter (see
/// [`line_break_with_language`]), re-indenting a line that now matches the
/// indentation rules' `decreaseIndentPattern`, typing over an auto-inserted
/// closing character, auto-closing a pair, surrounding the selections,
/// and lining up a closing bracket with its opening bracket (electric
/// characters).
///
/// `text` is one typed character (grapheme), as Monaco handles keyboard
/// input; longer text is typed without interceptors, so type it a grapheme
/// at a time. With `lang == None` this is exactly [`type_text`].
///
/// While an IME composition is in progress
/// ([`CursorCollection::is_composing`]), only typing over an auto-inserted
/// closing character applies, and the text replaces the selections as it is
/// (no line break handling or overtype mode), as in Monaco; the rest runs
/// when the composition ends (see
/// [`composition_end_with_language`](super::composition_end_with_language)).
///
/// Reads [`CursorCollection::auto_closed_ranges`] to decide what to type
/// over; the returned operation's `auto_closed` lists the closing characters
/// it inserted, and its `follow_up` (if any) must be applied right after it
/// (see [`EditOperation::follow_up`]).
pub fn type_with_language<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    text: &str,
    prev: Option<EditOperationType>,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let composing = cursors.is_composing();
    let Some(ctx) = lang else {
        if composing {
            return type_simple(model, cursors, text, prev, false);
        }
        return type_text(config, model, cursors, text, prev);
    };
    if !composing && matches!(text, "\n" | "\r\n" | "\r") {
        return line_break_with_language(config, model, cursors, lang);
    }
    if text.graphemes(true).count() != 1 {
        return type_simple(model, cursors, text, prev, config.overtype && !composing);
    }
    let lang = LanguageSupport::new(ctx);
    let dm: &dyn CursorModel = model;
    let selections = cursors.selections();
    let prev_kind = prev.unwrap_or(EditOperationType::Other);
    let auto_closed = cursors.auto_closed_ranges().closing_ranges();

    if !composing && let Some(op) = auto_indent_type(config, &lang, dm, &selections, text) {
        return op;
    }
    if is_auto_closing_overtype(config, &lang, dm, &selections, &auto_closed, text) {
        return auto_closing_overtype(dm, &selections, text, prev_kind);
    }
    if composing {
        return type_simple(dm, cursors, text, prev, false);
    }
    if let Some(pair) = auto_closing_pair(config, &lang, dm, &selections, text, false) {
        return auto_closing_open_char_type(dm, &selections, text, false, &pair);
    }
    if is_surround_selection_type(config, &lang, dm, &selections, text) {
        return surround_selection_type(&lang, dm, &selections, text);
    }
    if let Some(op) = electric_char(config, &lang, dm, &selections, text, prev_kind) {
        return op;
    }
    type_text(config, model, cursors, text, prev)
}

/// Inserts a line break at every cursor and indents the new line according
/// to the language (Monaco `EnterOperation`): onEnter rules, the indentation
/// rules, and indent-outdent between brackets (`{|}`), as far as
/// [`CursorConfig::auto_indent`] allows. With `lang == None` this is exactly
/// [`line_break`].
pub fn line_break_with_language<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let Some(ctx) = lang else {
        return line_break(config, model, cursors);
    };
    let lang = LanguageSupport::new(ctx);
    let dm: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = selections
        .iter()
        .map(|s| enter_command_with_language(config, &lang, dm, false, s.range()))
        .collect();
    execute_commands(
        dm,
        &selections,
        commands,
        EditOperationType::TypingOther,
        true,
        false,
    )
}

/// Inserts a line break after every cursor without moving it (Monaco
/// `EnterOperation.lineBreakInsert`, Ctrl+O on macOS). The new line is
/// indented like Enter would.
pub fn line_break_insert<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let dm: &dyn CursorModel = model;
    let selections = cursors.selections();
    let commands: Commands = match lang {
        Some(ctx) => {
            let lang = LanguageSupport::new(ctx);
            selections
                .iter()
                .map(|s| enter_command_with_language(config, &lang, dm, true, s.range()))
                .collect()
        }
        None => selections
            .iter()
            .map(|s| {
                auto_whitespace(Some(Box::new(ReplaceCommandWithoutChangingPosition {
                    range: s.range(),
                    text: enter_text(config, dm, &s.range()),
                })))
            })
            .collect(),
    };
    execute_commands(
        dm,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        false,
    )
}

/// Backspace (Monaco `DeleteOperations.deleteLeft`): between an
/// auto-closing pair's opening and closing character (`(|)`), deletes both,
/// as [`CursorConfig::auto_closing_delete`] allows; otherwise like
/// [`delete_left`]. With `lang == None` this is exactly [`delete_left`].
pub fn delete_left_with_language<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    prev: Option<EditOperationType>,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let Some(ctx) = lang else {
        return delete_left(config, model, cursors, prev);
    };
    let lang = LanguageSupport::new(ctx);
    let dm: &dyn CursorModel = model;
    let selections = cursors.selections();
    let auto_closed = cursors.auto_closed_ranges().closing_ranges();
    if is_auto_closing_pair_delete(config, &lang, dm, &selections, &auto_closed) {
        let commands: Commands = selections
            .iter()
            .map(|s| {
                let p = s.head;
                ReplaceCommand::boxed(
                    Position::new(p.row, p.column - 1)..Position::new(p.row, p.column + 1),
                    "",
                )
            })
            .collect();
        return execute_commands(
            dm,
            &selections,
            commands,
            EditOperationType::DeletingLeft,
            true,
            false,
        );
    }
    delete_left(config, model, cursors, prev)
}

/// Deletes words to the left or right of every cursor like
/// [`delete_word`](super::delete_word); to the left, a cursor between an
/// auto-closing pair's opening and closing character (`(|)`) deletes both,
/// as [`CursorConfig::auto_closing_delete`] allows (Monaco
/// `WordOperations.deleteWordLeft` with `autoClosingDelete`). With
/// `lang == None` this is exactly [`delete_word`](super::delete_word).
#[allow(clippy::too_many_arguments)]
pub fn delete_word_with_language<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    left: bool,
    nav: WordNavigationType,
    whitespace_heuristics: bool,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let dm: &dyn CursorModel = model;
    let lang = lang.map(LanguageSupport::new);
    let auto_closed = cursors.auto_closed_ranges().closing_ranges();
    let pair_delete = |selection: Selection| {
        lang.as_ref().is_some_and(|lang| {
            is_auto_closing_pair_delete(config, lang, dm, &[selection], &auto_closed)
        })
    };
    delete_word_with(
        config,
        dm,
        cursors,
        left,
        nav,
        whitespace_heuristics,
        &pair_delete,
    )
}

/// [`delete_word_part`](super::delete_word_part) that, to the left, deletes
/// both characters of an auto-closing pair around a cursor, like
/// [`delete_word_with_language`] (Monaco `deleteWordPartLeft`, whose
/// candidates include `deleteWordLeft`'s).
pub fn delete_word_part_with_language<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    left: bool,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let dm: &dyn CursorModel = model;
    let lang = lang.map(LanguageSupport::new);
    let auto_closed = cursors.auto_closed_ranges().closing_ranges();
    let pair_delete = |selection: Selection| {
        lang.as_ref().is_some_and(|lang| {
            is_auto_closing_pair_delete(config, lang, dm, &[selection], &auto_closed)
        })
    };
    delete_word_part_with(config, dm, cursors, left, &pair_delete)
}

/// The Tab key like [`tab`], except that on a whitespace-only line it
/// indents straight to the indentation the language's indentation and
/// onEnter rules expect there (Monaco `TabOperation` with
/// `_goodIndentForLine`). With `lang == None` this is exactly [`tab`].
pub fn tab_with_language<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let Some(ctx) = lang else {
        return tab(config, model, cursors);
    };
    let lang = LanguageSupport::new(ctx);
    let dm: &dyn CursorModel = model;
    tab_with_good_indent(config, dm, cursors, Some(&lang), &|row| {
        good_indent_for_line(config, &lang, dm, row)
    })
}

/// Shift+Tab like [`outdent`](super::outdent), keeping the continuation
/// lines of doc comments (` * `) aligned by the language's onEnter rules
/// (Monaco `ShiftCommand`). With `lang == None` this is exactly
/// [`outdent`](super::outdent).
pub fn outdent_with_language<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    lang: Option<&LanguageContext>,
) -> EditOperation {
    let lang = lang.map(LanguageSupport::new);
    shift(config, model, cursors, true, lang.as_ref())
}

/// The closing characters a snippet's edits insert, to track as
/// auto-closed (Monaco `Cursor._findAutoClosingPairs`, which `executeEdits`
/// runs for the `'snippet'` source): typing `"` in `id="|"` inserted by a
/// snippet then types over the closing quote, and Backspace deletes both.
///
/// `inserted` is each edit's text with where it starts in the text after
/// the edits. Every text must be on one line and end with a single-character
/// pair's closing character, possibly followed by text without closing
/// characters, after the pair's opening character: `id=""` or `log()` gives
/// a pair, and nothing is tracked if any text doesn't. Pass the result to
/// [`AutoClosedRanges::push`](super::AutoClosedRanges::push).
pub fn snippet_auto_closed_pairs(
    lang: &LanguageContext,
    inserted: &[(Position, &str)],
) -> Vec<AutoClosedPair> {
    const CLOSERS: &[u16] = &[
        b')' as u16,
        b']' as u16,
        b'}' as u16,
        b'>' as u16,
        b'\'' as u16,
        b'"' as u16,
        b'`' as u16,
    ];
    if inserted.is_empty() {
        return Vec::new();
    }
    let lang = LanguageSupport::new(lang);
    let mut pairs = Vec::with_capacity(inserted.len());
    for &(start, text) in inserted {
        if text.is_empty() || text.contains('\n') {
            return Vec::new();
        }
        // Monaco's `/([)\]}>'"`])([^)\]}>'"`]*)$/`: the last closing
        // character.
        let units = strings::utf16(text);
        let Some(close_index) = units.iter().rposition(|c| CLOSERS.contains(c)) else {
            return Vec::new();
        };
        let close_char = String::from_utf16_lossy(&units[close_index..=close_index]);
        // `autoClosingPairsCloseSingleChar`: exactly one single-character
        // pair must close with it.
        let mut candidates = lang
            .auto_closing_pairs
            .iter()
            .filter(|p| p.open_len() == 1 && p.close_len() == 1 && p.close == close_char);
        let (Some(pair), None) = (candidates.next(), candidates.next()) else {
            return Vec::new();
        };
        let open = strings::utf16(&pair.open);
        // `text.lastIndexOf(openChar, closeCharIndex - 1)`.
        let Some(open_index) = units[..close_index.max(1)]
            .iter()
            .rposition(|&c| c == open[0])
        else {
            return Vec::new();
        };
        let row = start.row;
        let at = |index: usize| Position::new(row, start.column + index as u32);
        pairs.push(AutoClosedPair {
            closing: at(close_index)..at(close_index + 1),
            enclosing: at(open_index)..at(close_index + 1),
        });
    }
    pairs
}

// ---------------------------------------------------------------------------
// Enter

/// Monaco `typeCommand`, which inserts auto whitespace.
fn type_command(
    range: Range<Position>,
    text: String,
    keep_position: bool,
) -> Option<Box<dyn Command>> {
    auto_whitespace(if keep_position {
        Some(Box::new(ReplaceCommandWithoutChangingPosition {
            range,
            text,
        }))
    } else {
        ReplaceCommand::boxed(range, text)
    })
}

/// Monaco `EnterOperation._enter`.
pub(crate) fn enter_command_with_language(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    keep_position: bool,
    range: Range<Position>,
) -> Option<Box<dyn Command>> {
    if config.auto_indent == AutoIndent::None {
        return type_command(range, "\n".to_string(), keep_position);
    }
    let line = model.line_content(range.start.row);
    let indentation =
        strings::utf16_prefix(strings::leading_whitespace(&line), range.start.column).to_string();
    if config.auto_indent == AutoIndent::Keep {
        let text = format!("\n{}", config.normalize_indentation(&indentation));
        return type_command(range, text, keep_position);
    }

    if let Some(r) = enter_action(config.auto_indent, lang, model, &range) {
        match r.indent_action {
            IndentAction::None | IndentAction::Indent => {
                let text = format!(
                    "\n{}",
                    config.normalize_indentation(&format!("{}{}", r.indentation, r.append_text))
                );
                return type_command(range, text, keep_position);
            }
            IndentAction::IndentOutdent => {
                let normal_indent = config.normalize_indentation(&r.indentation);
                let increased_indent =
                    config.normalize_indentation(&format!("{}{}", r.indentation, r.append_text));
                let text = format!("\n{increased_indent}\n{normal_indent}");
                if keep_position {
                    return auto_whitespace(Some(Box::new(
                        ReplaceCommandWithoutChangingPosition { range, text },
                    )));
                }
                return auto_whitespace(Some(Box::new(ReplaceCommandWithOffsetCursorState {
                    range,
                    text,
                    line_delta: -1,
                    column_delta: strings::utf16_len(&increased_indent) as i32
                        - strings::utf16_len(&normal_indent) as i32,
                })));
            }
            IndentAction::Outdent => {
                let actual_indentation = unshift_indent(config, &r.indentation);
                let text = format!(
                    "\n{}",
                    config.normalize_indentation(&format!("{actual_indentation}{}", r.append_text))
                );
                return type_command(range, text, keep_position);
            }
        }
    }

    if config.auto_indent >= AutoIndent::Full
        && let Some((_, after_enter)) = indent_for_enter(config, lang, model, &range)
    {
        let mut old_end_view_column = config.visible_column_from_column(model, range.end) as i64;
        let old_end_column = range.end.column;
        let new_line_units = line_units(model, range.end.row);
        let first_non_whitespace = strings::first_non_whitespace_index(&new_line_units);
        let end = match first_non_whitespace {
            Some(f) => Position::new(range.end.row, range.end.column.max(f as u32)),
            None => Position::new(range.end.row, model.line_max_column(range.end.row)),
        };
        let range = range.start..end;
        let after_enter = config.normalize_indentation(&after_enter);
        let text = format!("\n{after_enter}");
        if keep_position {
            return auto_whitespace(Some(Box::new(ReplaceCommandWithoutChangingPosition {
                range,
                text,
            })));
        }
        let mut offset = 0i64;
        if let Some(f) = first_non_whitespace
            && old_end_column <= f as u32
        {
            if !config.insert_spaces {
                old_end_view_column =
                    (old_end_view_column as f64 / config.indent_size as f64).ceil() as i64;
            }
            offset = (old_end_view_column + 1 - strings::utf16_len(&after_enter) as i64 - 1).min(0);
        }
        return auto_whitespace(Some(Box::new(ReplaceCommandWithOffsetCursorState {
            range,
            text,
            line_delta: 0,
            column_delta: offset as i32,
        })));
    }
    let text = format!("\n{}", config.normalize_indentation(&indentation));
    type_command(range, text, keep_position)
}

// ---------------------------------------------------------------------------
// Auto-indent on type

/// Monaco `AutoIndentOperation.getEdits`.
fn auto_indent_type(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    selections: &[Selection],
    ch: &str,
) -> Option<EditOperation> {
    if config.auto_indent < AutoIndent::Full {
        return None;
    }
    let mut indentation_for_selections = Vec::with_capacity(selections.len());
    for &selection in selections {
        // Monaco `_findActualIndentationForSelection`.
        let actual = indent_action_for_type(config, lang, model, &selection.range(), ch)?;
        let current = indentation_at_position(model, selection.start());
        if actual == config.normalize_indentation(&current) {
            return None;
        }
        indentation_for_selections.push((selection, actual));
    }
    let pair = auto_closing_pair(config, lang, model, selections, ch, false);
    let auto_close_with_undo_stop = pair
        .as_ref()
        .is_some_and(|p| strings::utf16_len(&p.open) > strings::utf16_len(ch));
    let commands: Commands = indentation_for_selections
        .iter()
        .map(|(selection, indentation)| match &pair {
            Some(pair) if !auto_close_with_undo_stop => {
                // Apply both the auto-closing and the indentation edits.
                let (range, text) =
                    edit_from_indentation(config, model, indentation, *selection, ch, false);
                Some(Box::new(TypeWithIndentationAndAutoClosingCommand {
                    indent_range: range,
                    indent_text: text,
                    selection: selection.range(),
                    open: ch.to_string(),
                    close: pair.close.clone(),
                }) as Box<dyn Command>)
            }
            _ => {
                let (range, text) =
                    edit_from_indentation(config, model, indentation, *selection, ch, true);
                type_command(range, text, false)
            }
        })
        .collect();
    if let Some(pair) = pair
        && auto_close_with_undo_stop
    {
        return Some(type_and_auto_close_with_undo_stop(
            model,
            selections,
            commands,
            ch,
            &pair.close,
        ));
    }
    Some(execute_commands(
        model,
        selections,
        commands,
        EditOperationType::TypingOther,
        true,
        false,
    ))
}

/// Monaco `AutoIndentOperation._getEditFromIndentationAndSelection`.
fn edit_from_indentation(
    config: &CursorConfig,
    model: &dyn CursorModel,
    indentation: &str,
    selection: Selection,
    ch: &str,
    include_ch: bool,
) -> (Range<Position>, String) {
    let start = selection.start();
    let mut text = config.normalize_indentation(indentation);
    let line = model.line_content(start.row);
    if let Some(first) = strings::first_non_whitespace_index(&strings::utf16(&line)) {
        // JavaScript's `substring` swaps reversed bounds.
        let (a, b) = (first as u32, start.column);
        text.push_str(&strings::utf16_slice(&line, a.min(b), a.max(b)));
    }
    if include_ch {
        text.push_str(ch);
    }
    (Position::new(start.row, 0)..selection.end(), text)
}

// ---------------------------------------------------------------------------
// Auto-closing

/// Monaco `isAutoClosingOvertype`.
pub(crate) fn is_auto_closing_overtype(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    selections: &[Selection],
    auto_closed: &[Range<Position>],
    ch: &str,
) -> bool {
    if config.auto_closing_overtype == AutoClosingEditStrategy::Never
        || !lang.has_single_char_close(ch)
    {
        return false;
    }
    let ch_unit = strings::utf16(ch);
    for selection in selections {
        if !selection.is_empty() {
            return false;
        }
        let position = selection.head;
        let line = line_units(model, position.row);
        let c = position.column as usize;
        if line.get(c) != ch_unit.first() {
            return false;
        }
        // Do not over-type quotes after a backslash.
        if is_quote(ch) && c >= 2 && line[c - 1] == b'\\' as u16 {
            return false;
        }
        // Must over-type a closing character typed by the editor.
        if config.auto_closing_overtype == AutoClosingEditStrategy::Auto
            && !auto_closed.iter().any(|r| r.start == position)
        {
            return false;
        }
    }
    true
}

/// Monaco `AutoClosingOvertypeOperation._runAutoClosingOvertype`.
fn auto_closing_overtype(
    model: &dyn CursorModel,
    selections: &[Selection],
    ch: &str,
    prev: EditOperationType,
) -> EditOperation {
    let commands: Commands = selections
        .iter()
        .map(|s| {
            let p = s.head;
            ReplaceCommand::boxed(p..Position::new(p.row, p.column + 1), ch)
        })
        .collect();
    execute_commands(
        model,
        selections,
        commands,
        EditOperationType::TypingOther,
        should_push_stack_element_between(prev, EditOperationType::TypingOther),
        false,
    )
}

/// The pair to auto-close and the closing text to insert.
pub(crate) struct PairToClose {
    open: String,
    close: String,
}

/// Monaco `AutoClosingOpenCharTypeOperation.getAutoClosingPair`: the pair
/// whose opening text typing `ch` completes at every cursor, if it should be
/// auto-closed there. `ch_is_already_typed` when a composition has typed it
/// before each cursor already.
pub(crate) fn auto_closing_pair(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    selections: &[Selection],
    ch: &str,
    ch_is_already_typed: bool,
) -> Option<PairToClose> {
    if selections.iter().any(|s| !s.is_empty()) {
        return None;
    }
    // Work with the position before `ch` and the one after it, whether or
    // not it is in the text yet.
    let ch_len = if ch_is_already_typed {
        strings::utf16_len(ch)
    } else {
        0
    };
    let positions: Vec<(Position, Position)> = selections
        .iter()
        .map(|s| {
            let p = s.head;
            (Position::new(p.row, p.column.saturating_sub(ch_len)), p)
        })
        .collect();
    let before_positions: Vec<Position> = positions.iter().map(|(before, _)| *before).collect();
    // The longest auto-closing open pair ending in `ch`.
    let pair = find_auto_closing_pair_open(lang, model, &before_positions, ch)?;

    let (strategy, for_quotes, check_bracket_balance) = if is_quote(ch) {
        (config.auto_closing_quotes, true, false)
    } else if lang
        .block_comment_start
        .as_ref()
        .is_some_and(|start| !start.is_empty() && pair.open.contains(start.as_str()))
    {
        (config.auto_closing_comments, false, false)
    } else {
        (config.auto_closing_brackets, false, true)
    };
    if strategy == AutoClosingStrategy::Never {
        return None;
    }

    // Pairs can contain each other, e.g. `(`/`)` and `(*`/`*)`: typing `*` in
    // `(|)` should give `(*|*)`, not `(*|*))`.
    let contained_pair = find_contained_auto_closing_pair(lang, pair);
    let contained_close: Vec<u16> = contained_pair
        .map(|p| strings::utf16(&p.close))
        .unwrap_or_default();
    let mut is_contained_pair_present = true;
    let brackets = Brackets::configured(lang.config());
    let classifier = WordClassifier::new(&config.word_separators);

    for &(position, after) in &positions {
        let line = line_units(model, position.row);
        let c = position.column as usize;
        let line_before = &line[..c.min(line.len())];
        let line_after = &line[(after.column as usize).min(line.len())..];
        if !line_after.starts_with(&contained_close) {
            is_contained_pair_present = false;
        }
        // Only auto-close before an allowed character or another pair's
        // closing text.
        if let Some(&character_after) = line_after.first() {
            let character_after = char::from_u32(character_after as u32).unwrap_or('\u{FFFD}');
            let line_after = String::from_utf16_lossy(line_after);
            if !is_before_closing_brace(lang, &line_after)
                && !lang.should_auto_close_before(strategy, for_quotes, character_after)
            {
                return None;
            }
        }
        if check_bracket_balance
            && strategy != AutoClosingStrategy::Always
            // With `ch` typed, the brackets are no longer imbalanced.
            && !ch_is_already_typed
            && let Some(brackets) = &brackets
            && has_unmatched_closing_bracket_after(model, brackets, position, &pair.open, &|p| {
                lang.is_skippable(p)
            })
        {
            return None;
        }
        // Do not auto-close ' or " after a word character.
        if pair.open_len() == 1
            && (ch == "'" || ch == "\"")
            && strategy != AutoClosingStrategy::Always
            && let Some(&before) = line_before.last()
            && classifier.get(before) == WordCharClass::Regular
        {
            return None;
        }
        if !pair.is_ok(lang.token_before(model, position.row, position.column)) {
            return None;
        }
        // A quote may start a string or end one: ask what typing a neutral
        // character here would be.
        if let Some(neutral) = pair.find_neutral_character()
            && !pair.is_ok(lang.token_type_if_inserting(
                model,
                position.row,
                position.column,
                neutral,
            ))
        {
            return None;
        }
    }
    let close = if is_contained_pair_present {
        let close = strings::utf16(&pair.close);
        String::from_utf16_lossy(&close[..close.len() - contained_close.len()])
    } else {
        pair.close.clone()
    };
    Some(PairToClose {
        open: pair.open.clone(),
        close,
    })
}

/// Monaco `_findAutoClosingPairOpen`.
fn find_auto_closing_pair_open<'p>(
    lang: &'p LanguageSupport,
    model: &dyn CursorModel,
    positions: &[Position],
    ch: &str,
) -> Option<&'p AutoClosingPair> {
    let mut result: Option<&AutoClosingPair> = None;
    for candidate in lang.pairs_open_by_end(ch) {
        if result.is_some_and(|r| candidate.open_len() <= r.open_len()) {
            continue;
        }
        let is_match = positions.iter().all(|p| {
            let line = model.line_content(p.row);
            let start = p.column.saturating_sub(candidate.open_len() - 1);
            let relevant = strings::utf16_slice(&line, start, p.column);
            format!("{relevant}{ch}") == candidate.open
        });
        if is_match {
            result = Some(candidate);
        }
    }
    result
}

/// Monaco `_findContainedAutoClosingPair`.
fn find_contained_auto_closing_pair<'p>(
    lang: &'p LanguageSupport,
    pair: &AutoClosingPair,
) -> Option<&'p AutoClosingPair> {
    if pair.open_len() <= 1 {
        return None;
    }
    let last = pair.close.chars().next_back()?;
    let mut result: Option<&AutoClosingPair> = None;
    for candidate in lang.pairs_close_by_end(last) {
        if candidate.open != pair.open
            && pair.open.contains(candidate.open.as_str())
            && pair.close.ends_with(candidate.close.as_str())
            && result.is_none_or(|r| candidate.open_len() > r.open_len())
        {
            result = Some(candidate);
        }
    }
    result
}

/// Monaco `_isBeforeClosingBrace`.
fn is_before_closing_brace(lang: &LanguageSupport, line_after: &str) -> bool {
    let Some(next) = line_after.chars().next() else {
        return false;
    };
    let before_starting = lang
        .pairs_open_by_start(next)
        .any(|p| line_after.starts_with(p.open.as_str()));
    let before_closing = lang
        .pairs_close_by_start(next)
        .any(|p| line_after.starts_with(p.close.as_str()));
    !before_starting && before_closing
}

/// Monaco `_runAutoClosingOpenCharType`.
pub(crate) fn auto_closing_open_char_type(
    model: &dyn CursorModel,
    selections: &[Selection],
    ch: &str,
    ch_is_already_typed: bool,
    pair: &PairToClose,
) -> EditOperation {
    if !ch_is_already_typed && strings::utf16_len(&pair.open) > strings::utf16_len(ch) {
        let commands = selections
            .iter()
            .map(|s| type_command(s.range(), ch.to_string(), false))
            .collect();
        return type_and_auto_close_with_undo_stop(model, selections, commands, ch, &pair.close);
    }
    let commands: Commands = selections
        .iter()
        .map(|s| {
            Some(Box::new(TypeWithAutoClosingCommand::new(
                s.range(),
                ch,
                !ch_is_already_typed,
                &pair.close,
            )) as Box<dyn Command>)
        })
        .collect();
    execute_commands(
        model,
        selections,
        commands,
        EditOperationType::TypingOther,
        true,
        false,
    )
}

/// Monaco `typeAndAutoCloseWithUndoStop`: runs `type_commands` (which type
/// `ch`), then, as a follow-up after an undo stop, inserts the closing text
/// after `ch`, so undo first removes only the auto-closed text and keeps a
/// typed multi-character opening (e.g. `begin`).
fn type_and_auto_close_with_undo_stop(
    model: &dyn CursorModel,
    selections: &[Selection],
    type_commands: Commands,
    ch: &str,
    close: &str,
) -> EditOperation {
    let command_count = type_commands.len();
    let mut result = execute_commands(
        model,
        selections,
        type_commands,
        EditOperationType::TypingOther,
        true,
        true,
    );
    let ch_len = strings::utf16_len(ch);
    let after = &result.selections_after;
    let can_auto_close = !result.edits.is_empty()
        && after.len() == command_count
        && after
            .iter()
            .all(|s| s.is_empty() && s.head.column >= ch_len);
    if can_auto_close {
        // The follow-up only inserts text at the cursors, so it does not need
        // to read the (not yet edited) model.
        let commands: Commands = after
            .iter()
            .map(|s| {
                Some(
                    Box::new(TypeWithAutoClosingCommand::new(s.range(), ch, false, close))
                        as Box<dyn Command>,
                )
            })
            .collect();
        let follow_up = execute_commands(
            model,
            after,
            commands,
            EditOperationType::TypingOther,
            false,
            false,
        );
        result.follow_up = Some(Box::new(follow_up));
    }
    result
}

/// Monaco `isAutoClosingPairDelete`.
fn is_auto_closing_pair_delete(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    selections: &[Selection],
    auto_closed: &[Range<Position>],
) -> bool {
    if config.auto_closing_brackets == AutoClosingStrategy::Never
        && config.auto_closing_quotes == AutoClosingStrategy::Never
    {
        return false;
    }
    if config.auto_closing_delete == AutoClosingEditStrategy::Never {
        return false;
    }
    for selection in selections {
        if !selection.is_empty() {
            return false;
        }
        let position = selection.head;
        let line = line_units(model, position.row);
        let c = position.column as usize;
        if c < 1 || c >= line.len() {
            return false;
        }
        let character = String::from_utf16_lossy(&line[c - 1..c]);
        let after_character = String::from_utf16_lossy(&line[c..c + 1]);
        let mut candidates = lang.pairs_open_by_end(&character).peekable();
        if candidates.peek().is_none() {
            return false;
        }
        let strategy = if is_quote(&character) {
            config.auto_closing_quotes
        } else {
            config.auto_closing_brackets
        };
        if strategy == AutoClosingStrategy::Never {
            return false;
        }
        if !candidates.any(|p| p.open == character && p.close == after_character) {
            return false;
        }
        // Must delete the pair only if the editor inserted it.
        if config.auto_closing_delete == AutoClosingEditStrategy::Auto
            && !auto_closed.iter().any(|r| r.start == position)
        {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Surround selection

/// Monaco `shouldSurroundChar`.
pub(crate) fn should_surround_char(config: &CursorConfig, ch: &str) -> bool {
    if is_quote(ch) {
        matches!(
            config.auto_surround,
            AutoSurroundStrategy::Quotes | AutoSurroundStrategy::LanguageDefined
        )
    } else {
        matches!(
            config.auto_surround,
            AutoSurroundStrategy::Brackets | AutoSurroundStrategy::LanguageDefined
        )
    }
}

/// Monaco `SurroundSelectionOperation._isSurroundSelectionType`.
fn is_surround_selection_type(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    selections: &[Selection],
    ch: &str,
) -> bool {
    if !should_surround_char(config, ch) || lang.surrounding_close(ch).is_none() {
        return false;
    }
    let is_typing_a_quote = is_quote(ch);
    for selection in selections {
        if selection.is_empty() {
            return false;
        }
        let (start, end) = (selection.start(), selection.end());
        let only_whitespace = (start.row..=end.row).all(|row| {
            let line = model.line_content(row);
            let from = if row == start.row { start.column } else { 0 };
            let to = if row == end.row { end.column } else { u32::MAX };
            strings::utf16_slice(&line, from, to)
                .chars()
                .all(|c| c == ' ' || c == '\t')
        });
        if only_whitespace {
            return false;
        }
        if is_typing_a_quote
            && start.row == end.row
            && start.column + 1 == end.column
            && is_quote(&strings::utf16_slice(
                &model.line_content(start.row),
                start.column,
                end.column,
            ))
        {
            // Typing a quote over another quote replaces it.
            return false;
        }
    }
    true
}

/// Monaco `SurroundSelectionOperation._runSurroundSelectionType`.
fn surround_selection_type(
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    selections: &[Selection],
    ch: &str,
) -> EditOperation {
    let close = lang.surrounding_close(ch).unwrap_or_default().to_string();
    let commands: Commands = selections
        .iter()
        .map(|&selection| {
            Some(Box::new(SurroundSelectionCommand {
                range: selection.range(),
                before: ch.to_string(),
                after: close.clone(),
            }) as Box<dyn Command>)
        })
        .collect();
    execute_commands(
        model,
        selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    )
}

// ---------------------------------------------------------------------------
// Electric characters

/// Monaco `InterceptorElectricCharOperation`: typing a closing bracket as
/// the first thing on a line lines it up with its opening bracket.
fn electric_char(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    selections: &[Selection],
    ch: &str,
    prev: EditOperationType,
) -> Option<EditOperation> {
    // Only for a single cursor: several cursors typing brackets would
    // interfere with bracket matching.
    let [selection] = selections else {
        return None;
    };
    if !is_electric_character(lang, ch) || !selection.is_empty() {
        return None;
    }
    let position = selection.head;
    let close = on_electric_character(lang, model, ch, position)?;
    // Matched by the brackets of the language at the cursor.
    let (embedded, _) = scoped_support(lang, model, position);
    let brackets = Brackets::configured(embedded.as_ref().unwrap_or(lang).config())?;
    let line = model.line_content(position.row);
    let line_with_ch: Vec<u16> = strings::utf16(&format!("{line}{ch}"));
    let close_units = strings::utf16(&close);
    let end_column = (0..=line_with_ch.len().saturating_sub(close_units.len()))
        .rev()
        .find(|&i| line_with_ch[i..].starts_with(&close_units))
        .unwrap_or(0) as u32;
    let matched = find_matching_bracket_up(
        model,
        &brackets,
        &close,
        Position::new(position.row, end_column),
        &|p| lang.is_skippable(p),
    )?;
    if matched.start.row == position.row {
        // Matched something on the same line: no change in indentation.
        return None;
    }
    let match_line = model.line_content(matched.start.row);
    let new_indentation = config.normalize_indentation(strings::leading_whitespace(&match_line));
    let first_non_blank = strings::first_non_whitespace_index(&strings::utf16(&line))
        .map(|i| i as u32)
        .unwrap_or(position.column);
    let (a, b) = (first_non_blank, position.column);
    let prefix = strings::utf16_slice(&line, a.min(b), a.max(b));
    let type_text = format!("{new_indentation}{prefix}{ch}");
    let kind = typing_operation(&type_text, prev);
    let commands: Commands = vec![ReplaceCommand::boxed(
        Position::new(position.row, 0)..position,
        type_text,
    )];
    Some(execute_commands(
        model, selections, commands, kind, false, true,
    ))
}

// ---------------------------------------------------------------------------
// Commands

/// Monaco `TypeWithAutoClosingCommand`: types the opening character (unless
/// already typed) and the closing text, leaving the cursor between them.
struct TypeWithAutoClosingCommand {
    range: Range<Position>,
    text: String,
    open_len: u32,
    close_len: u32,
}

impl TypeWithAutoClosingCommand {
    fn new(range: Range<Position>, open: &str, insert_open: bool, close: &str) -> Self {
        let text = if insert_open {
            format!("{open}{close}")
        } else {
            close.to_string()
        };
        Self {
            range,
            text,
            open_len: strings::utf16_len(open),
            close_len: strings::utf16_len(close),
        }
    }
}

/// Monaco `BaseTypeWithAutoClosingCommand._computeCursorStateWithRange`.
fn auto_closed_from_range(
    range: &Range<Position>,
    open_len: u32,
    close_len: u32,
) -> AutoClosedPair {
    let end = range.end;
    AutoClosedPair {
        closing: Position::new(range.start.row, end.column.saturating_sub(close_len))..end,
        enclosing: Position::new(
            range.start.row,
            end.column.saturating_sub(open_len + close_len),
        )..end,
    }
}

impl Command for TypeWithAutoClosingCommand {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit(self.range.clone(), &self.text);
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let end = helper.inverse_edit_ranges()[0].end;
        Selection::collapsed(Position::new(
            end.row,
            end.column.saturating_sub(self.close_len),
        ))
    }

    fn auto_closed(&self, helper: &CursorStateHelper) -> Option<AutoClosedPair> {
        Some(auto_closed_from_range(
            &helper.inverse_edit_ranges()[0],
            self.open_len,
            self.close_len,
        ))
    }
}

/// Monaco `TypeWithIndentationAndAutoClosingCommand`: re-indents the line
/// and auto-closes the typed character in one step.
struct TypeWithIndentationAndAutoClosingCommand {
    indent_range: Range<Position>,
    indent_text: String,
    selection: Range<Position>,
    open: String,
    close: String,
}

impl TypeWithIndentationAndAutoClosingCommand {
    fn union(helper: &CursorStateHelper) -> Range<Position> {
        let inverse = helper.inverse_edit_ranges();
        let first = inverse[0].clone();
        match inverse.get(1) {
            Some(second) => first.start.min(second.start)..first.end.max(second.end),
            None => first,
        }
    }
}

impl Command for TypeWithIndentationAndAutoClosingCommand {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit(self.indent_range.clone(), &self.indent_text);
        builder.add_edit(
            self.selection.clone(),
            &format!("{}{}", self.open, self.close),
        );
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let end = helper.inverse_edit_ranges()[0].end;
        Selection::collapsed(Position::new(
            end.row,
            end.column + strings::utf16_len(&self.open),
        ))
    }

    fn auto_closed(&self, helper: &CursorStateHelper) -> Option<AutoClosedPair> {
        Some(auto_closed_from_range(
            &Self::union(helper),
            strings::utf16_len(&self.open),
            strings::utf16_len(&self.close),
        ))
    }
}

/// Monaco `SurroundSelectionCommand`.
struct SurroundSelectionCommand {
    range: Range<Position>,
    before: String,
    after: String,
}

impl Command for SurroundSelectionCommand {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        let start = self.range.start;
        let end = self.range.end;
        builder.add_edit(start..start, &self.before);
        if self.after.is_empty() {
            // Keep a second inverse range for `compute_cursor_state`.
            builder.add_noop(end);
        } else {
            builder.add_edit(end..end, &self.after);
        }
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let inverse = helper.inverse_edit_ranges();
        let first = &inverse[0];
        let second = inverse.get(1).unwrap_or(first);
        Selection::new(
            first.end,
            Position::new(
                second.end.row,
                second
                    .end
                    .column
                    .saturating_sub(strings::utf16_len(&self.after)),
            ),
        )
    }
}
