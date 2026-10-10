//! The sequences that get diffed: lines (port of Monaco's `lineSequence.ts`)
//! and the characters of a range of lines (`linesSliceCharSequence.ts`).
//!
//! Text is handled as UTF-16 code units throughout, exactly like Monaco's
//! JavaScript strings, so offsets and columns match Monaco.

use super::algorithms::Sequence;
use super::range::{
    OffsetRange, Position, Range, find_first_idx_monotonous_or_len, find_last_idx_monotonous,
};

/// Whether `c` is whitespace for JavaScript's `String.prototype.trim` and
/// regex `\s` (which differ from Rust's `char::is_whitespace`: U+FEFF is
/// included, U+0085 is not).
pub(crate) fn is_js_whitespace(c: u16) -> bool {
    matches!(
        c,
        0x09..=0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

/// JavaScript's `trimStart`.
pub(crate) fn trim_start(s: &[u16]) -> &[u16] {
    let start = s
        .iter()
        .position(|&c| !is_js_whitespace(c))
        .unwrap_or(s.len());
    &s[start..]
}

/// JavaScript's `trimEnd`.
pub(crate) fn trim_end(s: &[u16]) -> &[u16] {
    let end = s
        .iter()
        .rposition(|&c| !is_js_whitespace(c))
        .map_or(0, |i| i + 1);
    &s[..end]
}

/// JavaScript's `trim`.
pub(crate) fn trim(s: &[u16]) -> &[u16] {
    trim_end(trim_start(s))
}

/// Space or tab (Monaco's `isSpace` in the diff utils).
pub(crate) fn is_space(c: u16) -> bool {
    c == u16::from(b' ') || c == u16::from(b'\t')
}

/// The lines of a text as hashes of their trimmed content.
pub(crate) struct LineSequence<'a> {
    trimmed_hash: &'a [u32],
    lines: &'a [Vec<u16>],
}

impl<'a> LineSequence<'a> {
    pub(crate) fn new(trimmed_hash: &'a [u32], lines: &'a [Vec<u16>]) -> Self {
        Self {
            trimmed_hash,
            lines,
        }
    }

    /// The number of UTF-16 code units in `range`'s lines joined with `\n`,
    /// not counting whitespace.
    pub(crate) fn non_whitespace_len(&self, range: OffsetRange) -> usize {
        self.lines[range.start..range.end_exclusive]
            .iter()
            .map(|line| line.iter().filter(|&&c| !is_js_whitespace(c)).count())
            .sum()
    }
}

fn indentation(line: &[u16]) -> usize {
    line.iter().take_while(|&&c| is_space(c)).count()
}

impl Sequence for LineSequence<'_> {
    fn element(&self, offset: usize) -> u32 {
        self.trimmed_hash[offset]
    }

    fn len(&self) -> usize {
        self.trimmed_hash.len()
    }

    fn boundary_score(&self, length: usize) -> i64 {
        let before = if length == 0 {
            0
        } else {
            indentation(&self.lines[length - 1])
        };
        let after = if length == self.lines.len() {
            0
        } else {
            indentation(&self.lines[length])
        };
        1000 - (before + after) as i64
    }

    fn is_strongly_equal(&self, offset1: usize, offset2: usize) -> bool {
        self.lines[offset1] == self.lines[offset2]
    }
}

/// The characters (UTF-16 code units) of a range of lines, joined with `\n`;
/// optionally with each line's leading and trailing whitespace removed.
pub(crate) struct LinesSliceCharSequence {
    start_line_number: usize,
    elements: Vec<u16>,
    first_element_offset_by_line_idx: Vec<usize>,
    line_start_offsets: Vec<usize>,
    trimmed_ws_lengths_by_line_idx: Vec<usize>,
}

impl LinesSliceCharSequence {
    pub(crate) fn new(lines: &[Vec<u16>], range: Range, consider_whitespace_changes: bool) -> Self {
        let mut this = Self {
            start_line_number: range.start_line_number,
            elements: Vec::new(),
            first_element_offset_by_line_idx: vec![0],
            line_start_offsets: Vec::new(),
            trimmed_ws_lengths_by_line_idx: Vec::new(),
        };
        for line_number in range.start_line_number..=range.end_line_number {
            let mut line: &[u16] = &lines[line_number - 1];
            let mut line_start_offset = 0;
            if line_number == range.start_line_number && range.start_column > 1 {
                line_start_offset = range.start_column - 1;
                line = &line[line_start_offset.min(line.len())..];
            }
            this.line_start_offsets.push(line_start_offset);

            let mut trimmed_ws_length = 0;
            if !consider_whitespace_changes {
                let trimmed_start = trim_start(line);
                trimmed_ws_length = line.len() - trimmed_start.len();
                line = trim_end(trimmed_start);
            }
            this.trimmed_ws_lengths_by_line_idx.push(trimmed_ws_length);

            let line_length = if line_number == range.end_line_number {
                let available = range.end_column as i128
                    - 1
                    - line_start_offset as i128
                    - trimmed_ws_length as i128;
                available.clamp(0, line.len() as i128) as usize
            } else {
                line.len()
            };
            this.elements.extend_from_slice(&line[..line_length]);

            if line_number < range.end_line_number {
                this.elements.push(u16::from(b'\n'));
                this.first_element_offset_by_line_idx
                    .push(this.elements.len());
            }
        }
        this
    }

    /// The code units in `range`.
    pub(crate) fn text(&self, range: OffsetRange) -> &[u16] {
        &self.elements[range.start..range.end_exclusive]
    }

    /// The text position of an offset. With `prefer_left`, an offset at the
    /// start of a trimmed line maps before the trimmed whitespace.
    pub(crate) fn translate_offset(&self, offset: usize, prefer_left: bool) -> Position {
        let i =
            find_last_idx_monotonous(&self.first_element_offset_by_line_idx, 0, |&v| v <= offset)
                .max(0) as usize;
        let line_offset = offset - self.first_element_offset_by_line_idx[i];
        let trimmed = if line_offset == 0 && prefer_left {
            0
        } else {
            self.trimmed_ws_lengths_by_line_idx[i]
        };
        Position::new(
            self.start_line_number + i,
            1 + self.line_start_offsets[i] + line_offset + trimmed,
        )
    }

    pub(crate) fn translate_range(&self, range: OffsetRange) -> Range {
        let pos1 = self.translate_offset(range.start, false);
        let pos2 = self.translate_offset(range.end_exclusive, true);
        if pos2.is_before(pos1) {
            return Range::from_positions(pos2, pos2);
        }
        Range::from_positions(pos1, pos2)
    }

    /// The word (`[a-zA-Z0-9]+`) containing the character at `offset`.
    pub(crate) fn find_word_containing(&self, offset: usize) -> Option<OffsetRange> {
        if offset >= self.elements.len() || !is_word_char(self.elements[offset]) {
            return None;
        }
        let mut start = offset;
        while start > 0 && is_word_char(self.elements[start - 1]) {
            start -= 1;
        }
        let mut end = offset;
        while end < self.elements.len() && is_word_char(self.elements[end]) {
            end += 1;
        }
        Some(OffsetRange::new(start, end))
    }

    /// The sub-word containing the character at `offset` (`fooBar` has the
    /// sub-words `foo` and `Bar`).
    pub(crate) fn find_sub_word_containing(&self, offset: usize) -> Option<OffsetRange> {
        if offset >= self.elements.len() || !is_word_char(self.elements[offset]) {
            return None;
        }
        let mut start = offset;
        while start > 0
            && is_word_char(self.elements[start - 1])
            && !is_upper_case(self.elements[start])
        {
            start -= 1;
        }
        let mut end = offset;
        while end < self.elements.len()
            && is_word_char(self.elements[end])
            && !is_upper_case(self.elements[end])
        {
            end += 1;
        }
        Some(OffsetRange::new(start, end))
    }

    pub(crate) fn count_lines_in(&self, range: OffsetRange) -> usize {
        self.translate_offset(range.end_exclusive, false)
            .line_number
            - self.translate_offset(range.start, false).line_number
    }

    /// Extends `range` to the starts of its first line and the line after it.
    pub(crate) fn extend_to_full_lines(&self, range: OffsetRange) -> OffsetRange {
        let offsets = &self.first_element_offset_by_line_idx;
        let start = usize::try_from(find_last_idx_monotonous(offsets, 0, |&x| x <= range.start))
            .map_or(0, |i| offsets[i]);
        let end_idx = find_first_idx_monotonous_or_len(offsets, |&x| range.end_exclusive <= x);
        let end = offsets.get(end_idx).copied().unwrap_or(self.elements.len());
        OffsetRange::new(start, end)
    }
}

impl Sequence for LinesSliceCharSequence {
    fn element(&self, offset: usize) -> u32 {
        u32::from(self.elements[offset])
    }

    fn len(&self) -> usize {
        self.elements.len()
    }

    fn boundary_score(&self, length: usize) -> i64 {
        //   a   b   c   ,           d   e   f
        // 11  0   0   12  15  6   13  0   0   11
        let prev = category(length.checked_sub(1).map(|i| self.elements[i]));
        let next = category(self.elements.get(length).copied());

        if prev == Category::LineBreakCr && next == Category::LineBreakLf {
            // Don't break between \r and \n.
            return 0;
        }
        if prev == Category::LineBreakLf {
            // Prefer the line break before the change.
            return 150;
        }

        let mut score = 0;
        if prev != next {
            score += 10;
            if prev == Category::WordLower && next == Category::WordUpper {
                score += 1;
            }
        }
        score + prev.score() + next.score()
    }

    fn is_strongly_equal(&self, offset1: usize, offset2: usize) -> bool {
        self.elements[offset1] == self.elements[offset2]
    }
}

fn is_word_char(c: u16) -> bool {
    u8::try_from(c).is_ok_and(|c| c.is_ascii_alphanumeric())
}

fn is_upper_case(c: u16) -> bool {
    u8::try_from(c).is_ok_and(|c| c.is_ascii_uppercase())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    WordLower,
    WordUpper,
    WordNumber,
    End,
    Other,
    Separator,
    Space,
    LineBreakCr,
    LineBreakLf,
}

impl Category {
    fn score(self) -> i64 {
        match self {
            Self::WordLower | Self::WordUpper | Self::WordNumber => 0,
            Self::End | Self::LineBreakCr | Self::LineBreakLf => 10,
            Self::Other => 2,
            Self::Separator => 30,
            Self::Space => 3,
        }
    }
}

/// The boundary category of a code unit (`None` is the sequence end).
fn category(c: Option<u16>) -> Category {
    let Some(c) = c else {
        return Category::End;
    };
    if is_space(c) {
        return Category::Space;
    }
    match u8::try_from(c) {
        Ok(b'\n') => Category::LineBreakLf,
        Ok(b'\r') => Category::LineBreakCr,
        Ok(b'a'..=b'z') => Category::WordLower,
        Ok(b'A'..=b'Z') => Category::WordUpper,
        Ok(b'0'..=b'9') => Category::WordNumber,
        Ok(b',' | b';') => Category::Separator,
        _ => Category::Other,
    }
}
