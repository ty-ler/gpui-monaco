//! Bracket matching over the text, skipping brackets in strings and
//! comments.
//!
//! Ports the bracket queries the cursor logic needs from Monaco's
//! `src/vs/editor/common/model/bracketPairsTextModelPart/bracketPairsImpl.ts`
//! (`matchBracket`, `findMatchingBracketUp`, the fallback
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
            .brackets
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
        let line = line_units(model, r);
        scanned += line.len() + 1;
        let found = brackets.in_line(&line, &|i| skip(Position::new(r, i)));
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
        let line = line_units(model, row);
        for f in brackets.in_line(&line, &|i| skip(Position::new(row, i))) {
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
        scanned += line.len() + 1;
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
        let line = line_units(model, row);
        for f in brackets
            .in_line(&line, &|i| skip(Position::new(row, i)))
            .iter()
            .rev()
        {
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
        scanned += line.len() + 1;
        if scanned > MAX_SCAN_UNITS {
            break;
        }
    }
    None
}

/// The bracket pair to highlight or jump between for a cursor at `position`
/// (Monaco `matchBracket`, extended to enclosing pairs): the pair of the
/// bracket right after the cursor, else of the bracket right before it, else
/// the nearest pair enclosing the cursor. Returns the opening and closing
/// bracket ranges.
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
    let model: &dyn CursorModel = model;
    if position.row >= model.line_count() {
        return None;
    }
    let brackets = Brackets::configured_or_default(config);
    let line = line_units(model, position.row);
    let found = brackets.in_line(&line, &|i| is_skippable(Position::new(position.row, i)));

    let after = found.iter().find(|f| f.start == position.column);
    let before = found.iter().find(|f| f.start + f.len == position.column);
    for f in after.into_iter().chain(before) {
        let range = found_range(position.row, f);
        if f.is_open {
            if let Some(close) =
                find_close_forward(model, &brackets, f.pair, range.end, is_skippable)
            {
                return Some((range, close));
            }
        } else if let Some(open) =
            find_open_backward(model, &brackets, f.pair, range.start, is_skippable)
        {
            return Some((open, range));
        }
    }

    // The nearest enclosing pair: the first opening bracket before the cursor
    // that is not closed before it.
    let mut pending_closes: Vec<usize> = Vec::new();
    let mut scanned = 0usize;
    for row in (0..=position.row).rev() {
        let line = line_units(model, row);
        for f in brackets
            .in_line(&line, &|i| is_skippable(Position::new(row, i)))
            .iter()
            .rev()
        {
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
                let close = find_close_forward(model, &brackets, f.pair, open.end, is_skippable)?;
                return Some((open, close));
            }
        }
        scanned += line.len() + 1;
        if scanned > MAX_SCAN_UNITS {
            break;
        }
    }
    None
}
