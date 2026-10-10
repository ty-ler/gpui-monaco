//! Sparse semantic token storage, ported from Monaco (MIT, Copyright (c) Microsoft
//! Corporation):
//!
//! - `vs/editor/common/tokens/sparseMultilineTokens.ts` (`SparseMultilineTokens`,
//!   `SparseMultilineTokensStorage`) -> [`SparseMultilineTokens`];
//! - `vs/editor/common/tokens/sparseTokensStore.ts` (`SparseTokensStore`) ->
//!   [`SparseTokensStore`];
//! - `vs/editor/common/services/semanticTokensProviderStyling.ts`
//!   (`toMultilineTokens2`) -> [`to_multiline_tokens`];
//! - `vs/editor/common/core/misc/eolCounter.ts` (`countEOL`) -> [`count_eol`].
//!
//! Each token is stored, like in Monaco, as 4 `u32`s:
//! `[deltaLine, startCharacter, endCharacter, metadata]`, where `deltaLine` is
//! relative to the start line of the area holding the token.
//!
//! Deviations from Monaco:
//!
//! - Lines are 0-based (Monaco's 1-based line numbers minus 1) and columns are
//!   0-based UTF-16 code units (Monaco's 1-based columns minus 1). Ranges are
//!   `Range<text::PointUtf16>`.
//! - Line bookkeeping (`start_line`/`end_line`) is signed (`i64`) because, as in
//!   Monaco, an empty area has `end_line == start_line - 1`, and bogus provider
//!   data can produce lines before the document start. Monaco's "no line yet"
//!   sentinel `0` (an invalid 1-based line) becomes `-1` here.
//! - The metadata word is an opaque style id chosen by the caller. Monaco's
//!   `addSparseTokens` (which merges semantic metadata bits into TextMate line
//!   tokens) is not ported; [`SparseTokensStore::line_tokens`] exposes the
//!   semantic tokens of a line instead (consulting only the first piece that
//!   contains the line, like `addSparseTokens`), and the editor merges styles.
//! - The styling callback of [`to_multiline_tokens`] replaces
//!   `SemanticTokensProviderStyling.getMetadata`; `None` means `NO_STYLING`.
//!   Invalid/overlapping-token warnings are not logged.
//! - `reportIfInvalid`, `toString` and the `RateLimiter` are not ported.
//! - Instead of a fixed `Uint32Array` with a separate token count, the storage
//!   is a `Vec<u32>` that is truncated. This also means a cleared storage can
//!   never return stale tokens from `line_tokens` (Monaco's `getLineTokens`
//!   reads index 0 even when the count is 0).

use std::ops::Range;

use text::PointUtf16;

/// Monaco's `SemanticColoringConstants.DesiredTokensPerArea`.
const DESIRED_TOKENS_PER_AREA: usize = 400;
/// Monaco's `SemanticColoringConstants.DesiredMaxAreas`.
const DESIRED_MAX_AREAS: usize = 1024;

/// A semantic token on a single line: `(start, end, style)`, with `start`/`end`
/// in UTF-16 code units from the line start.
pub(crate) type SparseLineToken = (u32, u32, u32);

/// Counts the line breaks in `text` (`\n`, `\r\n` and lone `\r` each count as
/// one). Returns `(eol_count, first_line_length, last_line_length)` in UTF-16
/// code units. Port of Monaco's `countEOL` (without the EOL kind).
pub(crate) fn count_eol(text: &str) -> (u32, u32, u32) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut eol_count = 0u32;
    let mut first_line_length = 0usize;
    let mut last_line_start = 0usize;
    let mut i = 0;
    while i < units.len() {
        let chr = units[i];
        if chr == u16::from(b'\r') {
            if eol_count == 0 {
                first_line_length = i;
            }
            eol_count += 1;
            if i + 1 < units.len() && units[i + 1] == u16::from(b'\n') {
                i += 1;
            }
            last_line_start = i + 1;
        } else if chr == u16::from(b'\n') {
            if eol_count == 0 {
                first_line_length = i;
            }
            eol_count += 1;
            last_line_start = i + 1;
        }
        i += 1;
    }
    if eol_count == 0 {
        first_line_length = units.len();
    }
    (
        eol_count,
        first_line_length as u32,
        (units.len() - last_line_start) as u32,
    )
}

/// A range with signed, 0-based line and column coordinates, used internally so
/// that line deltas relative to an area may go negative like in Monaco.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineRange {
    start_line: i64,
    start_col: i64,
    end_line: i64,
    end_col: i64,
}

impl LineRange {
    /// Builds a range, swapping the ends if needed (like Monaco's `Range`
    /// constructor).
    fn new(start_line: i64, start_col: i64, end_line: i64, end_col: i64) -> Self {
        if start_line > end_line || (start_line == end_line && start_col > end_col) {
            Self {
                start_line: end_line,
                start_col: end_col,
                end_line: start_line,
                end_col: start_col,
            }
        } else {
            Self {
                start_line,
                start_col,
                end_line,
                end_col,
            }
        }
    }

    fn from_points(range: &Range<PointUtf16>) -> Self {
        Self::new(
            i64::from(range.start.row),
            i64::from(range.start.column),
            i64::from(range.end.row),
            i64::from(range.end.column),
        )
    }

    fn to_points(self) -> Range<PointUtf16> {
        let clamp = |v: i64| v.clamp(0, i64::from(u32::MAX)) as u32;
        PointUtf16::new(clamp(self.start_line), clamp(self.start_col))
            ..PointUtf16::new(clamp(self.end_line), clamp(self.end_col))
    }

    /// Port of Monaco's `Range.plusRange`: the smallest range containing both.
    fn plus(self, b: Self) -> Self {
        let a = self;
        let (start_line, start_col) = if b.start_line < a.start_line {
            (b.start_line, b.start_col)
        } else if b.start_line == a.start_line {
            (b.start_line, b.start_col.min(a.start_col))
        } else {
            (a.start_line, a.start_col)
        };
        let (end_line, end_col) = if b.end_line > a.end_line {
            (b.end_line, b.end_col)
        } else if b.end_line == a.end_line {
            (b.end_line, b.end_col.max(a.end_col))
        } else {
            (a.end_line, a.end_col)
        };
        Self::new(start_line, start_col, end_line, end_col)
    }
}

/// Whether `code` is an ASCII digit or letter (Monaco's "word character" test
/// in `acceptInsertText`).
fn is_word_char_code(code: u16) -> bool {
    matches!(code, 0x30..=0x39 | 0x41..=0x5A | 0x61..=0x7A)
}

/// Port of Monaco's `SparseMultilineTokensStorage`: a flat list of tokens, 4
/// `u32`s each (`[deltaLine, startCharacter, endCharacter, metadata]`), sorted
/// by position. Values are read as `i64` for arithmetic and written back modulo
/// 2^32, mirroring Monaco's `Uint32Array`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SparseMultilineTokensStorage {
    tokens: Vec<u32>,
}

impl SparseMultilineTokensStorage {
    fn new(mut tokens: Vec<u32>) -> Self {
        debug_assert!(tokens.len().is_multiple_of(4), "tokens come in groups of 4");
        tokens.truncate(tokens.len() / 4 * 4);
        Self { tokens }
    }

    fn token_count(&self) -> usize {
        self.tokens.len() / 4
    }

    fn delta_line(&self, i: usize) -> i64 {
        i64::from(self.tokens[4 * i])
    }

    fn start_character(&self, i: usize) -> i64 {
        i64::from(self.tokens[4 * i + 1])
    }

    fn end_character(&self, i: usize) -> i64 {
        i64::from(self.tokens[4 * i + 2])
    }

    /// The delta line of the last token, or `-1` when empty.
    fn max_delta_line(&self) -> i64 {
        match self.token_count() {
            0 => -1,
            n => self.delta_line(n - 1),
        }
    }

    /// The range from the first token's start to the last token's end, in
    /// delta-line coordinates.
    fn range(&self) -> Option<LineRange> {
        let n = self.token_count();
        if n == 0 {
            return None;
        }
        Some(LineRange::new(
            0,
            self.start_character(0),
            self.delta_line(n - 1),
            self.end_character(n - 1),
        ))
    }

    fn is_empty(&self) -> bool {
        self.token_count() == 0
    }

    /// Binary search for the tokens on `delta_line`.
    fn line_tokens(&self, delta_line: i64) -> Option<Vec<SparseLineToken>> {
        if self.is_empty() {
            return None;
        }
        let collect = |min: usize, max: usize| {
            (min..=max)
                .map(|i| {
                    (
                        self.tokens[4 * i + 1],
                        self.tokens[4 * i + 2],
                        self.tokens[4 * i + 3],
                    )
                })
                .collect::<Vec<_>>()
        };

        let mut low: isize = 0;
        let mut high: isize = self.token_count() as isize - 1;
        while low < high {
            let mid = low + (high - low) / 2;
            let mid_delta_line = self.delta_line(mid as usize);
            if mid_delta_line < delta_line {
                low = mid + 1;
            } else if mid_delta_line > delta_line {
                high = mid - 1;
            } else {
                let mut min = mid;
                while min > low && self.delta_line((min - 1) as usize) == delta_line {
                    min -= 1;
                }
                let mut max = mid;
                while max < high && self.delta_line((max + 1) as usize) == delta_line {
                    max += 1;
                }
                return Some(collect(min as usize, max as usize));
            }
        }

        if self.delta_line(low as usize) == delta_line {
            return Some(collect(low as usize, low as usize));
        }
        None
    }

    fn clear(&mut self) {
        self.tokens.clear();
    }

    fn write(&mut self, dest_index: usize, delta_line: i64, start: i64, end: i64, metadata: u32) {
        let offset = 4 * dest_index;
        self.tokens[offset] = delta_line as u32;
        self.tokens[offset + 1] = start as u32;
        self.tokens[offset + 2] = end as u32;
        self.tokens[offset + 3] = metadata;
    }

    /// Removes the tokens touching the given range and rebases the remaining
    /// ones so the first one is on delta line 0. Returns the delta line of the
    /// first remaining token (0 if none remain).
    fn remove_tokens(
        &mut self,
        start_delta_line: i64,
        start_char: i64,
        end_delta_line: i64,
        end_char: i64,
    ) -> i64 {
        let token_count = self.token_count();
        let mut new_token_count = 0;
        let mut first_delta_line = 0;
        for i in 0..token_count {
            let token_delta_line = self.delta_line(i);
            let token_start = self.start_character(i);
            let token_end = self.end_character(i);
            let metadata = self.tokens[4 * i + 3];

            if (token_delta_line > start_delta_line
                || (token_delta_line == start_delta_line && token_end >= start_char))
                && (token_delta_line < end_delta_line
                    || (token_delta_line == end_delta_line && token_start <= end_char))
            {
                // Deleted.
            } else {
                if new_token_count == 0 {
                    first_delta_line = token_delta_line;
                }
                self.write(
                    new_token_count,
                    token_delta_line - first_delta_line,
                    token_start,
                    token_end,
                    metadata,
                );
                new_token_count += 1;
            }
        }
        self.tokens.truncate(4 * new_token_count);
        first_delta_line
    }

    /// Splits into the tokens before the range and the tokens after it (tokens
    /// touching the range are dropped). The third element is the delta line of
    /// the first token after the range (the second storage is rebased to it).
    fn split(
        &self,
        start_delta_line: i64,
        start_char: i64,
        end_delta_line: i64,
        end_char: i64,
    ) -> (Self, Self, i64) {
        let mut a_tokens = Vec::new();
        let mut b_tokens = Vec::new();
        let mut in_b = false;
        let mut dest_first_delta_line = 0i64;
        for i in 0..self.token_count() {
            let token_delta_line = self.delta_line(i);
            let token_start = self.start_character(i);
            let token_end = self.end_character(i);
            let metadata = self.tokens[4 * i + 3];

            if token_delta_line > start_delta_line
                || (token_delta_line == start_delta_line && token_end >= start_char)
            {
                if token_delta_line < end_delta_line
                    || (token_delta_line == end_delta_line && token_start <= end_char)
                {
                    // This token is touching the range.
                    continue;
                } else if !in_b {
                    // This token is the first token after the range.
                    in_b = true;
                    dest_first_delta_line = token_delta_line;
                }
            }

            let dest = if in_b { &mut b_tokens } else { &mut a_tokens };
            dest.extend_from_slice(&[
                (token_delta_line - dest_first_delta_line) as u32,
                token_start as u32,
                token_end as u32,
                metadata,
            ]);
        }
        (
            Self::new(a_tokens),
            Self::new(b_tokens),
            dest_first_delta_line,
        )
    }

    /// Port of Monaco's `acceptDeleteRange`. The cases (token `-`, deletion
    /// `x`):
    ///
    /// 1. token starts before the deletion: 1a completely before; 1b deletion
    ///    ends after the token; 1c deletion ends with the token; 1d deletion is
    ///    inside the token.
    /// 2. token starts with the deletion: 2a ends inside it; 2b ends with it;
    ///    2c ends after it.
    /// 3. token starts inside the deletion: 3a inside; 3b ends with it; 3c ends
    ///    after it.
    /// 4. token starts after the deletion.
    fn accept_delete_range(
        &mut self,
        horizontal_shift_for_first_line_tokens: i64,
        start_delta_line: i64,
        start_character: i64,
        end_delta_line: i64,
        end_character: i64,
    ) {
        let token_count = self.token_count();
        let deleted_line_count = end_delta_line - start_delta_line;
        let mut new_token_count = 0;
        let mut has_deleted_tokens = false;
        for i in 0..token_count {
            let mut token_delta_line = self.delta_line(i);
            let mut token_start = self.start_character(i);
            let mut token_end = self.end_character(i);
            let metadata = self.tokens[4 * i + 3];

            if token_delta_line < start_delta_line
                || (token_delta_line == start_delta_line && token_end <= start_character)
            {
                // 1a. Completely before the deletion: nothing to do.
                new_token_count += 1;
                continue;
            } else if token_delta_line == start_delta_line && token_start < start_character {
                // 1b, 1c, 1d: the token survives but shrinks.
                if token_delta_line == end_delta_line && token_end > end_character {
                    // 1d. The deletion is inside the token.
                    token_end -= end_character - start_character;
                } else {
                    // 1b, 1c. The token's end moves to the deletion start.
                    token_end = start_character;
                }
            } else if token_delta_line == start_delta_line && token_start == start_character {
                // 2a, 2b, 2c
                if token_delta_line == end_delta_line && token_end > end_character {
                    // 2c. Ends after the deletion: shrinks by the deleted count.
                    token_end -= end_character - start_character;
                } else {
                    // 2a, 2b. The token is deleted.
                    has_deleted_tokens = true;
                    continue;
                }
            } else if token_delta_line < end_delta_line
                || (token_delta_line == end_delta_line && token_start < end_character)
            {
                // 3a, 3b, 3c
                if token_delta_line == end_delta_line && token_end > end_character {
                    // 3c. Ends after the deletion: moves to continue right after it.
                    token_delta_line = start_delta_line;
                    token_start = start_character;
                    token_end = token_start + (token_end - end_character);
                } else {
                    // 3a, 3b. The token is deleted.
                    has_deleted_tokens = true;
                    continue;
                }
            } else if token_delta_line > end_delta_line {
                // 4. (partial) Starts after the deletion, on a line below.
                if deleted_line_count == 0 && !has_deleted_tokens {
                    // Early stop: nothing else changes.
                    new_token_count = token_count;
                    break;
                }
                token_delta_line -= deleted_line_count;
            } else if token_delta_line == end_delta_line && token_start >= end_character {
                // 4. (continued) Starts after the deletion, on its last line.
                if horizontal_shift_for_first_line_tokens != 0 && token_delta_line == 0 {
                    token_start += horizontal_shift_for_first_line_tokens;
                    token_end += horizontal_shift_for_first_line_tokens;
                }
                token_delta_line -= deleted_line_count;
                token_start -= end_character - start_character;
                token_end -= end_character - start_character;
            } else {
                unreachable!("Not possible!");
            }

            self.write(
                new_token_count,
                token_delta_line,
                token_start,
                token_end,
                metadata,
            );
            new_token_count += 1;
        }
        self.tokens.truncate(4 * new_token_count);
    }

    /// Port of Monaco's `acceptInsertText`. The cases (token `-`, insertion
    /// point `|`): 1. token completely before; 2. token ends at the insertion
    /// point; 3. token contains it; 4. token starts at it; 5. token completely
    /// after it.
    fn accept_insert_text(
        &mut self,
        delta_line: i64,
        character: i64,
        eol_count: i64,
        first_line_length: i64,
        last_line_length: i64,
        first_char_code: u16,
    ) {
        let is_inserting_precisely_one_word_character =
            eol_count == 0 && first_line_length == 1 && is_word_char_code(first_char_code);
        for i in 0..self.token_count() {
            let mut token_delta_line = self.delta_line(i);
            let mut token_start = self.start_character(i);
            let mut token_end = self.end_character(i);

            if token_delta_line < delta_line
                || (token_delta_line == delta_line && token_end < character)
            {
                // 1. Completely before the insertion point: nothing to do.
                continue;
            } else if token_delta_line == delta_line && token_end == character {
                // 2. Ends precisely at the insertion point: grow only when
                // inserting precisely one word character.
                if is_inserting_precisely_one_word_character {
                    token_end += 1;
                } else {
                    continue;
                }
            } else if token_delta_line == delta_line
                && token_start < character
                && character < token_end
            {
                // 3. Contains the insertion point.
                if eol_count == 0 {
                    token_end += first_line_length;
                } else {
                    token_end = character;
                }
            } else {
                // 4. or 5.
                if token_delta_line == delta_line
                    && token_start == character
                    && is_inserting_precisely_one_word_character
                {
                    // 4. Starts at the insertion point: grow by keeping the start
                    // constant when inserting one word character.
                    continue;
                }
                // The token moves and keeps its size.
                if token_delta_line == delta_line {
                    token_delta_line += eol_count;
                    if eol_count == 0 {
                        token_start += first_line_length;
                        token_end += first_line_length;
                    } else {
                        let token_length = token_end - token_start;
                        token_start = last_line_length + (token_start - character);
                        token_end = token_start + token_length;
                    }
                } else {
                    token_delta_line += eol_count;
                }
            }

            let metadata = self.tokens[4 * i + 3];
            self.write(i, token_delta_line, token_start, token_end, metadata);
        }
    }
}

/// Sparse semantic tokens over a contiguous range of lines (one "area"). Port
/// of Monaco's `SparseMultilineTokens`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SparseMultilineTokens {
    start_line: i64,
    end_line: i64,
    tokens: SparseMultilineTokensStorage,
}

impl SparseMultilineTokens {
    /// Creates an area starting at `start_line` (0-based) from tokens encoded
    /// as `[deltaLine, startCharacter, endCharacter, style]` quadruples, sorted
    /// by position, with `deltaLine` relative to `start_line`.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn create(start_line: u32, tokens: Vec<u32>) -> Self {
        Self::from_parts(
            i64::from(start_line),
            SparseMultilineTokensStorage::new(tokens),
        )
    }

    fn from_parts(start_line: i64, tokens: SparseMultilineTokensStorage) -> Self {
        let end_line = start_line + tokens.max_delta_line();
        Self {
            start_line,
            end_line,
            tokens,
        }
    }

    /// (Inclusive) 0-based start line of the area.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn start_line(&self) -> i64 {
        self.start_line
    }

    /// (Inclusive) 0-based end line of the area: the line of the last token, or
    /// `start_line() - 1` when the area is empty.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn end_line(&self) -> i64 {
        self.end_line
    }

    fn update_end_line(&mut self) {
        self.end_line = self.start_line + self.tokens.max_delta_line();
    }

    /// Whether the area holds no tokens.
    pub(crate) fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The tokens on `line` as `(start, end, style)`, or `None` when the line is
    /// outside the area or has no tokens. Port of `getLineTokens`.
    pub(crate) fn line_tokens(&self, line: u32) -> Option<Vec<SparseLineToken>> {
        let line = i64::from(line);
        if self.start_line <= line && line <= self.end_line {
            return self.tokens.line_tokens(line - self.start_line);
        }
        None
    }

    fn line_range(&self) -> Option<LineRange> {
        let delta = self.tokens.range()?;
        Some(LineRange::new(
            self.start_line + delta.start_line,
            delta.start_col,
            self.start_line + delta.end_line,
            delta.end_col,
        ))
    }

    /// The range from the first token's start to the last token's end, or
    /// `None` when empty. Port of `getRange`.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn range(&self) -> Option<Range<PointUtf16>> {
        self.line_range().map(LineRange::to_points)
    }

    /// Removes every token touching `range` (inclusive at both ends) and
    /// rebases the area on its first remaining token. Port of `removeTokens`.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn remove_tokens(&mut self, range: Range<PointUtf16>) {
        self.remove_tokens_in(LineRange::from_points(&range));
    }

    fn remove_tokens_in(&mut self, range: LineRange) {
        let start_line_index = range.start_line - self.start_line;
        let end_line_index = range.end_line - self.start_line;
        self.start_line += self.tokens.remove_tokens(
            start_line_index,
            range.start_col,
            end_line_index,
            range.end_col,
        );
        self.update_end_line();
    }

    /// Splits into the tokens before `range` and the tokens after it; tokens
    /// touching `range` are dropped. Port of `split`.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn split(&self, range: Range<PointUtf16>) -> (Self, Self) {
        self.split_in(LineRange::from_points(&range))
    }

    fn split_in(&self, range: LineRange) -> (Self, Self) {
        let start_line_index = range.start_line - self.start_line;
        let end_line_index = range.end_line - self.start_line;
        let (a, b, b_delta_line) = self.tokens.split(
            start_line_index,
            range.start_col,
            end_line_index,
            range.end_col,
        );
        (
            Self::from_parts(self.start_line, a),
            Self::from_parts(self.start_line + b_delta_line, b),
        )
    }

    /// Adjusts the tokens for replacing `range` with `text`. Port of
    /// `applyEdit`.
    pub(crate) fn apply_edit(&mut self, range: Range<PointUtf16>, text: &str) {
        let (eol_count, first_line_length, last_line_length) = count_eol(text);
        let first_char_code = text.encode_utf16().next().unwrap_or(0);
        self.accept_edit(
            range,
            eol_count,
            first_line_length,
            last_line_length,
            first_char_code,
        );
    }

    /// Adjusts the tokens for replacing `range` with text that has `eol_count`
    /// line breaks, a first line of `first_line_length` UTF-16 units, a last
    /// line of `last_line_length` units, and starts with `first_char_code`
    /// (0 for empty text). Typing a single ASCII letter or digit at a token's
    /// end extends the token. Port of `acceptEdit`.
    pub(crate) fn accept_edit(
        &mut self,
        range: Range<PointUtf16>,
        eol_count: u32,
        first_line_length: u32,
        last_line_length: u32,
        first_char_code: u16,
    ) {
        let range = LineRange::from_points(&range);
        self.accept_delete_range(range);
        self.accept_insert_text(
            range.start_line,
            range.start_col,
            i64::from(eol_count),
            i64::from(first_line_length),
            i64::from(last_line_length),
            first_char_code,
        );
        self.update_end_line();
    }

    fn accept_delete_range(&mut self, range: LineRange) {
        if range.start_line == range.end_line && range.start_col == range.end_col {
            // Nothing to delete.
            return;
        }

        let first_line_index = range.start_line - self.start_line;
        let last_line_index = range.end_line - self.start_line;

        if last_line_index < 0 {
            // Entirely before this block: only adjust line numbers.
            self.start_line -= last_line_index - first_line_index;
            return;
        }

        let token_max_delta_line = self.tokens.max_delta_line();

        if first_line_index > token_max_delta_line {
            // Entirely after this block.
            return;
        }

        if first_line_index < 0 && last_line_index > token_max_delta_line {
            // Completely encompasses this block.
            self.start_line = 0;
            self.tokens.clear();
            return;
        }

        if first_line_index < 0 {
            let deleted_before = -first_line_index;
            self.start_line -= deleted_before;
            self.tokens
                .accept_delete_range(range.start_col, 0, 0, last_line_index, range.end_col);
        } else {
            self.tokens.accept_delete_range(
                0,
                first_line_index,
                range.start_col,
                last_line_index,
                range.end_col,
            );
        }
    }

    fn accept_insert_text(
        &mut self,
        line: i64,
        column: i64,
        eol_count: i64,
        first_line_length: i64,
        last_line_length: i64,
        first_char_code: u16,
    ) {
        if eol_count == 0 && first_line_length == 0 {
            // Nothing to insert.
            return;
        }

        let line_index = line - self.start_line;

        if line_index < 0 {
            // Before this block: only adjust line numbers.
            self.start_line += eol_count;
            return;
        }

        let token_max_delta_line = self.tokens.max_delta_line();

        if line_index > token_max_delta_line {
            // After this block.
            return;
        }

        self.tokens.accept_insert_text(
            line_index,
            column,
            eol_count,
            first_line_length,
            last_line_length,
            first_char_code,
        );
    }
}

/// The semantic tokens of a document, as a sorted list of areas ("pieces").
/// Port of Monaco's `SparseTokensStore` (minus the metadata merge).
#[derive(Clone, Debug, Default)]
pub(crate) struct SparseTokensStore {
    pieces: Vec<SparseMultilineTokens>,
    is_complete: bool,
}

impl SparseTokensStore {
    /// An empty, incomplete store.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Drops all tokens and marks the store incomplete. Port of `flush`.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn clear(&mut self) {
        self.pieces.clear();
        self.is_complete = false;
    }

    /// Whether the store has no pieces.
    pub(crate) fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// The stored pieces, in document order.
    #[cfg_attr(not(test), allow(dead_code))] // Used by tests (Monaco parity).
    pub(crate) fn pieces(&self) -> &[SparseMultilineTokens] {
        &self.pieces
    }

    /// Replaces all tokens. `is_complete` records whether they cover the whole
    /// document (a full provider result) rather than a range.
    pub(crate) fn set(&mut self, pieces: Option<Vec<SparseMultilineTokens>>, is_complete: bool) {
        self.pieces = pieces.unwrap_or_default();
        self.is_complete = is_complete;
    }

    /// Replaces the tokens in `range` (extended to cover `pieces`) with
    /// `pieces` (a range provider result). Existing pieces are trimmed or split
    /// around the range. Returns the range that changed. Port of `setPartial`.
    pub(crate) fn set_partial(
        &mut self,
        range: Range<PointUtf16>,
        pieces: Vec<SparseMultilineTokens>,
    ) -> Range<PointUtf16> {
        let mut line_range = LineRange::from_points(&range);
        if let (Some(first), Some(last)) = (pieces.first(), pieces.last()) {
            let (Some(first_range), Some(last_range)) = (first.line_range(), last.line_range())
            else {
                return range;
            };
            line_range = line_range.plus(first_range).plus(last_range);
        }
        let range = line_range;

        let mut insert_position: Option<usize> = None;
        let mut i = 0;
        while i < self.pieces.len() {
            let piece = &mut self.pieces[i];
            if piece.end_line < range.start_line {
                // Before the range.
                i += 1;
                continue;
            }

            if piece.start_line > range.end_line {
                // After the range: insert before it and stop.
                insert_position.get_or_insert(i);
                break;
            }

            // This piece might intersect with the range.
            piece.remove_tokens_in(range);

            if piece.is_empty() {
                self.pieces.remove(i);
                continue;
            }

            if piece.end_line < range.start_line {
                // After removal, before the range.
                i += 1;
                continue;
            }

            if piece.start_line > range.end_line {
                // After removal, after the range.
                insert_position.get_or_insert(i);
                i += 1;
                continue;
            }

            // After removal, this piece contains the range.
            let (a, b) = piece.split_in(range);
            if a.is_empty() {
                // Actually after the range.
                insert_position.get_or_insert(i);
                i += 1;
                continue;
            }
            if b.is_empty() {
                // Actually before the range.
                i += 1;
                continue;
            }
            self.pieces.splice(i..=i, [a, b]);
            i += 1;
            insert_position.get_or_insert(i);
            i += 1;
        }

        let insert_position = insert_position.unwrap_or(self.pieces.len());
        if !pieces.is_empty() {
            self.pieces.splice(insert_position..insert_position, pieces);
        }

        range.to_points()
    }

    /// Whether the last [`Self::set`] covered the whole document.
    pub(crate) fn is_complete(&self) -> bool {
        self.is_complete
    }

    /// The semantic tokens on `line` as `(start, end, style)`. Like Monaco's
    /// `addSparseTokens`, only the first piece containing the line is
    /// consulted. Tokens are not clamped to the line length and, after edits,
    /// may overlap; the caller must clamp and keep end offsets monotonic.
    pub(crate) fn line_tokens(&self, line: u32) -> Vec<SparseLineToken> {
        if self.pieces.is_empty() {
            return Vec::new();
        }
        let index = Self::find_first_piece_with_line(&self.pieces, i64::from(line));
        self.pieces[index].line_tokens(line).unwrap_or_default()
    }

    /// Binary search for the first piece containing `line`; returns some index
    /// in bounds even when no piece contains it.
    fn find_first_piece_with_line(pieces: &[SparseMultilineTokens], line: i64) -> usize {
        let mut low: isize = 0;
        let mut high: isize = pieces.len() as isize - 1;
        while low < high {
            let mut mid = low + (high - low) / 2;
            if pieces[mid as usize].end_line < line {
                low = mid + 1;
            } else if pieces[mid as usize].start_line > line {
                high = mid - 1;
            } else {
                while mid > low
                    && pieces[(mid - 1) as usize].start_line <= line
                    && line <= pieces[(mid - 1) as usize].end_line
                {
                    mid -= 1;
                }
                return mid as usize;
            }
        }
        low as usize
    }

    /// Adjusts every piece for an edit (see
    /// [`SparseMultilineTokens::accept_edit`]) and removes pieces that became
    /// empty.
    pub(crate) fn accept_edit(
        &mut self,
        range: Range<PointUtf16>,
        eol_count: u32,
        first_line_length: u32,
        last_line_length: u32,
        first_char_code: u16,
    ) {
        self.pieces.retain_mut(|piece| {
            piece.accept_edit(
                range.clone(),
                eol_count,
                first_line_length,
                last_line_length,
                first_char_code,
            );
            !piece.is_empty()
        });
    }
}

/// Decodes LSP semantic token data (5 `u32`s per token: `deltaLine`,
/// `deltaStartCharacter`, `length`, `tokenType`, `tokenModifiers`) into areas
/// of about `max(ceil(count / 1024), 400)` tokens, never splitting a line
/// across areas. Tokens with `end <= start`, tokens overlapping the previous
/// kept token on the same line, and tokens for which `style` returns `None`
/// are dropped. Port of Monaco's `toMultilineTokens2`, including its int32
/// wrap-around arithmetic.
pub(crate) fn to_multiline_tokens(
    data: &[u32],
    style: &mut dyn FnMut(u32, u32) -> Option<u32>,
) -> Vec<SparseMultilineTokens> {
    // Monaco's `x | 0`.
    fn int32(value: i64) -> i64 {
        i64::from(value as i32)
    }

    let token_count = data.len() / 5;
    let tokens_per_area = token_count
        .div_ceil(DESIRED_MAX_AREAS)
        .max(DESIRED_TOKENS_PER_AREA);
    let mut result = Vec::new();

    let mut token_index = 0;
    let mut last_line: i64 = 0;
    let mut last_start_character: i64 = 0;
    while token_index < token_count {
        let token_start_index = token_index;
        let mut token_end_index = (token_start_index + tokens_per_area).min(token_count);

        // Keep tokens on the same line in the same area.
        if token_end_index < token_count {
            let mut small_token_end_index = token_end_index;
            while small_token_end_index - 1 > token_start_index
                && data[5 * small_token_end_index] == 0
            {
                small_token_end_index -= 1;
            }

            if small_token_end_index - 1 == token_start_index {
                // So many tokens on this line that the area would be empty: go
                // right instead.
                let mut big_token_end_index = token_end_index;
                while big_token_end_index + 1 < token_count && data[5 * big_token_end_index] == 0 {
                    big_token_end_index += 1;
                }
                token_end_index = big_token_end_index;
            } else {
                token_end_index = small_token_end_index;
            }
        }

        let mut dest = Vec::with_capacity((token_end_index - token_start_index) * 4);
        // Monaco uses `0` (an invalid 1-based line) as "unset"; that is `-1`
        // here.
        let mut area_line: i64 = -1;
        let mut prev_line: i64 = -1;
        let mut prev_end_character: i64 = 0;
        while token_index < token_end_index {
            let src = 5 * token_index;
            let delta_line = i64::from(data[src]);
            let delta_character = i64::from(data[src + 1]);
            let line = int32(last_line + delta_line);
            let start_character = if delta_line == 0 {
                int32(last_start_character + delta_character)
            } else {
                delta_character
            };
            let length = i64::from(data[src + 2]);
            let end_character = int32(start_character + length);
            let token_type = data[src + 3];
            let token_modifiers = data[src + 4];

            if end_character <= start_character {
                // Invalid (most likely a negative length cast to u32).
            } else if prev_line == line && prev_end_character > start_character {
                // Overlaps the previous token.
            } else if let Some(metadata) = style(token_type, token_modifiers) {
                if area_line == -1 {
                    area_line = line;
                }
                dest.extend_from_slice(&[
                    (line - area_line) as u32,
                    start_character as u32,
                    end_character as u32,
                    metadata,
                ]);
                prev_line = line;
                prev_end_character = end_character;
            }

            last_line = line;
            last_start_character = start_character;
            token_index += 1;
        }

        result.push(SparseMultilineTokens::from_parts(
            area_line,
            SparseMultilineTokensStorage::new(dest),
        ));
    }

    result
}

#[cfg(test)]
mod tests {
    //! Ported from Monaco's `test/common/model/tokensStore.test.ts` and
    //! `test/common/services/semanticTokensProviderStyling.test.ts`, plus a few
    //! direct tests of the edit cases and area splitting.

    use super::*;

    /// The style id used for tokens written as `|text|` in the test states
    /// (Monaco's `SEMANTIC_COLOR`).
    const SEMANTIC: u32 = 5;

    fn pt(row: u32, column: u32) -> PointUtf16 {
        PointUtf16::new(row, column)
    }

    /// Converts a Monaco 1-based `Range(startLine, startCol, endLine, endCol)`.
    fn monaco_range(sl: u32, sc: u32, el: u32, ec: u32) -> Range<PointUtf16> {
        pt(sl - 1, sc - 1)..pt(el - 1, ec - 1)
    }

    /// `(eol_count, first_line_length, last_line_length, first_char_code)` for
    /// `text`, as Monaco's model computes them for `acceptEdit`.
    fn edit_args(text: &str) -> (u32, u32, u32, u16) {
        let (eol, first, last) = count_eol(text);
        (eol, first, last, text.encode_utf16().next().unwrap_or(0))
    }

    /// Port of `parseTokensState`: `|xyz|` marks a semantic token, `||` is
    /// literal text.
    fn parse_tokens_state(state: &[&str]) -> (Vec<String>, SparseMultilineTokens) {
        let mut text = Vec::new();
        let mut tokens = Vec::new();
        let mut base_line = 0u32;
        for (i, line) in state.iter().enumerate() {
            let mut start_offset = 0;
            let mut line_text = String::new();
            while let Some(first_pipe) = line[start_offset..].find('|').map(|o| o + start_offset) {
                let Some(second_pipe) =
                    line[first_pipe + 1..].find('|').map(|o| o + first_pipe + 1)
                else {
                    break;
                };
                if first_pipe + 1 == second_pipe {
                    line_text.push_str(&line[start_offset..=second_pipe]);
                    start_offset = second_pipe + 1;
                    continue;
                }
                line_text.push_str(&line[start_offset..first_pipe]);
                let token_start = line_text.len() as u32;
                let token_length = (second_pipe - first_pipe - 1) as u32;
                if tokens.is_empty() {
                    base_line = i as u32;
                }
                tokens.extend_from_slice(&[
                    i as u32 - base_line,
                    token_start,
                    token_start + token_length,
                    SEMANTIC,
                ]);
                line_text.push_str(&line[first_pipe + 1..second_pipe]);
                start_offset = second_pipe + 1;
            }
            line_text.push_str(&line[start_offset..]);
            text.push(line_text);
        }
        (text, SparseMultilineTokens::create(base_line, tokens))
    }

    /// Port of `extractState`: renders each line with `|...|` around semantic
    /// tokens. A model without a tokenizer has one plain token per line; this
    /// replays `addSparseTokens`' merge against it (clamping to the line length
    /// and skipping non-increasing end offsets).
    fn extract_state(lines: &[String], store: &SparseTokensStore) -> Vec<String> {
        lines
            .iter()
            .enumerate()
            .map(|(i, content)| {
                let len = content.len() as u32;
                if len == 0 {
                    return String::new();
                }
                let mut segments: Vec<(u32, bool)> = Vec::new();
                let mut last_end = 0;
                let mut emit = |end: u32, semantic: bool| {
                    if end > last_end {
                        last_end = end;
                        segments.push((end, semantic));
                    }
                };
                let mut a_consumed = false;
                for (start, end, style) in store.line_tokens(i as u32) {
                    let b_start = start.min(len);
                    let b_end = end.min(len);
                    let semantic = style == SEMANTIC;
                    if !a_consumed && len <= b_start {
                        emit(len, false);
                        a_consumed = true;
                    }
                    if !a_consumed && 0 < b_start {
                        emit(b_start, false);
                    }
                    emit(b_end, semantic);
                    if !a_consumed && len == b_end {
                        a_consumed = true;
                    }
                }
                if !a_consumed {
                    emit(len, false);
                }
                let mut out = String::new();
                let mut start = 0;
                for (end, semantic) in segments {
                    let piece = &content[start as usize..end as usize];
                    if semantic {
                        out.push('|');
                        out.push_str(piece);
                        out.push('|');
                    } else {
                        out.push_str(piece);
                    }
                    start = end;
                }
                out
            })
            .collect()
    }

    /// Replaces `range` in `lines` (ASCII only).
    fn replace_text(lines: &mut Vec<String>, range: &Range<PointUtf16>, text: &str) {
        let offset = |p: PointUtf16| {
            lines[..p.row as usize]
                .iter()
                .map(|l| l.len() + 1)
                .sum::<usize>()
                + p.column as usize
        };
        let (start, end) = (offset(range.start), offset(range.end));
        let mut full = lines.join("\n");
        full.replace_range(start..end, text);
        *lines = full.split('\n').map(str::to_owned).collect();
    }

    /// Port of `testTokensAdjustment`. Like Monaco's piece tree, the edits
    /// (non-overlapping, in original coordinates) are applied bottom-up so each
    /// range is valid when it is applied.
    fn test_tokens_adjustment(
        initial: &[&str],
        edits: &[(Range<PointUtf16>, &str)],
        expected: &[&str],
    ) {
        let (mut lines, tokens) = parse_tokens_state(initial);
        let mut store = SparseTokensStore::new();
        store.set(Some(vec![tokens]), true);

        let mut edits = edits.to_vec();
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.0.start));
        for (range, text) in edits {
            replace_text(&mut lines, &range, text);
            let (eol, first, last, code) = edit_args(text);
            store.accept_edit(range, eol, first, last, code);
        }

        assert_eq!(extract_state(&lines, &store), expected);
    }

    #[test]
    fn issue_86303_color_shifting_between_different_tokens() {
        test_tokens_adjustment(
            &[
                "import { |URI| } from 'vs/base/common/uri';",
                "const foo = |URI|.parse('hey');",
            ],
            &[(monaco_range(2, 9, 2, 10), "")],
            &[
                "import { |URI| } from 'vs/base/common/uri';",
                "const fo = |URI|.parse('hey');",
            ],
        );
    }

    #[test]
    fn deleting_a_newline() {
        test_tokens_adjustment(
            &[
                "import { |URI| } from 'vs/base/common/uri';",
                "const foo = |URI|.parse('hey');",
            ],
            &[(monaco_range(1, 42, 2, 1), "")],
            &["import { |URI| } from 'vs/base/common/uri';const foo = |URI|.parse('hey');"],
        );
    }

    #[test]
    fn inserting_a_newline() {
        test_tokens_adjustment(
            &["import { |URI| } from 'vs/base/common/uri';const foo = |URI|.parse('hey');"],
            &[(monaco_range(1, 42, 1, 42), "\n")],
            &[
                "import { |URI| } from 'vs/base/common/uri';",
                "const foo = |URI|.parse('hey');",
            ],
        );
    }

    #[test]
    fn deleting_a_newline_2() {
        test_tokens_adjustment(
            &[
                "import { ",
                "    |URI| } from 'vs/base/common/uri';const foo = |URI|.parse('hey');",
            ],
            &[(monaco_range(1, 10, 2, 5), "")],
            &["import { |URI| } from 'vs/base/common/uri';const foo = |URI|.parse('hey');"],
        );
    }

    #[test]
    fn issue_179268_a_complex_edit() {
        test_tokens_adjustment(
            &[
                "|export| |'interior_material_selector.dart'|;",
                "|export| |'mileage_selector.dart'|;",
                "|export| |'owners_selector.dart'|;",
                "|export| |'price_selector.dart'|;",
                "|export| |'seat_count_selector.dart'|;",
                "|export| |'year_selector.dart'|;",
                "|export| |'winter_options_selector.dart'|;|export| |'camera_selector.dart'|;",
            ],
            &[
                (monaco_range(1, 9, 1, 9), "camera_selector.dart';\nexport '"),
                (monaco_range(6, 9, 7, 9), ""),
                (monaco_range(7, 39, 7, 39), "\n"),
                (monaco_range(7, 47, 7, 48), "ye"),
                (monaco_range(7, 49, 7, 51), ""),
                (monaco_range(7, 52, 7, 53), ""),
            ],
            &[
                "|export| |'|camera_selector.dart';",
                "export 'interior_material_selector.dart';",
                "|export| |'mileage_selector.dart'|;",
                "|export| |'owners_selector.dart'|;",
                "|export| |'price_selector.dart'|;",
                "|export| |'seat_count_selector.dart'|;",
                "|export| |'||winter_options_selector.dart'|;",
                "|export| |'year_selector.dart'|;",
            ],
        );
    }

    /// The TS test checks the merged metadata of `addSparseTokens`, which is
    /// not ported; this checks the semantic tokens the merge would consume.
    #[test]
    fn issue_91936_semantic_tokens_on_line_with_selected_text() {
        let raw = vec![
            0,
            20,
            24,
            0b01111000000000010000, //
            0,
            25,
            27,
            0b01111000000000010000, //
            0,
            28,
            29,
            0b00001000000000010000, //
            0,
            29,
            31,
            0b10000000000000010000, //
            0,
            32,
            33,
            0b00001000000000010000, //
            0,
            34,
            36,
            0b00110000000000010000, //
            0,
            36,
            37,
            0b00001000000000010000, //
            0,
            38,
            42,
            0b01111000000000010000, //
            0,
            43,
            47,
            0b01011000000000010000,
        ];
        let mut store = SparseTokensStore::new();
        store.set(
            Some(vec![SparseMultilineTokens::create(0, raw.clone())]),
            true,
        );
        let expected: Vec<_> = raw.chunks(4).map(|c| (c[1], c[2], c[3])).collect();
        assert_eq!(store.line_tokens(0), expected);
        assert!(store.line_tokens(1).is_empty());
    }

    // `issue #147944: Language id "vs.editor.nullLanguage" is not configured
    // nor known` is skipped: it checks `TextModel.getWordAtPosition` with a
    // zero-length semantic token, which needs a model and language config.

    fn partial_tokens_initial(store: &mut SparseTokensStore) {
        // setPartial: [1,1 -> 31,2], [(5,5-10),(10,5-10),(15,5-10),(20,5-10),(25,5-10),(30,5-10)]
        store.set_partial(
            monaco_range(1, 1, 31, 2),
            vec![SparseMultilineTokens::create(
                4,
                vec![
                    0, 5, 10, 1, //
                    5, 5, 10, 2, //
                    10, 5, 10, 3, //
                    15, 5, 10, 4, //
                    20, 5, 10, 5, //
                    25, 5, 10, 6,
                ],
            )],
        );
    }

    /// The TS tests assert `addSparseTokens` yields 3 tokens for a 12-char
    /// line, i.e. exactly one semantic token at 5..10 is found.
    #[test]
    fn partial_tokens_1() {
        let mut store = SparseTokensStore::new();
        partial_tokens_initial(&mut store);
        // setPartial: [18,1 -> 42,1], [(20,5-10),(25,5-10),(30,5-10),(35,5-10),(40,5-10)]
        store.set_partial(
            monaco_range(18, 1, 42, 1),
            vec![SparseMultilineTokens::create(
                19,
                vec![
                    0, 5, 10, 4, //
                    5, 5, 10, 5, //
                    10, 5, 10, 6, //
                    15, 5, 10, 7, //
                    20, 5, 10, 8,
                ],
            )],
        );
        partial_tokens_initial(&mut store);
        assert_eq!(store.line_tokens(9), vec![(5, 10, 2)]);
    }

    #[test]
    fn partial_tokens_2() {
        let mut store = SparseTokensStore::new();
        partial_tokens_initial(&mut store);
        // setPartial: [6,1 -> 36,2], [(10,5-10),(15,5-10),(20,5-10),(25,5-10),(30,5-10),(35,5-10)]
        store.set_partial(
            monaco_range(6, 1, 36, 2),
            vec![SparseMultilineTokens::create(
                9,
                vec![
                    0, 5, 10, 2, //
                    5, 5, 10, 3, //
                    10, 5, 10, 4, //
                    15, 5, 10, 5, //
                    20, 5, 10, 6,
                ],
            )],
        );
        // setPartial: [17,1 -> 42,1], [(20,5-10),(25,5-10),(30,5-10),(35,5-10),(40,5-10)]
        store.set_partial(
            monaco_range(17, 1, 42, 1),
            vec![SparseMultilineTokens::create(
                19,
                vec![
                    0, 5, 10, 4, //
                    5, 5, 10, 5, //
                    10, 5, 10, 6, //
                    15, 5, 10, 7, //
                    20, 5, 10, 8,
                ],
            )],
        );
        assert_eq!(store.line_tokens(19), vec![(5, 10, 4)]);
    }

    #[test]
    fn partial_tokens_3() {
        let mut store = SparseTokensStore::new();
        partial_tokens_initial(&mut store);
        // setPartial: [11,1 -> 16,2], [(15,5-10),(20,5-10)]
        store.set_partial(
            monaco_range(11, 1, 16, 2),
            vec![SparseMultilineTokens::create(
                9,
                vec![
                    0, 5, 10, 3, //
                    5, 5, 10, 4,
                ],
            )],
        );
        assert_eq!(store.line_tokens(4), vec![(5, 10, 1)]);
    }

    #[test]
    fn issue_94133_semantic_colors_stick_around_with_range_provider() {
        let mut store = SparseTokensStore::new();
        // setPartial: [1,1 -> 1,20] [(1,9-11)]
        store.set_partial(
            monaco_range(1, 1, 1, 20),
            vec![SparseMultilineTokens::create(0, vec![0, 9, 11, 1])],
        );
        // setPartial: [1,1 -> 1,20], []
        store.set_partial(monaco_range(1, 1, 1, 20), vec![]);
        assert!(store.line_tokens(0).is_empty());
    }

    /// Port of the `bug` test's `createTokens`: parses Monaco's
    /// `[(line,start-end),...]` dump (1-based lines) into an area, with
    /// metadata `(line + start) % 13`.
    fn create_tokens(s: &str) -> SparseMultilineTokens {
        let s = s.trim_start_matches("[(").trim_end_matches(")]");
        let mut result = Vec::new();
        let mut first_line = 0u32;
        for token in s.split("),(") {
            let (line, chars) = token.split_once(',').unwrap();
            let (start, end) = chars.split_once('-').unwrap();
            let line: u32 = line.parse().unwrap();
            let start: u32 = start.parse().unwrap();
            let end: u32 = end.parse().unwrap();
            if first_line == 0 {
                first_line = line;
            }
            result.extend_from_slice(&[line - first_line, start, end, (line + start) % 13]);
        }
        SparseMultilineTokens::create(first_line - 1, result)
    }

    #[test]
    fn bug() {
        let mut store = SparseTokensStore::new();
        store.set_partial(
            monaco_range(36446, 1, 36475, 115),
            vec![create_tokens(
                "[(36448,24-29),(36448,33-46),(36448,47-54),(36450,25-35),(36450,36-50),(36451,28-33),(36451,36-49),(36451,50-57),(36452,35-53),(36452,54-62),(36454,33-38),(36454,41-54),(36454,55-60),(36455,35-53),(36455,54-62),(36457,33-44),(36457,45-49),(36457,50-56),(36457,62-83),(36457,84-88),(36458,35-53),(36458,54-62),(36460,33-37),(36460,38-42),(36460,47-57),(36460,58-67),(36461,35-53),(36461,54-62),(36463,34-38),(36463,39-45),(36463,46-51),(36463,54-63),(36463,64-71),(36463,76-80),(36463,81-87),(36463,88-92),(36463,97-107),(36463,108-119),(36464,35-53),(36464,54-62),(36466,33-71),(36466,72-76),(36467,35-53),(36467,54-62),(36469,24-29),(36469,33-46),(36469,47-54),(36470,24-35),(36470,38-46),(36473,25-35),(36473,36-51),(36474,28-33),(36474,36-49),(36474,50-58),(36475,35-53),(36475,54-62)]",
            )],
        );
        store.set_partial(
            monaco_range(36436, 1, 36464, 142),
            vec![create_tokens(
                "[(36437,33-37),(36437,38-42),(36437,47-57),(36437,58-67),(36438,35-53),(36438,54-62),(36440,24-29),(36440,33-46),(36440,47-53),(36442,25-35),(36442,36-50),(36443,30-39),(36443,42-46),(36443,47-53),(36443,54-58),(36443,63-73),(36443,74-84),(36443,87-91),(36443,92-98),(36443,101-105),(36443,106-112),(36443,113-119),(36444,28-37),(36444,38-42),(36444,47-57),(36444,58-75),(36444,80-95),(36444,96-105),(36445,35-53),(36445,54-62),(36448,24-29),(36448,33-46),(36448,47-54),(36450,25-35),(36450,36-50),(36451,28-33),(36451,36-49),(36451,50-57),(36452,35-53),(36452,54-62),(36454,33-38),(36454,41-54),(36454,55-60),(36455,35-53),(36455,54-62),(36457,33-44),(36457,45-49),(36457,50-56),(36457,62-83),(36457,84-88),(36458,35-53),(36458,54-62),(36460,33-37),(36460,38-42),(36460,47-57),(36460,58-67),(36461,35-53),(36461,54-62),(36463,34-38),(36463,39-45),(36463,46-51),(36463,54-63),(36463,64-71),(36463,76-80),(36463,81-87),(36463,88-92),(36463,97-107),(36463,108-119),(36464,35-53),(36464,54-62)]",
            )],
        );
        store.set_partial(
            monaco_range(36457, 1, 36485, 140),
            vec![create_tokens(
                "[(36457,33-44),(36457,45-49),(36457,50-56),(36457,62-83),(36457,84-88),(36458,35-53),(36458,54-62),(36460,33-37),(36460,38-42),(36460,47-57),(36460,58-67),(36461,35-53),(36461,54-62),(36463,34-38),(36463,39-45),(36463,46-51),(36463,54-63),(36463,64-71),(36463,76-80),(36463,81-87),(36463,88-92),(36463,97-107),(36463,108-119),(36464,35-53),(36464,54-62),(36466,33-71),(36466,72-76),(36467,35-53),(36467,54-62),(36469,24-29),(36469,33-46),(36469,47-54),(36470,24-35),(36470,38-46),(36473,25-35),(36473,36-51),(36474,28-33),(36474,36-49),(36474,50-58),(36475,35-53),(36475,54-62),(36477,28-32),(36477,33-37),(36477,42-52),(36477,53-69),(36478,32-36),(36478,37-41),(36478,46-56),(36478,57-74),(36479,32-36),(36479,37-41),(36479,46-56),(36479,57-76),(36480,32-36),(36480,37-41),(36480,46-56),(36480,57-68),(36481,32-36),(36481,37-41),(36481,46-56),(36481,57-68),(36482,39-57),(36482,58-66),(36484,34-38),(36484,39-45),(36484,46-50),(36484,55-65),(36484,66-82),(36484,86-97),(36484,98-102),(36484,103-109),(36484,111-124),(36484,125-133),(36485,39-57),(36485,58-66)]",
            )],
        );
        store.set_partial(
            monaco_range(36441, 1, 36469, 56),
            vec![create_tokens(
                "[(36442,25-35),(36442,36-50),(36443,30-39),(36443,42-46),(36443,47-53),(36443,54-58),(36443,63-73),(36443,74-84),(36443,87-91),(36443,92-98),(36443,101-105),(36443,106-112),(36443,113-119),(36444,28-37),(36444,38-42),(36444,47-57),(36444,58-75),(36444,80-95),(36444,96-105),(36445,35-53),(36445,54-62),(36448,24-29),(36448,33-46),(36448,47-54),(36450,25-35),(36450,36-50),(36451,28-33),(36451,36-49),(36451,50-57),(36452,35-53),(36452,54-62),(36454,33-38),(36454,41-54),(36454,55-60),(36455,35-53),(36455,54-62),(36457,33-44),(36457,45-49),(36457,50-56),(36457,62-83),(36457,84-88),(36458,35-53),(36458,54-62),(36460,33-37),(36460,38-42),(36460,47-57),(36460,58-67),(36461,35-53),(36461,54-62),(36463,34-38),(36463,39-45),(36463,46-51),(36463,54-63),(36463,64-71),(36463,76-80),(36463,81-87),(36463,88-92),(36463,97-107),(36463,108-119),(36464,35-53),(36464,54-62),(36466,33-71),(36466,72-76),(36467,35-53),(36467,54-62),(36469,24-29),(36469,33-46),(36469,47-54),(36470,24-35)]",
            )],
        );

        // The TS test expects 7 merged tokens on a 60-char line: the 3
        // semantic tokens plus the 4 gaps around them.
        let meta = |start: u32| (36451 + start) % 13;
        assert_eq!(
            store.line_tokens(36450),
            vec![(28, 33, meta(28)), (36, 49, meta(36)), (50, 57, meta(50))]
        );
    }

    /// The TS test checks `addSparseTokens`' metadata-bit merge (not ported);
    /// this checks the semantic token it would merge.
    #[test]
    fn issue_95949_identifiers_colored_bold_when_targeting_keywords() {
        let mut store = SparseTokensStore::new();
        store.set(
            Some(vec![SparseMultilineTokens::create(0, vec![0, 6, 11, 1])]),
            true,
        );
        assert_eq!(store.line_tokens(0), vec![(6, 11, 1)]);
    }

    #[test]
    fn set_partial_removing_all_tokens_of_piece_starting_after_line_0() {
        let mut store = SparseTokensStore::new();
        store.set(
            Some(vec![SparseMultilineTokens::create(4, vec![0, 5, 10, 1])]),
            false,
        );
        assert!(!store.is_empty());
        store.set_partial(monaco_range(5, 1, 5, 20), vec![]);
        assert!(
            store.is_empty(),
            "store should be empty after setPartial removes all tokens"
        );
    }

    #[test]
    fn set_partial_split_with_empty_first_piece() {
        let mut store = SparseTokensStore::new();
        store.set(
            Some(vec![SparseMultilineTokens::create(0, vec![10, 5, 10, 1])]),
            false,
        );
        store.set_partial(monaco_range(1, 1, 5, 1), vec![]);
        assert!(
            !store.is_empty(),
            "store should still have the token on line 11"
        );
        assert_eq!(store.line_tokens(10), vec![(5, 10, 1)]);
        assert_eq!(store.pieces()[0].start_line(), 10);
        assert_eq!(store.pieces()[0].end_line(), 10);
    }

    /// The TS test checks that `addSparseTokens` (not ported) never produces
    /// backward end offsets for overlapping semantic tokens. The store itself
    /// returns the overlapping tokens unchanged; keeping end offsets monotonic
    /// is the editor's merge's job (see `extract_state` above for the rule).
    #[test]
    fn overlapping_semantic_tokens_are_returned_as_is() {
        let mut store = SparseTokensStore::new();
        store.set(
            Some(vec![SparseMultilineTokens::create(
                0,
                vec![
                    0, 0, 1, 1, //
                    0, 1, 2, 2, //
                    0, 2, 3, 1, //
                    0, 3, 5, 2, //
                    0, 4, 5, 1,
                ],
            )]),
            true,
        );
        assert_eq!(
            store.line_tokens(0),
            vec![(0, 1, 1), (1, 2, 2), (2, 3, 1), (3, 5, 2), (4, 5, 1)]
        );
    }

    #[test]
    fn piece_with_start_0_and_end_minus_1_after_encompassing_deletion() {
        let mut store = SparseTokensStore::new();
        let piece = SparseMultilineTokens::create(4, vec![0, 0, 5, 1, 5, 0, 5, 2]);
        assert_eq!(piece.start_line(), 4);
        assert_eq!(piece.end_line(), 9);
        assert!(!piece.is_empty());
        store.set(Some(vec![piece.clone()]), false);

        let mut piece = piece;
        piece.accept_edit(pt(0, 0)..pt(19, 0), 0, 0, 0, 0);
        assert!(piece.is_empty());
        assert_eq!((piece.start_line(), piece.end_line()), (0, -1));

        store.accept_edit(pt(0, 0)..pt(19, 0), 0, 0, 0, 0);
        assert!(
            store.is_empty(),
            "store should be empty after all tokens are deleted by an encompassing edit"
        );
    }

    // ---- semanticTokensProviderStyling.test.ts ----

    /// Mirrors the TS tests' theme: token type `stN` gets foreground `N` and
    /// types outside the legend get `NO_STYLING`; foreground 0 with no font
    /// style is also `NO_STYLING`. The style id is the type index.
    fn legend_style(type_count: u32) -> impl FnMut(u32, u32) -> Option<u32> {
        move |token_type, _| (token_type != 0 && token_type < type_count).then_some(token_type)
    }

    fn area_tokens(area: &SparseMultilineTokens) -> (i64, Vec<u32>) {
        (area.start_line(), area.tokens.tokens.clone())
    }

    #[test]
    fn issue_134973_invalid_semantic_tokens_handled_better() {
        let data = [
            0, 13, 16, 1, 0, //
            1, 2, 6, 2, 0, //
            0, 7, 6, 3, 0, //
            0, 15, 8, 4, 0, //
            0, 17, 1, 5, 0, //
            0, 7, 5, 6, 0, //
            1, 12, 8, 7, 0, //
            0, 19, 5, 8, 0, //
            0, 7, 1, 9, 0, //
            0, 4294967294, 5, 10, 0,
        ];
        let result = to_multiline_tokens(&data, &mut legend_style(11));
        assert_eq!(result.len(), 1);
        assert_eq!(
            area_tokens(&result[0]),
            (
                0,
                vec![
                    0, 13, 29, 1, //
                    1, 2, 8, 2, //
                    1, 9, 15, 3, //
                    1, 24, 32, 4, //
                    1, 41, 42, 5, //
                    1, 48, 53, 6, //
                    2, 12, 20, 7, //
                    2, 31, 36, 8, //
                    2, 38, 39, 9,
                ]
            )
        );
    }

    #[test]
    fn issue_148651_negative_values_do_not_hang() {
        let data = [
            0, 0, 3, 0, 0, //
            0, 4, 2, 2, 0, //
            0, 2, 3, 8, 0, //
            0, 3, 1, 9, 0, //
            0, 1, 1, 10, 0, //
            0, 1, 4, 8, 0, //
            0, 4, 4294967292, 2, 0, //
            0, 4294967292, 4294967294, 8, 0, //
            0, 4294967294, 1, 9, 0, //
            0, 1, 1, 10, 0, //
            0, 1, 3, 8, 0, //
            0, 3, 4294967291, 8, 0, //
            0, 4294967291, 1, 9, 0, //
            0, 1, 1, 10, 0, //
            0, 1, 4, 8, 0,
        ];
        let result = to_multiline_tokens(&data, &mut legend_style(10));
        assert_eq!(result.len(), 1);
        assert_eq!(
            area_tokens(&result[0]),
            (
                0,
                vec![
                    0, 4, 6, 2, //
                    0, 6, 9, 8, //
                    0, 9, 10, 9, //
                    0, 11, 15, 8,
                ]
            )
        );
    }

    #[test]
    fn issue_149130_bracket_pair_colorization_freeze() {
        let data = [
            0, 11, 1, 1, 0, //
            0, 4, 1, 1, 0, //
            0, 4294967289, 1, 1, 0,
        ];
        let result = to_multiline_tokens(&data, &mut legend_style(6));
        assert_eq!(result.len(), 1);
        assert_eq!(
            area_tokens(&result[0]),
            (0, vec![0, 11, 12, 1, 0, 15, 16, 1])
        );
    }

    // ---- Additional direct tests ----

    #[test]
    fn decoding_splits_areas_at_line_boundaries() {
        // 1000 tokens, two per line: areas of 400 tokens (200 lines) each.
        let mut data = Vec::new();
        for i in 0..1000u32 {
            let delta_line = u32::from(i % 2 == 0 && i != 0);
            let delta_start = if i % 2 == 0 { 0 } else { 5 };
            data.extend_from_slice(&[delta_line, delta_start, 3, 1, 0]);
        }
        let result = to_multiline_tokens(&data, &mut |_, _| Some(7));
        let shape: Vec<_> = result
            .iter()
            .map(|a| (a.start_line(), a.end_line(), a.tokens.token_count()))
            .collect();
        assert_eq!(shape, vec![(0, 199, 400), (200, 399, 400), (400, 499, 200)]);

        // Odd boundary: tokens 399 and 400 share a line, so the first area ends
        // early (399 tokens) rather than splitting the line.
        let mut data = vec![0, 0, 1, 1, 0];
        for i in 1..1000u32 {
            let delta_line = u32::from(i % 2 == 1);
            let delta_start = if i % 2 == 1 { 0 } else { 5 };
            data.extend_from_slice(&[delta_line, delta_start, 3, 1, 0]);
        }
        let result = to_multiline_tokens(&data, &mut |_, _| Some(7));
        let counts: Vec<_> = result.iter().map(|a| a.tokens.token_count()).collect();
        assert_eq!(counts, vec![399, 400, 201]);
    }

    #[test]
    fn decoding_keeps_a_huge_line_in_one_area() {
        // 500 tokens on one line followed by one on the next line.
        let mut data = Vec::new();
        for _ in 0..500 {
            data.extend_from_slice(&[0, 2, 1, 1, 0]);
        }
        data.extend_from_slice(&[1, 0, 1, 1, 0]);
        let result = to_multiline_tokens(&data, &mut |_, _| Some(1));
        let shape: Vec<_> = result
            .iter()
            .map(|a| (a.start_line(), a.tokens.token_count()))
            .collect();
        assert_eq!(shape, vec![(0, 500), (1, 1)]);
    }

    #[test]
    fn decoding_unstyled_area_is_empty() {
        let result = to_multiline_tokens(&[3, 0, 4, 1, 0], &mut |_, _| None);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_empty());
    }

    #[test]
    fn typing_word_char_at_token_end_extends_it() {
        let mut t = SparseMultilineTokens::create(2, vec![0, 4, 7, 1]);
        t.apply_edit(pt(2, 7)..pt(2, 7), "x");
        assert_eq!(t.line_tokens(2), Some(vec![(4, 8, 1)]));
        // A non-word character does not extend it.
        t.apply_edit(pt(2, 8)..pt(2, 8), ".");
        assert_eq!(t.line_tokens(2), Some(vec![(4, 8, 1)]));
        // A word char at the token start keeps the token where it is (Monaco
        // "grows" it by keeping its start; the end is not moved).
        t.apply_edit(pt(2, 4)..pt(2, 4), "a");
        assert_eq!(t.line_tokens(2), Some(vec![(4, 8, 1)]));
        // Anything else at the start moves it.
        t.apply_edit(pt(2, 4)..pt(2, 4), "  ");
        assert_eq!(t.line_tokens(2), Some(vec![(6, 10, 1)]));
        // Typing inside expands it.
        t.apply_edit(pt(2, 7)..pt(2, 7), "--");
        assert_eq!(t.line_tokens(2), Some(vec![(6, 12, 1)]));
        // A newline inside cuts it off.
        t.apply_edit(pt(2, 8)..pt(2, 8), "\n");
        assert_eq!(t.line_tokens(2), Some(vec![(6, 8, 1)]));
        assert_eq!(t.end_line(), 2);
    }

    #[test]
    fn edits_before_area_shift_lines() {
        let mut t = SparseMultilineTokens::create(5, vec![0, 1, 2, 1, 2, 3, 4, 2]);
        t.apply_edit(pt(1, 0)..pt(1, 0), "a\nb\n");
        assert_eq!((t.start_line(), t.end_line()), (7, 9));
        t.apply_edit(pt(0, 3)..pt(3, 1), "");
        assert_eq!((t.start_line(), t.end_line()), (4, 6));
        // Edits after the area are ignored.
        t.apply_edit(pt(7, 0)..pt(9, 0), "zz\n");
        assert_eq!((t.start_line(), t.end_line()), (4, 6));
        assert_eq!(t.line_tokens(6), Some(vec![(3, 4, 2)]));
    }

    #[test]
    fn deletion_starting_before_area_shifts_first_line_tokens() {
        // Token on the area's first line, after the deletion's end column.
        let mut t = SparseMultilineTokens::create(3, vec![0, 6, 9, 1, 1, 0, 2, 2]);
        // Delete from (1, 4) to (3, 2): the token lands on line 1 at 4 + (6 - 2).
        t.apply_edit(pt(1, 4)..pt(3, 2), "");
        assert_eq!(t.start_line(), 1);
        assert_eq!(t.line_tokens(1), Some(vec![(8, 11, 1)]));
        assert_eq!(t.line_tokens(2), Some(vec![(0, 2, 2)]));
    }

    #[test]
    fn remove_tokens_and_split() {
        let t = SparseMultilineTokens::create(
            10,
            vec![
                0, 0, 2, 1, //
                1, 0, 2, 2, //
                2, 4, 6, 3, //
                4, 0, 1, 4,
            ],
        );
        let (a, b) = t.split(pt(11, 1)..pt(12, 4));
        assert_eq!((a.start_line(), a.end_line()), (10, 10));
        assert_eq!(a.line_tokens(10), Some(vec![(0, 2, 1)]));
        assert_eq!((b.start_line(), b.end_line()), (14, 14));
        assert_eq!(b.line_tokens(14), Some(vec![(0, 1, 4)]));

        let mut r = t.clone();
        r.remove_tokens(pt(0, 0)..pt(11, 5));
        assert_eq!((r.start_line(), r.end_line()), (12, 14));
        assert_eq!(r.range(), Some(pt(12, 4)..pt(14, 1)));
    }

    #[test]
    fn line_tokens_binary_search() {
        let t = SparseMultilineTokens::create(
            0,
            vec![
                0, 0, 1, 1, //
                0, 2, 3, 2, //
                2, 0, 1, 3, //
                2, 1, 2, 4, //
                2, 2, 3, 5, //
                5, 0, 1, 6,
            ],
        );
        assert_eq!(t.line_tokens(0), Some(vec![(0, 1, 1), (2, 3, 2)]));
        assert_eq!(t.line_tokens(1), None);
        assert_eq!(
            t.line_tokens(2),
            Some(vec![(0, 1, 3), (1, 2, 4), (2, 3, 5)])
        );
        assert_eq!(t.line_tokens(5), Some(vec![(0, 1, 6)]));
        assert_eq!(t.line_tokens(6), None);
    }

    #[test]
    fn store_set_and_line_tokens_across_pieces() {
        let mut store = SparseTokensStore::new();
        assert!(!store.is_complete());
        assert!(store.line_tokens(0).is_empty());
        store.set(
            Some(vec![
                SparseMultilineTokens::create(0, vec![0, 0, 1, 1, 2, 0, 1, 2]),
                SparseMultilineTokens::create(5, vec![0, 3, 4, 3]),
                SparseMultilineTokens::create(9, vec![0, 3, 4, 4]),
            ]),
            true,
        );
        assert!(store.is_complete());
        assert_eq!(store.line_tokens(2), vec![(0, 1, 2)]);
        assert!(store.line_tokens(3).is_empty());
        assert_eq!(store.line_tokens(5), vec![(3, 4, 3)]);
        assert_eq!(store.line_tokens(9), vec![(3, 4, 4)]);
        assert!(store.line_tokens(10).is_empty());

        let changed = store.set_partial(
            pt(4, 0)..pt(6, 0),
            vec![SparseMultilineTokens::create(5, vec![0, 0, 2, 9])],
        );
        assert_eq!(changed, pt(4, 0)..pt(6, 0));
        assert_eq!(store.line_tokens(5), vec![(0, 2, 9)]);
        assert_eq!(store.pieces().len(), 3);

        store.clear();
        assert!(store.is_empty());
        assert!(!store.is_complete());
    }

    #[test]
    fn count_eol_matches_monaco() {
        assert_eq!(count_eol(""), (0, 0, 0));
        assert_eq!(count_eol("abc"), (0, 3, 3));
        assert_eq!(count_eol("ab\r\ncde\rf\n"), (3, 2, 0));
        assert_eq!(count_eol("\u{1F600}\nxy"), (1, 2, 2));
    }
}
