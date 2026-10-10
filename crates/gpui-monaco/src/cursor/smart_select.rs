//! Smart select (expand and shrink selection): the ranges around a
//! position, from providers, the syntax tree or brackets, and words.
//!
//! Ports Monaco's `src/vs/editor/contrib/smartSelect/browser/`
//! (`smartSelect.ts` `provideSelectionRanges`, `wordSelections.ts`) (MIT).

use std::ops::Range;

pub use super::brackets::bracket_selection_ranges;
use super::{CursorModel, Position, line_units, max_position};

/// Monaco `editor.smartSelect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmartSelectOptions {
    /// `selectLeadingAndTrailingWhitespace`: add the whole lines, with and
    /// without their indentation, between ranges that span lines.
    pub select_leading_and_trailing_whitespace: bool,
    /// `selectSubwords`: select camelCase and snake_case word parts first.
    pub select_subwords: bool,
}

impl Default for SmartSelectOptions {
    fn default() -> Self {
        Self {
            select_leading_and_trailing_whitespace: true,
            select_subwords: true,
        }
    }
}

/// Monaco `WordSelectionRangeProvider`: the word part, the word, a
/// whitespace-only line, and the whole text.
pub fn word_selection_ranges(
    model: &dyn CursorModel,
    word_regex: &syntax::fancy_regex::Regex,
    position: Position,
    select_subwords: bool,
) -> Vec<Range<Position>> {
    let mut ranges = Vec::new();
    let line = model.line_content(position.row);
    let word = super::in_place_replace::word_at(&line, position.column, word_regex);
    if let Some(word) = &word {
        if select_subwords {
            let units = line_units(model, position.row);
            let text = &units[word.start as usize..word.end as usize];
            if let Some(part) = word_part(text, position.column - word.start) {
                ranges.push(
                    Position::new(position.row, word.start + part.start)
                        ..Position::new(position.row, word.start + part.end),
                );
            }
        }
        ranges.push(Position::new(position.row, word.start)..Position::new(position.row, word.end));
    }
    let len = model.line_max_column(position.row);
    if len > 0 && line.trim().is_empty() {
        ranges.push(Position::new(position.row, 0)..Position::new(position.row, len));
    }
    ranges.push(Position::new(0, 0)..max_position(model));
    ranges
}

/// Monaco `_addInWordRanges`: the camelCase hump or `_`/`-` separated part
/// of `word` around `offset` (UTF-16 units).
fn word_part(word: &[u16], offset: u32) -> Option<Range<u32>> {
    let is_lower = |c: Option<u16>| c.is_some_and(|c| (b'a' as u16..=b'z' as u16).contains(&c));
    let is_upper = |c: Option<u16>| c.is_some_and(|c| (b'A' as u16..=b'Z' as u16).contains(&c));
    let is_separator = |c: Option<u16>| c == Some(b'_' as u16) || c == Some(b'-' as u16);
    let at = |i: i64| (i >= 0).then(|| word.get(i as usize).copied()).flatten();
    let offset = offset as i64;
    let mut start = offset;
    let mut end = offset;
    let mut last: Option<u16> = None;
    while start >= 0 {
        let ch = at(start);
        if (start != offset && is_separator(ch)) || (is_lower(ch) && is_upper(last)) {
            break;
        }
        last = ch;
        start -= 1;
    }
    start += 1;
    while end < word.len() as i64 {
        let ch = at(end);
        if (is_upper(ch) && is_lower(last)) || is_separator(ch) {
            break;
        }
        last = ch;
        end += 1;
    }
    (start < end).then_some(start as u32..end as u32)
}

/// Monaco `provideSelectionRanges`' merging for one position: keeps ranges
/// containing `position`, orders them inside out, drops ranges that don't
/// contain the previous one, and (with `trivia`) adds whole-line ranges
/// where a range grows onto new lines.
pub fn merge_selection_ranges(
    model: &dyn CursorModel,
    mut raw: Vec<Range<Position>>,
    position: Position,
    trivia: bool,
) -> Vec<Range<Position>> {
    raw.retain(|range| range.start <= position && position <= range.end);
    // Later starts first; for equal starts, earlier ends first.
    raw.sort_by(|a, b| b.start.cmp(&a.start).then(a.end.cmp(&b.end)));
    let contains = |outer: &Range<Position>, inner: &Range<Position>| {
        outer.start <= inner.start && inner.end <= outer.end
    };
    let mut ranges: Vec<Range<Position>> = Vec::new();
    for range in raw {
        if ranges
            .last()
            .is_none_or(|last| contains(&range, last) && range != *last)
        {
            ranges.push(range);
        }
    }
    if !trivia || ranges.is_empty() {
        return ranges;
    }
    // Monaco's 1-based columns, where 0 means a blank line.
    let first = |row: u32| -> i64 {
        super::first_non_whitespace_column(model, row).map_or(-1, |c| c as i64)
    };
    let last = |row: u32| -> i64 {
        let units = line_units(model, row);
        super::strings::last_non_whitespace_index(&units, None).map_or(-1, |i| i as i64 + 1)
    };
    let point = |row: u32, column: i64| Position::new(row, column.max(0) as u32);
    let mut result = vec![ranges[0].clone()];
    for pair in ranges.windows(2) {
        let (prev, cur) = (&pair[0], &pair[1]);
        if cur.start.row != prev.start.row || cur.end.row != prev.end.row {
            let no_whitespace = point(prev.start.row, first(prev.start.row))
                ..point(prev.end.row, last(prev.end.row));
            if contains(&no_whitespace, prev)
                && no_whitespace != *prev
                && contains(cur, &no_whitespace)
                && *cur != no_whitespace
            {
                result.push(no_whitespace.clone());
            }
            let full = Position::new(prev.start.row, 0)
                ..Position::new(prev.end.row, model.line_max_column(prev.end.row));
            if contains(&full, prev)
                && full != no_whitespace
                && contains(cur, &full)
                && *cur != full
            {
                result.push(full);
            }
        }
        result.push(cur.clone());
    }
    result
}
