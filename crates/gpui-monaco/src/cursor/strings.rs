//! String helpers for cursor logic, measured in UTF-16 code units like Monaco.
//!
//! Ports parts of Monaco's `src/vs/base/common/strings.ts` (grapheme helpers,
//! `isFullWidthCharacter`, `isEmojiImprecise`, `getLeftDeleteOffset`,
//! whitespace helpers), `src/vs/editor/common/core/cursorColumns.ts`,
//! `src/vs/editor/common/core/misc/indentation.ts` and
//! `src/vs/editor/common/cursor/cursorAtomicMoveOperations.ts` (MIT).
//!
//! Grapheme clusters come from `unicode-segmentation` (full UAX #29) instead
//! of Monaco's simplified pairwise break table. The two agree on everything
//! Monaco's tests exercise; they differ on e.g. runs of regional indicators,
//! which Monaco never breaks.

use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub(crate) fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// A grapheme cluster of a line: its UTF-16 start offset, UTF-16 length and
/// first code point.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Grapheme {
    pub start: u32,
    pub len: u32,
    pub first: u32,
}

pub(crate) fn graphemes(line: &str) -> impl Iterator<Item = Grapheme> + '_ {
    let mut offset = 0u32;
    line.graphemes(true).map(move |g| {
        let len = utf16_len(g);
        let start = offset;
        offset += len;
        Grapheme {
            start,
            len,
            first: g.chars().next().map_or(0, |c| c as u32),
        }
    })
}

/// Monaco `strings.nextCharLength`: UTF-16 length of the grapheme starting at
/// (or containing) `offset`.
pub(crate) fn next_char_length(line: &str, offset: u32) -> u32 {
    for g in graphemes(line) {
        if g.start + g.len > offset {
            return g.start + g.len - offset;
        }
    }
    0
}

/// Monaco `strings.prevCharLength`: UTF-16 length of the grapheme ending at
/// (or containing) `offset`.
pub(crate) fn prev_char_length(line: &str, offset: u32) -> u32 {
    let mut last_start = None;
    for g in graphemes(line) {
        if g.start >= offset {
            break;
        }
        last_start = Some(g.start);
    }
    last_start.map_or(0, |start| offset - start)
}

/// Monaco `strings.isFullWidthCharacter`.
pub(crate) fn is_full_width_character(code_point: u32) -> bool {
    (0x2E80..=0xD7AF).contains(&code_point)
        || (0xF900..=0xFAFF).contains(&code_point)
        || (0xFF01..=0xFF5E).contains(&code_point)
        || (0xFFE0..=0xFFE6).contains(&code_point)
}

/// Monaco `strings.isEmojiImprecise`.
pub(crate) fn is_emoji_imprecise(x: u32) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&x)
        || x == 8986
        || x == 8987
        || x == 9200
        || x == 9203
        || (9728..=10175).contains(&x)
        || x == 11088
        || x == 11093
        || (127744..=128591).contains(&x)
        || (128640..=128764).contains(&x)
        || (128992..=129008).contains(&x)
        || (129280..=129535).contains(&x)
        || (129648..=129782).contains(&x)
}

fn is_whitespace_unit(c: u16) -> bool {
    c == b' ' as u16 || c == b'\t' as u16
}

/// Monaco `strings.firstNonWhitespaceIndex` (only space and tab count).
pub(crate) fn first_non_whitespace_index(line: &[u16]) -> Option<usize> {
    line.iter().position(|&c| !is_whitespace_unit(c))
}

/// Monaco `strings.lastNonWhitespaceIndex`, searching from `start_index`
/// (inclusive) backwards.
pub(crate) fn last_non_whitespace_index(line: &[u16], start_index: Option<usize>) -> Option<usize> {
    let end = match start_index {
        Some(i) => (i + 1).min(line.len()),
        None => line.len(),
    };
    line[..end].iter().rposition(|&c| !is_whitespace_unit(c))
}

/// Monaco `strings.getLeadingWhitespace`.
pub(crate) fn leading_whitespace(line: &str) -> &str {
    let end = line.find(|c| c != ' ' && c != '\t').unwrap_or(line.len());
    &line[..end]
}

/// UTF-16 length of the leading whitespace (Monaco `indentOfLine`).
pub(crate) fn indent_len(line: &str) -> u32 {
    leading_whitespace(line).len() as u32
}

/// The prefix of `s` that is `units` UTF-16 code units long (clamped).
pub(crate) fn utf16_prefix(s: &str, units: u32) -> &str {
    let mut count = 0u32;
    for (i, c) in s.char_indices() {
        if count >= units {
            return &s[..i];
        }
        count += c.len_utf16() as u32;
    }
    s
}

/// The substring of `s` between two UTF-16 offsets (clamped).
pub(crate) fn utf16_slice(s: &str, start: u32, end: u32) -> String {
    let units = utf16(s);
    let end = (end as usize).min(units.len());
    let start = (start as usize).min(end);
    String::from_utf16_lossy(&units[start..end])
}

// ---------------------------------------------------------------------------
// cursorColumns.ts

fn next_visible_column(code_point: u32, visible_column: u32, tab_size: u32) -> u32 {
    if code_point == '\t' as u32 {
        return next_render_tab_stop(visible_column, tab_size);
    }
    if is_full_width_character(code_point) || is_emoji_imprecise(code_point) {
        return visible_column + 2;
    }
    visible_column + 1
}

/// Monaco `CursorColumns.visibleColumnFromColumn`, with a 0-based `column`.
pub(crate) fn visible_column_from_column(line: &str, column: u32, tab_size: u32) -> u32 {
    let mut result = 0;
    for g in graphemes(line) {
        if g.start >= column {
            break;
        }
        result = next_visible_column(g.first, result, tab_size);
    }
    result
}

/// Monaco `CursorColumns.toStatusbarColumn`, with a 0-based `column`: the
/// 1-based column a status bar shows, counting a tab to the next tab stop
/// and anything else, a surrogate pair included, as one.
pub(crate) fn to_statusbar_column(line: &str, column: u32, tab_size: u32) -> u32 {
    let mut result = 0;
    let mut offset = 0;
    for ch in line.chars() {
        if offset >= column {
            break;
        }
        // Half of a surrogate pair counts as one code point too.
        offset += ch.len_utf16() as u32;
        result = if ch == '\t' {
            next_render_tab_stop(result, tab_size)
        } else {
            result + 1
        };
    }
    result + 1
}

/// Monaco `CursorColumns.columnFromVisibleColumn`, returning a 0-based column.
pub(crate) fn column_from_visible_column(line: &str, visible_column: i64, tab_size: u32) -> u32 {
    if visible_column <= 0 {
        return 0;
    }
    let mut before_visible_column: i64 = 0;
    let mut before_column = 0;
    for g in graphemes(line) {
        let after_visible_column =
            next_visible_column(g.first, before_visible_column as u32, tab_size) as i64;
        let after_column = g.start + g.len;
        if after_visible_column >= visible_column {
            let before_delta = visible_column - before_visible_column;
            let after_delta = after_visible_column - visible_column;
            return if after_delta < before_delta {
                after_column
            } else {
                before_column
            };
        }
        before_visible_column = after_visible_column;
        before_column = after_column;
    }
    utf16_len(line)
}

/// Monaco `CursorColumns.nextRenderTabStop` (0-based visible columns).
pub(crate) fn next_render_tab_stop(visible_column: u32, tab_size: u32) -> u32 {
    visible_column + tab_size - visible_column % tab_size
}

/// Monaco `CursorColumns.nextIndentTabStop`.
pub(crate) fn next_indent_tab_stop(visible_column: u32, indent_size: u32) -> u32 {
    next_render_tab_stop(visible_column, indent_size)
}

/// Monaco `CursorColumns.prevRenderTabStop`.
pub(crate) fn prev_render_tab_stop(column: u32, tab_size: u32) -> u32 {
    if column == 0 {
        return 0;
    }
    (column - 1) - (column - 1) % tab_size
}

/// Monaco `CursorColumns.prevIndentTabStop`.
pub(crate) fn prev_indent_tab_stop(column: u32, indent_size: u32) -> u32 {
    prev_render_tab_stop(column, indent_size)
}

// ---------------------------------------------------------------------------
// indentation.ts

/// Monaco `normalizeIndentation`.
pub(crate) fn normalize_indentation(s: &str, indent_size: u32, insert_spaces: bool) -> String {
    let ws = leading_whitespace(s);
    let mut spaces_cnt = 0;
    for c in ws.chars() {
        if c == '\t' {
            spaces_cnt = next_indent_tab_stop(spaces_cnt, indent_size);
        } else {
            spaces_cnt += 1;
        }
    }
    let mut result = String::new();
    if !insert_spaces {
        let tabs_cnt = spaces_cnt / indent_size;
        spaces_cnt %= indent_size;
        for _ in 0..tabs_cnt {
            result.push('\t');
        }
    }
    for _ in 0..spaces_cnt {
        result.push(' ');
    }
    result.push_str(&s[ws.len()..]);
    result
}

// ---------------------------------------------------------------------------
// strings.getLeftDeleteOffset

const ZWJ: u32 = 0x200D;
const EMOJI_VARIANT_SELECTOR: u32 = 0xFE0F;
const ENCLOSING_KEY_CAP: u32 = 0x20E3;

fn is_emoji_modifier(code_point: u32) -> bool {
    (0x1F3FB..=0x1F3FF).contains(&code_point)
}

fn is_high_surrogate(c: u16) -> bool {
    (0xD800..=0xDBFF).contains(&c)
}

fn is_low_surrogate(c: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&c)
}

/// Monaco `CodePointIterator.prevCodePoint`: returns the code point before
/// `*offset` and moves `*offset` back over it.
fn prev_code_point(units: &[u16], offset: &mut usize) -> u32 {
    let c = units[*offset - 1];
    if is_low_surrogate(c) && *offset > 1 {
        let h = units[*offset - 2];
        if is_high_surrogate(h) {
            *offset -= 2;
            return 0x10000 + (((h as u32) - 0xD800) << 10) + ((c as u32) - 0xDC00);
        }
    }
    *offset -= 1;
    c as u32
}

/// Monaco `strings.getLeftDeleteOffset`: the offset after a left delete at
/// `offset`, deleting whole emoji (with modifiers / ZWJ) or one code point.
pub(crate) fn left_delete_offset(offset: u32, line: &str) -> u32 {
    if offset == 0 {
        return 0;
    }
    let units = utf16(line);
    if let Some(o) = offset_before_last_emoji_component(offset as usize, &units) {
        return o as u32;
    }
    let mut o = offset as usize;
    prev_code_point(&units, &mut o);
    o as u32
}

fn offset_before_last_emoji_component(initial_offset: usize, units: &[u16]) -> Option<usize> {
    let mut offset = initial_offset;
    let mut code_point = prev_code_point(units, &mut offset);
    while is_emoji_modifier(code_point)
        || code_point == EMOJI_VARIANT_SELECTOR
        || code_point == ENCLOSING_KEY_CAP
    {
        if offset == 0 {
            return None;
        }
        code_point = prev_code_point(units, &mut offset);
    }
    if !is_emoji_imprecise(code_point) {
        return None;
    }
    let mut result = offset;
    if result > 0 {
        let optional_zwj = prev_code_point(units, &mut offset);
        if optional_zwj == ZWJ {
            result = offset;
        }
    }
    Some(result)
}

/// True if the UTF-16 unit before `column` is a high surrogate, i.e. `column`
/// splits a surrogate pair.
pub(crate) fn splits_surrogate_pair(line: &str, column: u32) -> bool {
    if column == 0 {
        return false;
    }
    let units = utf16(line);
    (column as usize) <= units.len() && is_high_surrogate(units[column as usize - 1])
}

// ---------------------------------------------------------------------------
// cursorAtomicMoveOperations.ts

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomicDirection {
    Left,
    Right,
    #[allow(dead_code)]
    Nearest,
}

/// Monaco `AtomicTabMoveOperations.whitespaceVisibleColumn`. Returns
/// `(prevTabStopPosition, prevTabStopVisibleColumn, visibleColumn)`, with -1
/// meaning "not applicable".
fn whitespace_visible_column(line: &[u16], position: usize, tab_size: u32) -> (i64, i64, i64) {
    let mut visible_column: i64 = 0;
    let mut prev_tab_stop_position: i64 = -1;
    let mut prev_tab_stop_visible_column: i64 = -1;
    for (i, &ch) in line.iter().enumerate() {
        if i == position {
            return (
                prev_tab_stop_position,
                prev_tab_stop_visible_column,
                visible_column,
            );
        }
        if visible_column % tab_size as i64 == 0 {
            prev_tab_stop_position = i as i64;
            prev_tab_stop_visible_column = visible_column;
        }
        match ch {
            0x20 => visible_column += 1,
            0x09 => visible_column = next_render_tab_stop(visible_column as u32, tab_size) as i64,
            _ => return (-1, -1, -1),
        }
    }
    if position == line.len() {
        return (
            prev_tab_stop_position,
            prev_tab_stop_visible_column,
            visible_column,
        );
    }
    (-1, -1, -1)
}

/// Monaco `AtomicTabMoveOperations.atomicPosition` (0-based in and out).
pub(crate) fn atomic_position(
    line: &[u16],
    position: usize,
    tab_size: u32,
    direction: AtomicDirection,
) -> Option<usize> {
    let tab = tab_size as i64;
    let (prev_tab_stop_position, prev_tab_stop_visible_column, visible_column) =
        whitespace_visible_column(line, position, tab_size);
    if visible_column == -1 {
        return None;
    }
    let left = match direction {
        AtomicDirection::Left => true,
        AtomicDirection::Right => false,
        AtomicDirection::Nearest => {
            if visible_column % tab == 0 {
                return Some(position);
            }
            (visible_column % tab) as f64 <= tab as f64 / 2.0
        }
    };
    let step = |current: i64, ch: u16| -> Option<i64> {
        match ch {
            0x20 => Some(current + 1),
            0x09 => Some(next_render_tab_stop(current as u32, tab_size) as i64),
            _ => None,
        }
    };
    if left {
        if prev_tab_stop_position == -1 {
            return None;
        }
        let mut current = prev_tab_stop_visible_column;
        for &ch in &line[prev_tab_stop_position as usize..] {
            if current == prev_tab_stop_visible_column + tab {
                return Some(prev_tab_stop_position as usize);
            }
            current = step(current, ch)?;
        }
        if current == prev_tab_stop_visible_column + tab {
            return Some(prev_tab_stop_position as usize);
        }
        return None;
    }
    let target = next_render_tab_stop(visible_column as u32, tab_size) as i64;
    let mut current = visible_column;
    for (i, &ch) in line.iter().enumerate().skip(position) {
        if current == target {
            return Some(i);
        }
        current = step(current, ch)?;
    }
    if current == target {
        return Some(line.len());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_columns() {
        // |<TAB>|<TAB>|T|ext with tab size 4
        let line = "\t\tText";
        assert_eq!(visible_column_from_column(line, 0, 4), 0);
        assert_eq!(visible_column_from_column(line, 1, 4), 4);
        assert_eq!(visible_column_from_column(line, 2, 4), 8);
        assert_eq!(visible_column_from_column(line, 3, 4), 9);
        assert_eq!(column_from_visible_column(line, 9, 4), 3);
        assert_eq!(column_from_visible_column(line, 5, 4), 1);
        assert_eq!(column_from_visible_column(line, 7, 4), 2);
    }

    /// Monaco `cursorColumns.test.ts` "toStatusbarColumn" (1-based columns
    /// there, 0-based here).
    #[test]
    fn statusbar_columns() {
        let t = |text: &str, tab_size: u32, column: u32, expected: u32| {
            assert_eq!(
                to_statusbar_column(text, column - 1, tab_size),
                expected,
                "{text:?} {tab_size} {column}"
            );
        };
        for column in 1..=11 {
            t("    spaces", 4, column, column);
        }
        t("\ttab", 4, 1, 1);
        t("\ttab", 4, 2, 5);
        t("\ttab", 4, 3, 6);
        t("\ttab", 4, 4, 7);
        t("\ttab", 4, 5, 8);
        let text = "𐌀𐌁𐌂𐌃𐌄𐌅𐌆";
        t(text, 4, 1, 1);
        t(text, 4, 2, 2);
        t(text, 4, 3, 2);
        t(text, 4, 4, 3);
        t(text, 4, 5, 3);
        t(text, 4, 6, 4);
        t(text, 4, 7, 4);
        t(text, 4, 8, 5);
        t(text, 4, 9, 5);
        t(text, 4, 10, 6);
        t(text, 4, 11, 6);
        t(text, 4, 12, 7);
        t(text, 4, 13, 7);
        t(text, 4, 14, 8);
        t(text, 4, 15, 8);
        // Past the end, as Monaco clamps.
        t("ab", 4, 10, 3);
    }

    #[test]
    fn graphemes_and_surrogates() {
        let line = "a🐶b";
        assert_eq!(next_char_length(line, 1), 2);
        assert_eq!(prev_char_length(line, 3), 2);
        assert!(splits_surrogate_pair(line, 2));
        assert!(!splits_surrogate_pair(line, 3));
        assert_eq!(visible_column_from_column(line, 3, 4), 3);
    }

    #[test]
    fn left_delete() {
        assert_eq!(left_delete_offset(2, "👍🏻"), 0);
        assert_eq!(left_delete_offset(4, "👍🏻"), 0);
        assert_eq!(left_delete_offset(1, "ab"), 0);
    }
}
