//! Bracket matching over the text, skipping brackets in strings and
//! comments.
//!
//! Ports the bracket queries the cursor logic needs from Monaco's
//! `src/vs/editor/common/model/bracketPairsTextModelPart/bracketPairsImpl.ts`
//! (`matchBracket`, `findEnclosingBrackets`, `findNextBracket`,
//! `findMatchingBracketUp`, the fallback
//! `_findMatchingBracketUp`) and `hasUnmatchedClosingBracketAfter` from
//! `bracketPairsTree/bracketPairsTree.ts`, plus the bracket regexes of
//! `src/vs/editor/common/languages/supports/richEditBrackets.ts` (MIT).
//! Monaco answers these from an incrementally maintained bracket-pair AST;
//! this port scans the text instead, with the same notion of which closing
//! brackets are unmatched (a closing bracket closes the innermost open
//! bracket of its kind, abandoning any unclosed brackets in between).

use std::ops::Range;

use syntax::LanguageConfiguration;

use super::{CursorModel, Position, line_units};

/// How far [`matching_bracket`] scans in each direction, in UTF-16 units.
const MAX_SCAN_UNITS: usize = 20_000;
/// Documents longer than this are not scanned for unmatched closing brackets
/// (Monaco also skips that check until the whole document is tokenized).
const MAX_BALANCE_LINES: u32 = 10_000;

/// Monaco's default brackets, used when a language configures none.
const DEFAULT_BRACKETS: [(&str, &str); 3] = [("(", ")"), ("[", "]"), ("{", "}")];

/// A language's bracket pairs, as UTF-16.
pub(crate) struct Brackets {
    pairs: Vec<(Vec<u16>, Vec<u16>)>,
}

/// A bracket found in a line.
#[derive(Clone, Copy, Debug)]
struct Found {
    start: u32,
    len: u32,
    pair: usize,
    is_open: bool,
}

fn is_word_unit(c: u16) -> bool {
    c == b'_' as u16 || char::from_u32(c as u32).is_some_and(|c| c.is_ascii_alphanumeric())
}

impl Brackets {
    /// The configured brackets (Monaco `RichEditBrackets`), or `None` if the
    /// language has none.
    pub(crate) fn configured(config: &LanguageConfiguration) -> Option<Self> {
        let pairs: Vec<(Vec<u16>, Vec<u16>)> = config
            .brackets()
            .iter()
            .filter(|(open, close)| !open.is_empty() && !close.is_empty())
            .map(|(open, close)| {
                (
                    open.encode_utf16().collect(),
                    close.encode_utf16().collect(),
                )
            })
            .collect();
        (!pairs.is_empty()).then_some(Self { pairs })
    }

    fn configured_or_default(config: &LanguageConfiguration) -> Self {
        Self::configured(config).unwrap_or_else(|| Self {
            pairs: DEFAULT_BRACKETS
                .iter()
                .map(|(open, close)| {
                    (
                        open.encode_utf16().collect(),
                        close.encode_utf16().collect(),
                    )
                })
                .collect(),
        })
    }

    pub(crate) fn has_open(&self, open: &str) -> bool {
        let open: Vec<u16> = open.encode_utf16().collect();
        self.pairs.iter().any(|(o, _)| *o == open)
    }

    /// Whether `text` matches at `i` in `line`, honoring word boundaries for
    /// brackets made of letters (Monaco `prepareBracketForRegExp`).
    fn matches_at(line: &[u16], i: usize, text: &[u16]) -> bool {
        if !line[i..].starts_with(text) {
            return false;
        }
        let wordy = text.iter().all(|&c| is_word_unit(c) || c == b' ' as u16);
        if wordy {
            if i > 0 && is_word_unit(line[i - 1]) {
                return false;
            }
            if let Some(&next) = line.get(i + text.len())
                && is_word_unit(next)
            {
                return false;
            }
        }
        true
    }

    /// The brackets in `line`, left to right, skipping those where `skip`
    /// says so (given the bracket's start column).
    fn in_line(&self, line: &[u16], skip: &dyn Fn(u32) -> bool) -> Vec<Found> {
        let mut result = Vec::new();
        let mut i = 0;
        while i < line.len() {
            let mut best: Option<Found> = None;
            for (pair, (open, close)) in self.pairs.iter().enumerate() {
                for (text, is_open) in [(open, true), (close, false)] {
                    if best.is_none_or(|b| text.len() as u32 > b.len)
                        && Self::matches_at(line, i, text)
                    {
                        best = Some(Found {
                            start: i as u32,
                            len: text.len() as u32,
                            pair,
                            is_open,
                        });
                    }
                }
            }
            match best {
                Some(found) => {
                    if !skip(found.start) {
                        result.push(found);
                    }
                    i += found.len as usize;
                }
                None => i += 1,
            }
        }
        result
    }

    fn open_text(&self, pair: usize) -> &[u16] {
        &self.pairs[pair].0
    }

    fn close_text(&self, pair: usize) -> &[u16] {
        &self.pairs[pair].1
    }
}

fn found_range(row: u32, found: &Found) -> Range<Position> {
    Position::new(row, found.start)..Position::new(row, found.start + found.len)
}

/// Monaco `hasUnmatchedClosingBracketAfter`: whether a closing bracket for
/// `open` that no opening bracket matches comes at or after `position`.
/// `false` if `open` is not one of the language's opening brackets.
pub(crate) fn has_unmatched_closing_bracket_after(
    model: &dyn CursorModel,
    brackets: &Brackets,
    position: Position,
    open: &str,
    skip: &dyn Fn(Position) -> bool,
) -> bool {
    if !brackets.has_open(open) || model.line_count() > MAX_BALANCE_LINES {
        return false;
    }
    let open: Vec<u16> = open.encode_utf16().collect();
    let mut stack: Vec<usize> = Vec::new();
    for row in 0..model.line_count() {
        let line = line_units(model, row);
        for found in brackets.in_line(&line, &|i| skip(Position::new(row, i))) {
            if found.is_open {
                stack.push(found.pair);
                continue;
            }
            let close = brackets.close_text(found.pair);
            match stack.iter().rposition(|&p| brackets.close_text(p) == close) {
                Some(index) => stack.truncate(index),
                None => {
                    // An unexpected closing bracket.
                    let closes_open = brackets
                        .pairs
                        .iter()
                        .any(|(o, c)| *o == open && c.as_slice() == close);
                    if closes_open && Position::new(row, found.start) >= position {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Monaco `findMatchingBracketUp`: the opening bracket of the innermost
/// unclosed pair before `position` that `close` would close.
pub(crate) fn find_matching_bracket_up(
    model: &dyn CursorModel,
    brackets: &Brackets,
    close: &str,
    position: Position,
    skip: &dyn Fn(Position) -> bool,
) -> Option<Range<Position>> {
    let close: Vec<u16> = close.to_lowercase().encode_utf16().collect();
    let kind = |pair: usize| brackets.close_text(pair) == close.as_slice();
    if !(0..brackets.pairs.len()).any(kind) {
        return None;
    }
    let mut depth = 0usize;
    let mut scanned = 0usize;
    let mut row = position.row as i64;
    while row >= 0 && scanned <= MAX_SCAN_UNITS * 50 {
        let r = row as u32;
        let window = if r == position.row {
            Window::Columns(position.column.saturating_sub(WINDOW)..position.column)
        } else {
            Window::Tail
        };
        let (found, len) = scan_row(model, brackets, r, window, skip);
        scanned += len as usize + 1;
        for f in found.iter().rev() {
            if r == position.row && f.start + f.len > position.column {
                continue;
            }
            if !kind(f.pair) {
                continue;
            }
            if f.is_open {
                if depth == 0 {
                    return Some(found_range(r, f));
                }
                depth -= 1;
            } else {
                depth += 1;
            }
        }
        row -= 1;
    }
    None
}

/// The brackets of `row` in `columns` (clamped to it), with columns in the
/// row: long lines are only scanned near where they matter.
fn found_in(
    model: &dyn CursorModel,
    brackets: &Brackets,
    row: u32,
    columns: Range<u32>,
    skip: &dyn Fn(Position) -> bool,
) -> Vec<Found> {
    scan_row(model, brackets, row, Window::Columns(columns), skip).0
}

/// Which columns of a row [`scan_row`] scans.
enum Window {
    /// These, clamped to the row.
    Columns(Range<u32>),
    /// The last `WINDOW` columns.
    Tail,
}

/// [`found_in`] for `window`, with how many columns it scanned. Looks the
/// row's length up once: the scans visit many rows, and each lookup seeks
/// the rope.
fn scan_row(
    model: &dyn CursorModel,
    brackets: &Brackets,
    row: u32,
    window: Window,
    skip: &dyn Fn(Position) -> bool,
) -> (Vec<Found>, u32) {
    let max = model.line_max_column(row);
    let (from, to) = match window {
        Window::Columns(columns) => (columns.start.min(max), columns.end.min(max)),
        Window::Tail => (max.saturating_sub(WINDOW), max),
    };
    let units: Vec<u16> = model.line_slice(row, from..to).encode_utf16().collect();
    let mut found = brackets.in_line(&units, &|i| skip(Position::new(row, from + i)));
    for f in &mut found {
        f.start += from;
    }
    (found, to.saturating_sub(from))
}

/// How far from a position long lines are scanned.
const WINDOW: u32 = MAX_SCAN_UNITS as u32;

/// Scans from (after) `from` for the bracket closing `pair`, counting
/// nested brackets of the same pair.
fn find_close_forward(
    model: &dyn CursorModel,
    brackets: &Brackets,
    pair: usize,
    from: Position,
    skip: &dyn Fn(Position) -> bool,
) -> Option<Range<Position>> {
    let mut depth = 0usize;
    let mut scanned = 0usize;
    let open = brackets.open_text(pair);
    let close = brackets.close_text(pair);
    for row in from.row..model.line_count() {
        let window = if row == from.row {
            from.column..from.column.saturating_add(WINDOW)
        } else {
            0..WINDOW
        };
        let (found, len) = scan_row(model, brackets, row, Window::Columns(window), skip);
        for f in found {
            if row == from.row && f.start < from.column {
                continue;
            }
            let text = if f.is_open {
                brackets.open_text(f.pair)
            } else {
                brackets.close_text(f.pair)
            };
            if f.is_open && text == open {
                depth += 1;
            } else if !f.is_open && text == close {
                if depth == 0 {
                    return Some(found_range(row, &f));
                }
                depth -= 1;
            }
        }
        scanned += len as usize + 1;
        if scanned > MAX_SCAN_UNITS {
            break;
        }
    }
    None
}

/// Scans backwards from (before) `to` for the bracket opening `pair`.
fn find_open_backward(
    model: &dyn CursorModel,
    brackets: &Brackets,
    pair: usize,
    to: Position,
    skip: &dyn Fn(Position) -> bool,
) -> Option<Range<Position>> {
    let mut depth = 0usize;
    let mut scanned = 0usize;
    let open = brackets.open_text(pair);
    let close = brackets.close_text(pair);
    for row in (0..=to.row).rev() {
        let window = if row == to.row {
            Window::Columns(to.column.saturating_sub(WINDOW)..to.column)
        } else {
            Window::Tail
        };
        let (found, len) = scan_row(model, brackets, row, window, skip);
        for f in found.iter().rev() {
            if row == to.row && f.start + f.len > to.column {
                continue;
            }
            let text = if f.is_open {
                brackets.open_text(f.pair)
            } else {
                brackets.close_text(f.pair)
            };
            if !f.is_open && text == close {
                depth += 1;
            } else if f.is_open && text == open {
                if depth == 0 {
                    return Some(found_range(row, f));
                }
                depth -= 1;
            }
        }
        scanned += len as usize + 1;
        if scanned > MAX_SCAN_UNITS {
            break;
        }
    }
    None
}

/// The bracket pair to highlight or jump between for a cursor at `position`
/// (Monaco `matchBracket`, extended to enclosing pairs): [`match_bracket`],
/// else [`enclosing_brackets`]. Returns the opening and closing bracket
/// ranges.
///
/// Brackets come from `config.brackets` (Monaco's `()`, `[]`, `{}` if
/// empty). `is_skippable` tells whether the bracket starting at a position
/// is in a string or comment and should be ignored. At most about 20 000
/// characters are scanned in each direction.
pub fn matching_bracket(
    model: &impl CursorModel,
    config: &LanguageConfiguration,
    position: Position,
    is_skippable: &dyn Fn(Position) -> bool,
) -> Option<(Range<Position>, Range<Position>)> {
    match_or_enclosing(model, config, position, is_skippable, true)
        .map(|(open, close, _)| (open, close))
}

/// [`match_bracket`], else with `enclosing` [`enclosing_brackets`], and
/// whether the pair is next to `position` (the first). Scans the cursor's
/// row once for both.
pub(crate) fn match_or_enclosing(
    model: &impl CursorModel,
    config: &LanguageConfiguration,
    position: Position,
    is_skippable: &dyn Fn(Position) -> bool,
    enclosing: bool,
) -> Option<(Range<Position>, Range<Position>, bool)> {
    let model: &dyn CursorModel = model;
    if position.row >= model.line_count() {
        return None;
    }
    let brackets = Brackets::configured_or_default(config);
    let found = found_around(model, &brackets, position, is_skippable);
    if let Some((open, close)) = adjacent_pair(model, &brackets, position, &found, is_skippable) {
        return Some((open, close, true));
    }
    if !enclosing {
        return None;
    }
    let (open, close) = enclosing_pair(model, &brackets, position, found, is_skippable)?;
    Some((open, close, false))
}

/// The brackets of `position`'s row within a window around it.
fn found_around(
    model: &dyn CursorModel,
    brackets: &Brackets,
    position: Position,
    is_skippable: &dyn Fn(Position) -> bool,
) -> Vec<Found> {
    found_in(
        model,
        brackets,
        position.row,
        position.column.saturating_sub(WINDOW)..position.column.saturating_add(WINDOW),
        is_skippable,
    )
}

/// Monaco `matchBracket`: the pair of the bracket right after `position`,
/// else of the bracket right before it, as the opening and closing bracket
/// ranges. `None` when neither is a bracket with a match. Takes the same
/// arguments as [`matching_bracket`].
pub fn match_bracket(
    model: &impl CursorModel,
    config: &LanguageConfiguration,
    position: Position,
    is_skippable: &dyn Fn(Position) -> bool,
) -> Option<(Range<Position>, Range<Position>)> {
    match_or_enclosing(model, config, position, is_skippable, false)
        .map(|(open, close, _)| (open, close))
}

/// [`match_bracket`] with the brackets of `position`'s row ([`found_around`]).
fn adjacent_pair(
    model: &dyn CursorModel,
    brackets: &Brackets,
    position: Position,
    found: &[Found],
    is_skippable: &dyn Fn(Position) -> bool,
) -> Option<(Range<Position>, Range<Position>)> {
    let after = found.iter().find(|f| f.start == position.column);
    let before = found.iter().find(|f| f.start + f.len == position.column);
    for f in after.into_iter().chain(before) {
        let range = found_range(position.row, f);
        let pair = if f.is_open {
            find_close_forward(model, brackets, f.pair, range.end, is_skippable)
                .map(|close| (range, close))
        } else {
            find_open_backward(model, brackets, f.pair, range.start, is_skippable)
                .map(|open| (open, range))
        };
        if pair.is_some() {
            return pair;
        }
    }
    None
}

/// Monaco `findEnclosingBrackets`: the nearest pair around `position`, the
/// first opening bracket before it that isn't closed before it, with its
/// closing bracket. Takes the same arguments as [`matching_bracket`].
pub fn enclosing_brackets(
    model: &impl CursorModel,
    config: &LanguageConfiguration,
    position: Position,
    is_skippable: &dyn Fn(Position) -> bool,
) -> Option<(Range<Position>, Range<Position>)> {
    let model: &dyn CursorModel = model;
    if position.row >= model.line_count() {
        return None;
    }
    let brackets = Brackets::configured_or_default(config);
    let found = found_around(model, &brackets, position, is_skippable);
    enclosing_pair(model, &brackets, position, found, is_skippable)
}

/// [`enclosing_brackets`], given the brackets of `position`'s row
/// ([`found_around`]).
fn enclosing_pair(
    model: &dyn CursorModel,
    brackets: &Brackets,
    position: Position,
    row_found: Vec<Found>,
    is_skippable: &dyn Fn(Position) -> bool,
) -> Option<(Range<Position>, Range<Position>)> {
    let mut pending_closes: Vec<usize> = Vec::new();
    let mut scanned = 0usize;
    let mut row_found = Some(row_found);
    for row in (0..=position.row).rev() {
        let (found, len) = match row_found.take() {
            // The cursor's row, scanned up to the cursor.
            Some(found) => (found, position.column.min(WINDOW)),
            None => scan_row(model, brackets, row, Window::Tail, is_skippable),
        };
        for f in found.iter().rev() {
            if row == position.row && f.start + f.len > position.column {
                continue;
            }
            if !f.is_open {
                pending_closes.push(f.pair);
            } else if let Some(index) = pending_closes
                .iter()
                .rposition(|&p| brackets.close_text(p) == brackets.close_text(f.pair))
            {
                pending_closes.truncate(index);
            } else {
                let open = found_range(row, f);
                let close = find_close_forward(model, brackets, f.pair, open.end, is_skippable)?;
                return Some((open, close));
            }
        }
        scanned += len as usize + 1;
        if scanned > MAX_SCAN_UNITS {
            break;
        }
    }
    None
}

/// Monaco `findNextBracket`: the range of the first bracket ending after
/// `position`, skipping closing brackets with no opening one (Monaco's
/// bracket pair tree leaves those out). Takes the same arguments as
/// [`matching_bracket`]; scans at most about 20 000 characters.
pub fn next_bracket(
    model: &impl CursorModel,
    config: &LanguageConfiguration,
    position: Position,
    is_skippable: &dyn Fn(Position) -> bool,
) -> Option<Range<Position>> {
    let model: &dyn CursorModel = model;
    let brackets = Brackets::configured_or_default(config);
    let mut scanned = 0usize;
    for row in position.row..model.line_count() {
        let window = if row == position.row {
            position.column.saturating_sub(WINDOW)..position.column.saturating_add(WINDOW)
        } else {
            0..WINDOW
        };
        let (found, len) = scan_row(model, &brackets, row, Window::Columns(window), is_skippable);
        for f in found {
            if row == position.row && f.start + f.len <= position.column {
                continue;
            }
            let range = found_range(row, &f);
            if f.is_open
                || find_open_backward(model, &brackets, f.pair, range.start, is_skippable).is_some()
            {
                return Some(range);
            }
        }
        scanned += len as usize + 1;
        if scanned > MAX_SCAN_UNITS {
            break;
        }
    }
    None
}

/// Monaco `BracketSelectionRangeProvider` for one position: the inside and
/// outside of each bracket pair around `position`, innermost first, plus
/// the leading text before a multi-line pair's opening bracket. `skip`
/// tells whether the bracket starting at a position is in a string or
/// comment.
pub fn bracket_selection_ranges(
    model: &dyn CursorModel,
    config: &LanguageConfiguration,
    position: Position,
    skip: &dyn Fn(Position) -> bool,
) -> Vec<Range<Position>> {
    let brackets = Brackets::configured_or_default(config);
    let line_count = model.line_count();
    // Closing brackets with nothing open after `position`, by pair.
    let mut closings: Vec<std::collections::VecDeque<Range<Position>>> =
        vec![Default::default(); brackets.pairs.len()];
    let mut counts = vec![0i64; brackets.pairs.len()];
    let last_row = (position.row + MAX_BALANCE_LINES).min(line_count.saturating_sub(1));
    for row in position.row..=last_row {
        let line = line_units(model, row);
        for found in brackets.in_line(&line, &|i| skip(Position::new(row, i))) {
            if row == position.row && found.start < position.column {
                continue;
            }
            if found.is_open {
                counts[found.pair] += 1;
            } else {
                let value = counts[found.pair] - 1;
                counts[found.pair] = value.max(0);
                if value < 0 {
                    closings[found.pair].push_back(found_range(row, &found));
                }
            }
        }
    }
    let mut ranges = Vec::new();
    let mut counts = vec![0i64; brackets.pairs.len()];
    let first_row = position.row.saturating_sub(MAX_BALANCE_LINES);
    'rows: for row in (first_row..=position.row).rev() {
        if closings.iter().all(|c| c.is_empty()) {
            break;
        }
        let line = line_units(model, row);
        let found = brackets.in_line(&line, &|i| skip(Position::new(row, i)));
        for found in found.iter().rev() {
            if row == position.row && found.start + found.len > position.column {
                continue;
            }
            if !found.is_open {
                counts[found.pair] += 1;
                continue;
            }
            let value = counts[found.pair] - 1;
            counts[found.pair] = value.max(0);
            if value < 0
                && let Some(closing) = closings[found.pair].pop_front()
            {
                let open = found_range(row, found);
                let outer = open.start..closing.end;
                ranges.push(open.end..closing.start);
                ranges.push(outer.clone());
                add_bracket_leading(model, &outer, &mut ranges);
                if closings.iter().all(|c| c.is_empty()) {
                    break 'rows;
                }
            }
        }
    }
    ranges
}

/// Monaco `_addBracketLeading`: for a multi-line pair, the text before the
/// opening bracket on its line, or the line above when the bracket starts
/// its own line.
fn add_bracket_leading(
    model: &dyn CursorModel,
    bracket: &Range<Position>,
    ranges: &mut Vec<Range<Position>>,
) {
    if bracket.start.row == bracket.end.row {
        return;
    }
    let row = bracket.start.row;
    if let Some(column) = super::first_non_whitespace_column(model, row)
        && column != bracket.start.column
    {
        ranges.push(Position::new(row, column)..bracket.end);
        ranges.push(Position::new(row, 0)..bracket.end);
    }
    if row > 0 {
        let above = row - 1;
        if let Some(column) = super::first_non_whitespace_column(model, above)
            && column == bracket.start.column
        {
            ranges.push(Position::new(above, column)..bracket.end);
            ranges.push(Position::new(above, 0)..bracket.end);
        }
    }
}
