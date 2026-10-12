//! In-place replace: step the number or value under the cursor up or down.
//!
//! Ports Monaco's `src/vs/editor/contrib/inPlaceReplace/browser/inPlaceReplace.ts`,
//! `inPlaceReplaceCommand.ts`, `BasicInplaceReplace` from
//! `common/languages/supports/inplaceReplaceSupport.ts` and the worker's
//! `navigateValueSet` (MIT).

use std::ops::Range;

use syntax::Pattern;

use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType, execute_commands,
};
use super::{CursorCollection, CursorConfig, CursorModel, Position, Selection, strings};

/// Monaco's `DEFAULT_WORD_REGEXP` (a compiled copy).
pub(crate) fn default_word_regex() -> &'static syntax::fancy_regex::Regex {
    static REGEX: std::sync::OnceLock<syntax::fancy_regex::Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| syntax::fancy_regex::Regex::new(DEFAULT_WORD_PATTERN).expect("valid"))
}

/// Monaco's `DEFAULT_WORD_REGEXP`: numbers like `-1.5` or runs of
/// non-separator characters.
const DEFAULT_WORD_PATTERN: &str = r#"(-?\d*\.\d\w*)|([^`~!@#$%^&*()\-=+\[{\]}\\|;:'",.<>/?\s]+)"#;

/// Monaco `BasicInplaceReplace._defaultValueSet`.
const VALUE_SETS: &[&[&str]] = &[
    &["true", "false"],
    &["True", "False"],
    &[
        "Private",
        "Public",
        "Friend",
        "ReadOnly",
        "Partial",
        "Protected",
        "WriteOnly",
    ],
    &["public", "protected", "private"],
];

/// Monaco `numberReplace`: steps the last decimal digit.
fn number_replace(value: &str, up: bool) -> Option<String> {
    let n: f64 = value.trim().parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    if n == 0.0 && !up {
        // Don't go negative.
        return None;
    }
    let decimals = value.encode_utf16().count() as i32
        - value
            .rfind('.')
            .map_or(0, |i| value[..i].encode_utf16().count() as i32 + 1);
    let precision = 10f64.powi(decimals);
    let mut stepped = (n * precision).floor();
    stepped += if up { precision } else { -precision };
    Some(format!("{}", stepped / precision))
}

fn value_set_replace(value: &str, up: bool) -> Option<String> {
    VALUE_SETS.iter().find_map(|set| {
        let index = set.iter().position(|v| *v == value)? as isize;
        let next = (index + if up { 1 } else { -1 }).rem_euclid(set.len() as isize);
        Some(set[next as usize].to_string())
    })
}

fn navigate(text: &str, up: bool) -> Option<String> {
    number_replace(text, up).or_else(|| value_set_replace(text, up))
}

/// Monaco `model.getWordAtPosition(position, wordDefinition)`: the match of
/// `pattern` containing (or touching) `column` on `line`.
pub(crate) fn word_at(
    line: &str,
    column: u32,
    pattern: &syntax::fancy_regex::Regex,
) -> Option<Range<u32>> {
    let mut start = 0;
    while let Ok(Some(m)) = pattern.find_from_pos(line, start) {
        let word = strings::utf16_len(&line[..m.start()])..strings::utf16_len(&line[..m.end()]);
        if word.start <= column && column <= word.end && !m.as_str().is_empty() {
            return Some(word);
        }
        if word.start > column {
            break;
        }
        start = if m.end() > m.start() {
            m.end()
        } else {
            m.end() + line[m.end()..].chars().next().map_or(1, char::len_utf8)
        };
        if start > line.len() {
            break;
        }
    }
    None
}

/// Monaco `InPlaceReplaceCommand`.
struct InPlaceReplaceCommand {
    range: Range<Position>,
    original: Selection,
    text: String,
}

impl Command for InPlaceReplaceCommand {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit(self.range.clone(), &self.text);
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let end = helper.inverse_edit_ranges()[0].end;
        if !self.original.is_empty() {
            // Keep the selection, extended over the new text.
            let len = strings::utf16_len(&self.text);
            return Selection::new(Position::new(end.row, end.column.saturating_sub(len)), end);
        }
        Selection::collapsed(Position::new(
            end.row,
            self.original.head.column.min(end.column),
        ))
    }
}

/// Replaces the selected text, or the number or value at the primary cursor,
/// with the next (`up`) or previous one in its set: `1` → `2`, `1.5` →
/// `2.5`, `true` → `false`, `public` → `protected`. `up` is Monaco's
/// `InplaceReplaceSupport` argument: `editor.action.inPlaceReplace.down`
/// ("Replace with Next Value", Cmd/Ctrl+Shift+.) passes `true` and `.up`
/// ("Replace with Previous Value", Cmd/Ctrl+Shift+,) `false`.
/// `word_pattern` is the language's `wordPattern`. Also returns the
/// replaced range, which Monaco briefly highlights.
pub fn in_place_replace(
    _config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &CursorCollection,
    word_pattern: Option<&Pattern>,
    up: bool,
) -> Option<(EditOperation, Range<Position>)> {
    let model: &dyn CursorModel = model;
    let selections = cursors.selections();
    let mut selection = selections[0];
    let (start, end) = (selection.start(), selection.end());
    if start.row != end.row {
        return None;
    }
    let line = model.line_content(start.row);
    // An empty selection looks at the character after the cursor.
    let mut range = start.column..end.column;
    if range.is_empty() {
        range.end += 1;
    }
    let selected = strings::utf16_slice(&line, range.start, range.end);
    let pattern = match word_pattern.and_then(Pattern::regex) {
        Some(pattern) => pattern,
        None => default_word_regex(),
    };
    let word = word_at(&line, start.column, pattern);

    let (edit_range, value) = match (!selected.is_empty())
        .then(|| navigate(&selected, up))
        .flatten()
    {
        Some(value) => (range, value),
        None => {
            let word = word?;
            let text = strings::utf16_slice(&line, word.start, word.end);
            (word, navigate(&text, up)?)
        }
    };
    let diff = strings::utf16_len(&value) as i64 - (end.column as i64 - start.column as i64);
    if diff > 1 {
        let head = Position::new(end.row, (end.column as i64 + diff - 1) as u32);
        selection = Selection::new(selection.start(), head);
    }
    let row = start.row;
    let edit = Position::new(row, edit_range.start)..Position::new(row, edit_range.end);
    let highlight = Position::new(row, edit_range.start)
        ..Position::new(row, edit_range.start + strings::utf16_len(&value));
    let mut commands: Vec<Option<Box<dyn Command>>> = vec![Some(Box::new(InPlaceReplaceCommand {
        range: edit,
        original: selection,
        text: value,
    }))];
    commands.resize_with(selections.len(), || None);
    let operation = execute_commands(
        model,
        &selections,
        commands,
        EditOperationType::Other,
        true,
        true,
    );
    Some((operation, highlight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_value_sets() {
        assert_eq!(number_replace("1", true).as_deref(), Some("2"));
        assert_eq!(number_replace("9", true).as_deref(), Some("10"));
        assert_eq!(number_replace("1.5", true).as_deref(), Some("2.5"));
        assert_eq!(number_replace("1.5", false).as_deref(), Some("0.5"));
        assert_eq!(number_replace("0", false), None);
        assert_eq!(number_replace("-1", true).as_deref(), Some("0"));
        assert_eq!(number_replace("abc", true), None);
        assert_eq!(value_set_replace("true", true).as_deref(), Some("false"));
        assert_eq!(value_set_replace("false", true).as_deref(), Some("true"));
        assert_eq!(
            value_set_replace("public", false).as_deref(),
            Some("private")
        );
        assert_eq!(value_set_replace("maybe", true), None);
    }
}
