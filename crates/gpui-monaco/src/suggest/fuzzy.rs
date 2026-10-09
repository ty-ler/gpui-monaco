//! Fuzzy matching of a typed pattern against a candidate word.
//!
//! Ports the `fuzzyScore` region of Monaco's `src/vs/base/common/filters.ts`
//! (MIT): `fuzzyScore`, `FuzzyScore`, `FuzzyScoreOptions`, `createMatches`,
//! `anyScore`, `isPatternInWord`, the graceful variants
//! (`fuzzyScoreGraceful`, `fuzzyScoreGracefulAggressive`) and the helpers they
//! use, plus `isEmojiImprecise` from `src/vs/base/common/strings.ts`.
//!
//! Monaco works on JavaScript strings, i.e. UTF-16 code units; so does this
//! port, so scores, positions and match ranges are identical to Monaco's.
//! Lower-casing is done per code point, keeping the original character where
//! its lower-case form has a different UTF-16 length (JavaScript's
//! `toLowerCase` would change the string length there and misalign Monaco's
//! own tables).

use std::ops::Range;

/// A fuzzy match: its score (higher is better) and the matched ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyScore {
    pub score: i32,
    /// Matched char ranges in the *word* (target), in UTF-16 code units, like
    /// Monaco's `createMatches`.
    pub matches: Vec<Range<usize>>,
}

impl FuzzyScore {
    /// Monaco `FuzzyScore.Default`: no matches and a score of `-100`, out of
    /// the range real matches get.
    pub const DEFAULT_SCORE: i32 = -100;

    pub fn default_score() -> Self {
        Self {
            score: Self::DEFAULT_SCORE,
            matches: Vec::new(),
        }
    }

    /// Monaco `FuzzyScore.isDefault`.
    pub fn is_default(&self) -> bool {
        self.score == Self::DEFAULT_SCORE && self.matches.is_empty()
    }
}

/// Monaco `FuzzyScoreOptions`. Note that `Default` is all-false; Monaco's
/// `FuzzyScoreOptions.default` is [`FuzzyScoreOptions::MONACO_DEFAULT`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FuzzyScoreOptions {
    /// Accept matches whose first character isn't at a word start, camel-case
    /// hump or separator.
    pub first_match_can_be_weak: bool,
    /// Boost a pattern that matches the whole word.
    pub boost_full_match: bool,
}

impl FuzzyScoreOptions {
    /// Monaco `FuzzyScoreOptions.default`.
    pub const MONACO_DEFAULT: Self = Self {
        first_match_can_be_weak: false,
        boost_full_match: true,
    };
}

/// A string as UTF-16 code units together with its lower-cased form (Monaco
/// passes `word` and `wordLow` separately to avoid re-lower-casing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Utf16Text {
    pub units: Vec<u16>,
    pub low: Vec<u16>,
}

impl Utf16Text {
    pub fn new(text: &str) -> Self {
        let mut units = Vec::with_capacity(text.len());
        let mut low = Vec::with_capacity(text.len());
        let mut buf = [0u16; 2];
        for ch in text.chars() {
            let original = ch.encode_utf16(&mut buf).len();
            units.extend_from_slice(&buf[..original]);
            let mut lower = ch.to_lowercase();
            let lowered = match (lower.next(), lower.next()) {
                (Some(l), None) if l.len_utf16() == original => l,
                _ => ch,
            };
            low.extend_from_slice(lowered.encode_utf16(&mut buf));
        }
        Self { units, low }
    }

    pub fn len(&self) -> usize {
        self.units.len()
    }
}

/// A Monaco `FuzzyScorer`: `(pattern, patternPos, word, wordPos, options)`.
pub(crate) type Scorer =
    fn(&Utf16Text, usize, &Utf16Text, usize, FuzzyScoreOptions) -> Option<FuzzyScore>;

// ----- public string API -------------------------------------------------------

/// Monaco `fuzzyScore(pattern, lowPattern, 0, word, lowWord, 0, options)`.
pub fn fuzzy_score(pattern: &str, word: &str, options: FuzzyScoreOptions) -> Option<FuzzyScore> {
    fuzzy_score_at(pattern, 0, word, 0, options)
}

/// Monaco `fuzzyScore` starting at `pattern_pos` in the pattern and
/// `word_pos` in the word (both in UTF-16 code units).
pub fn fuzzy_score_at(
    pattern: &str,
    pattern_pos: usize,
    word: &str,
    word_pos: usize,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    score_utf16(
        &Utf16Text::new(pattern),
        pattern_pos,
        &Utf16Text::new(word),
        word_pos,
        options,
    )
}

/// Monaco `fuzzyScoreGraceful`: [`fuzzy_score`], falling back to patterns
/// with two neighbouring characters swapped when it doesn't match.
pub fn fuzzy_score_graceful(
    pattern: &str,
    word: &str,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    fuzzy_score_graceful_at(pattern, 0, word, 0, options)
}

/// [`fuzzy_score_graceful`] with start positions, like [`fuzzy_score_at`].
pub fn fuzzy_score_graceful_at(
    pattern: &str,
    pattern_pos: usize,
    word: &str,
    word_pos: usize,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    score_graceful_utf16(
        &Utf16Text::new(pattern),
        pattern_pos,
        &Utf16Text::new(word),
        word_pos,
        options,
    )
}

/// Monaco `fuzzyScoreGracefulAggressive`: always tries the swapped-neighbour
/// permutations too and keeps the best (penalized) result.
pub fn fuzzy_score_graceful_aggressive(
    pattern: &str,
    word: &str,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    fuzzy_score_graceful_aggressive_at(pattern, 0, word, 0, options)
}

/// [`fuzzy_score_graceful_aggressive`] with start positions, like
/// [`fuzzy_score_at`].
pub fn fuzzy_score_graceful_aggressive_at(
    pattern: &str,
    pattern_pos: usize,
    word: &str,
    word_pos: usize,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    score_graceful_aggressive_utf16(
        &Utf16Text::new(pattern),
        pattern_pos,
        &Utf16Text::new(word),
        word_pos,
        options,
    )
}

/// Monaco `anyScore`: the first match of the pattern (or a suffix of it,
/// dropping up to 13 leading characters) in the word, accepting weak first
/// matches; a zero score with no matches when nothing matches.
pub fn any_score(pattern: &str, pattern_pos: usize, word: &str, word_pos: usize) -> FuzzyScore {
    any_score_utf16(
        &Utf16Text::new(pattern),
        pattern_pos,
        &Utf16Text::new(word),
        word_pos,
    )
}

// ----- UTF-16 implementation -----------------------------------------------------

pub(crate) fn any_score_utf16(
    pattern: &Utf16Text,
    mut pattern_pos: usize,
    word: &Utf16Text,
    word_pos: usize,
) -> FuzzyScore {
    let max = pattern.len().min(13);
    let options = FuzzyScoreOptions {
        first_match_can_be_weak: true,
        boost_full_match: true,
    };
    while pattern_pos < max {
        if let Some(result) = score_utf16(pattern, pattern_pos, word, word_pos, options) {
            return result;
        }
        pattern_pos += 1;
    }
    FuzzyScore {
        score: 0,
        matches: Vec::new(),
    }
}

pub(crate) fn score_graceful_utf16(
    pattern: &Utf16Text,
    pattern_pos: usize,
    word: &Utf16Text,
    word_pos: usize,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    score_with_permutations(pattern, pattern_pos, word, word_pos, false, options)
}

pub(crate) fn score_graceful_aggressive_utf16(
    pattern: &Utf16Text,
    pattern_pos: usize,
    word: &Utf16Text,
    word_pos: usize,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    score_with_permutations(pattern, pattern_pos, word, word_pos, true, options)
}

fn score_with_permutations(
    pattern: &Utf16Text,
    pattern_pos: usize,
    word: &Utf16Text,
    word_pos: usize,
    aggressive: bool,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    let mut top = score_utf16(pattern, pattern_pos, word, word_pos, options);
    if top.is_some() && !aggressive {
        // When the original pattern matches, return it unless aggressive,
        // which looks for a better alignment, e.g. `cno` -> `^c^o^nsole`.
        return top;
    }

    if pattern.len() >= 3 {
        // Try a few (max 7) permutations that swap neighbouring characters,
        // e.g. `cnoso` becomes `conso`, `cnsoo`, `cnoos`.
        let tries = 7.min(pattern.len() - 1);
        for moving in pattern_pos + 1..tries {
            let Some(new_pattern) = next_typo_permutation(pattern, moving) else {
                continue;
            };
            if let Some(mut candidate) =
                score_utf16(&new_pattern, pattern_pos, word, word_pos, options)
            {
                candidate.score -= 3; // permutation penalty
                if top.as_ref().is_none_or(|top| candidate.score > top.score) {
                    top = Some(candidate);
                }
            }
        }
    }
    top
}

fn next_typo_permutation(pattern: &Utf16Text, pos: usize) -> Option<Utf16Text> {
    if pos + 1 >= pattern.len() {
        return None;
    }
    let (a, b) = (pattern.units[pos], pattern.units[pos + 1]);
    if a == b {
        return None;
    }
    let mut units = pattern.units.clone();
    units.swap(pos, pos + 1);
    // Monaco lower-cases the permuted string again; swapping the lower-cased
    // units is the same thing.
    let mut low = pattern.low.clone();
    low.swap(pos, pos + 1);
    Some(Utf16Text { units, low })
}

const MAX_LEN: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arrow {
    None,
    Diag,
    Left,
    LeftLeft,
}

/// Row-major `(rows) x (cols)` table.
struct Table<T> {
    cols: usize,
    cells: Vec<T>,
}

impl<T: Copy> Table<T> {
    fn new(rows: usize, cols: usize, fill: T) -> Self {
        Self {
            cols,
            cells: vec![fill; rows * cols],
        }
    }

    fn get(&self, row: usize, col: usize) -> T {
        self.cells[row * self.cols + col]
    }

    fn set(&mut self, row: usize, col: usize, value: T) {
        self.cells[row * self.cols + col] = value;
    }
}

/// Monaco `isPatternInWord`: whether the pattern's characters occur in order
/// in the word. Fills `min_word_match_pos` when given.
fn is_pattern_in_word(
    pattern_low: &[u16],
    mut pattern_pos: usize,
    pattern_len: usize,
    word_low: &[u16],
    mut word_pos: usize,
    word_len: usize,
    mut min_word_match_pos: Option<&mut [usize]>,
) -> bool {
    while pattern_pos < pattern_len && word_pos < word_len {
        if pattern_low[pattern_pos] == word_low[word_pos] {
            if let Some(min) = min_word_match_pos.as_deref_mut() {
                min[pattern_pos] = word_pos;
            }
            pattern_pos += 1;
        }
        word_pos += 1;
    }
    pattern_pos == pattern_len
}

/// Monaco `fuzzyScore` on UTF-16 text.
pub(crate) fn score_utf16(
    pattern: &Utf16Text,
    pattern_start: usize,
    word: &Utf16Text,
    word_start: usize,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    let pattern_len = pattern.len().min(MAX_LEN);
    let word_len = word.len().min(MAX_LEN);

    if pattern_start >= pattern_len
        || word_start >= word_len
        || (pattern_len - pattern_start) > (word_len - word_start)
    {
        return None;
    }

    let pattern_low = &pattern.low[..];
    let word_low = &word.low[..];

    // Bail out early unless the pattern's characters occur (in order) in the
    // word; this also fills in the min matching word position per pattern
    // position.
    let mut min_word_match_pos = vec![0usize; pattern_len];
    if !is_pattern_in_word(
        pattern_low,
        pattern_start,
        pattern_len,
        word_low,
        word_start,
        word_len,
        Some(&mut min_word_match_pos),
    ) {
        return None;
    }

    // The max matching word position for each pattern position.
    let mut max_word_match_pos = vec![0usize; pattern_len];
    {
        let mut pattern_pos = pattern_len as isize - 1;
        let mut word_pos = word_len as isize - 1;
        while pattern_pos >= pattern_start as isize && word_pos >= word_start as isize {
            if pattern_low[pattern_pos as usize] == word_low[word_pos as usize] {
                max_word_match_pos[pattern_pos as usize] = word_pos as usize;
                pattern_pos -= 1;
            }
            word_pos -= 1;
        }
    }

    let rows = pattern_len - pattern_start + 1;
    let cols = word_len - word_start + 1;
    let mut table = Table::new(rows, cols, 0i32);
    let mut diag = Table::new(rows, cols, 0i32);
    let mut arrows = Table::new(rows, cols, Arrow::None);

    let mut has_strong_first_match = false;

    let mut row = 1;
    let mut column = 1;
    for pattern_pos in pattern_start..pattern_len {
        // Reduce the search space to possible matching word positions and to
        // positions reachable from the next row.
        let min_pos = min_word_match_pos[pattern_pos];
        let max_pos = max_word_match_pos[pattern_pos];
        let next_max_pos = if pattern_pos + 1 < pattern_len {
            max_word_match_pos[pattern_pos + 1]
        } else {
            word_len
        };

        column = min_pos - word_start + 1;
        for word_pos in min_pos..next_max_pos {
            let mut score = None;
            if word_pos <= max_pos {
                score = do_score(
                    pattern,
                    pattern_pos,
                    pattern_start,
                    word,
                    word_pos,
                    word_len,
                    word_start,
                    diag.get(row - 1, column - 1) == 0,
                    &mut has_strong_first_match,
                );
            }

            let diag_score = score.map(|score| score + table.get(row - 1, column - 1));

            let can_come_left = word_pos > min_pos;
            let left_score = if can_come_left {
                // penalty for a gap start
                table.get(row, column - 1) + if diag.get(row, column - 1) > 0 { -5 } else { 0 }
            } else {
                0
            };

            let can_come_left_left = word_pos > min_pos + 1 && diag.get(row, column - 1) > 0;
            let left_left_score = if can_come_left_left {
                table.get(row, column - 2) + if diag.get(row, column - 2) > 0 { -5 } else { 0 }
            } else {
                0
            };

            if can_come_left_left
                && (!can_come_left || left_left_score >= left_score)
                && diag_score.is_none_or(|d| left_left_score >= d)
            {
                // Prefer jumping over a diagonal: the match is earlier in the word.
                table.set(row, column, left_left_score);
                arrows.set(row, column, Arrow::LeftLeft);
                diag.set(row, column, 0);
            } else if can_come_left && diag_score.is_none_or(|d| left_score >= d) {
                // Prefer left: the match is earlier in the word.
                table.set(row, column, left_score);
                arrows.set(row, column, Arrow::Left);
                diag.set(row, column, 0);
            } else if let Some(diag_score) = diag_score {
                table.set(row, column, diag_score);
                arrows.set(row, column, Arrow::Diag);
                diag.set(row, column, diag.get(row - 1, column - 1) + 1);
            } else {
                unreachable!("fuzzy score: no path into cell");
            }
            column += 1;
        }
        row += 1;
    }

    if !has_strong_first_match && !options.first_match_can_be_weak {
        return None;
    }

    row -= 1;
    column -= 1;

    let mut score = table.get(row, column);
    // Matched columns (1-based, relative to `word_start`), last first.
    let mut matched: Vec<usize> = Vec::with_capacity(rows);

    let mut backwards_diag_length = 0;
    let mut max_match_column = 0usize;

    while row >= 1 {
        // Find the column where we go diagonally up.
        let mut diag_column = column as isize;
        loop {
            match arrows.get(row, diag_column as usize) {
                Arrow::LeftLeft => diag_column -= 2,
                Arrow::Left => diag_column -= 1,
                _ => break,
            }
            if diag_column < 1 {
                break;
            }
        }
        let mut diag_column = diag_column as usize;

        // Overturn the "forwards" decision if keeping the "backwards"
        // diagonal gives a better (longer contiguous) match.
        if backwards_diag_length > 1
            && pattern_low[pattern_start + row - 1] == word_low[word_start + column - 1]
            && !is_upper_case_at_pos(diag_column + word_start - 1, word)
            && backwards_diag_length + 1 > diag.get(row, diag_column)
        {
            diag_column = column;
        }

        if diag_column == column {
            backwards_diag_length += 1;
        } else {
            backwards_diag_length = 1;
        }

        if max_match_column == 0 {
            max_match_column = diag_column;
        }

        row -= 1;
        column = diag_column - 1;
        matched.push(column);
    }

    if word_len - word_start == pattern_len && options.boost_full_match {
        // The word matches the pattern with all characters: boost it.
        score += 2;
    }

    // One penalty point per skipped character in the word.
    score -= max_match_column as i32 - pattern_len as i32;

    Some(FuzzyScore {
        score,
        matches: create_matches(word_start, &matched),
    })
}

/// Monaco `createMatches`: `matched` holds positions relative to
/// `word_start`, last one first.
fn create_matches(word_start: usize, matched: &[usize]) -> Vec<Range<usize>> {
    let mut result: Vec<Range<usize>> = Vec::new();
    for &column in matched.iter().rev() {
        let pos = column + word_start;
        match result.last_mut() {
            Some(last) if last.end == pos => last.end = pos + 1,
            _ => result.push(pos..pos + 1),
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn do_score(
    pattern: &Utf16Text,
    pattern_pos: usize,
    pattern_start: usize,
    word: &Utf16Text,
    word_pos: usize,
    word_len: usize,
    word_start: usize,
    new_match_start: bool,
    out_first_match_strong: &mut bool,
) -> Option<i32> {
    if pattern.low[pattern_pos] != word.low[word_pos] {
        return None;
    }
    let word_low = &word.low[..];
    let prev = word_pos as isize - 1;

    let mut score = 1;
    let mut is_gap_location = false;
    if word_pos == pattern_pos - pattern_start {
        // common prefix: `foobar <-> foobaz`
        score = if pattern.units[pattern_pos] == word.units[word_pos] {
            7
        } else {
            5
        };
    } else if is_upper_case_at_pos(word_pos, word)
        && (word_pos == 0 || !is_upper_case_at_pos(word_pos - 1, word))
    {
        // hitting upper-case: `foo <-> forOthers`
        score = if pattern.units[pattern_pos] == word.units[word_pos] {
            7
        } else {
            5
        };
        is_gap_location = true;
    } else if is_separator_at_pos(word_low, word_pos as isize)
        && (word_pos == 0 || !is_separator_at_pos(word_low, prev))
    {
        // hitting a separator: `. <-> foo.bar`
        score = 5;
    } else if is_separator_at_pos(word_low, prev) || is_whitespace_at_pos(word_low, prev) {
        // post separator: `foo <-> bar_foo`
        score = 5;
        is_gap_location = true;
    }

    if score > 1 && pattern_pos == pattern_start {
        *out_first_match_strong = true;
    }

    if !is_gap_location {
        is_gap_location = is_upper_case_at_pos(word_pos, word)
            || is_separator_at_pos(word_low, prev)
            || is_whitespace_at_pos(word_low, prev);
    }

    if pattern_pos == pattern_start {
        // First pattern character not at the word start: penalize the gap
        // before it.
        if word_pos > word_start {
            score -= if is_gap_location { 3 } else { 5 };
        }
    } else if new_match_start {
        // Start of a new match (a gap before this location).
        score += if is_gap_location { 2 } else { 0 };
    } else {
        // Part of a contiguous match: a slight bonus unless it'd be a
        // preferred gap location.
        score += if is_gap_location { 0 } else { 1 };
    }

    if word_pos + 1 == word_len {
        // Pretend there's a gap after the last character so matching it
        // isn't unfairly favoured.
        score -= if is_gap_location { 3 } else { 5 };
    }

    Some(score)
}

fn is_upper_case_at_pos(pos: usize, word: &Utf16Text) -> bool {
    word.units[pos] != word.low[pos]
}

/// Monaco `isSeparatorAtPos` (reads a code point like `codePointAt`).
fn is_separator_at_pos(value: &[u16], index: isize) -> bool {
    if index < 0 || index as usize >= value.len() {
        return false;
    }
    let index = index as usize;
    let unit = value[index];
    let code = if (0xD800..0xDC00).contains(&unit) {
        match value.get(index + 1) {
            Some(&low) if (0xDC00..0xE000).contains(&low) => {
                0x10000 + (((unit as u32) - 0xD800) << 10) + ((low as u32) - 0xDC00)
            }
            _ => unit as u32,
        }
    } else {
        unit as u32
    };
    matches!(
        code,
        0x5F // _
        | 0x2D // -
        | 0x2E // .
        | 0x20 // space
        | 0x2F // /
        | 0x5C // \
        | 0x27 // '
        | 0x22 // "
        | 0x3A // :
        | 0x24 // $
        | 0x3C // <
        | 0x3E // >
        | 0x28 // (
        | 0x29 // )
        | 0x5B // [
        | 0x5D // ]
        | 0x7B // {
        | 0x7D // }
    ) || is_emoji_imprecise(code)
}

fn is_whitespace_at_pos(value: &[u16], index: isize) -> bool {
    if index < 0 || index as usize >= value.len() {
        return false;
    }
    matches!(value[index as usize], 0x20 | 0x09)
}

/// Monaco `strings.isEmojiImprecise`.
fn is_emoji_imprecise(x: u32) -> bool {
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
