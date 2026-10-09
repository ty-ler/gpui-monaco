//! Word navigation and word deletion.
//!
//! Ports Monaco's `src/vs/editor/common/cursor/cursorWordOperations.ts` and
//! `src/vs/editor/common/core/wordCharacterClassifier.ts` (MIT). Lines are
//! classified per UTF-16 code unit exactly like Monaco's `charCodeAt` loops.
//! `Intl.Segmenter`-based word segmentation (`wordSegmenterLocales`) is not
//! ported.

use std::collections::HashMap;
use std::ops::Range;

use super::{
    CursorCollection, CursorConfig, CursorModel, Position, SelectionStartKind, SingleCursorState,
    USUAL_WORD_SEPARATORS, last_row, line_units, strings,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WordCharClass {
    Regular,
    Whitespace,
    WordSeparator,
}

/// Monaco `WordCharacterClassifier`.
pub(crate) struct WordClassifier {
    ascii: [WordCharClass; 256],
    other: HashMap<u16, WordCharClass>,
}

impl WordClassifier {
    pub(crate) fn new(word_separators: &str) -> Self {
        let mut this = Self {
            ascii: [WordCharClass::Regular; 256],
            other: HashMap::new(),
        };
        for c in word_separators.encode_utf16() {
            this.set(c, WordCharClass::WordSeparator);
        }
        this.set(b' ' as u16, WordCharClass::Whitespace);
        this.set(b'\t' as u16, WordCharClass::Whitespace);
        this
    }

    fn set(&mut self, c: u16, class: WordCharClass) {
        if c < 256 {
            self.ascii[c as usize] = class;
        } else {
            self.other.insert(c, class);
        }
    }

    pub(crate) fn get(&self, c: u16) -> WordCharClass {
        if c < 256 {
            self.ascii[c as usize]
        } else {
            self.other
                .get(&c)
                .copied()
                .unwrap_or(WordCharClass::Regular)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WordType {
    None,
    Regular,
    Separator,
}

/// Which word boundaries word navigation stops at (Monaco
/// `WordNavigationType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordNavigationType {
    /// Stop at word starts (`cursorWordStartLeft`/`Right`).
    WordStart,
    /// Stop at word starts, skipping single separators glued to a word
    /// (`cursorWordLeft`, Monaco's default Ctrl/Alt+Left).
    WordStartFast,
    /// Stop at word ends (`cursorWordEndLeft`/`Right`, `cursorWordRight`).
    WordEnd,
    /// Stop at starts of non-separator words, using the default separators
    /// (`cursorWordAccessibilityLeft`/`Right`).
    WordAccessibility,
}

#[derive(Clone, Copy, Debug)]
struct FoundWord {
    /// UTF-16 index where the word starts.
    start: u32,
    /// UTF-16 index where the word ends.
    end: u32,
    word_type: WordType,
    /// The class of the character that ended the search.
    next_char_class: WordCharClass,
}

fn find_previous_word_on_line(
    classifier: &WordClassifier,
    model: &dyn CursorModel,
    position: Position,
) -> Option<FoundWord> {
    let line = line_units(model, position.row);
    do_find_previous_word_on_line(&line, classifier, position.column)
}

fn do_find_previous_word_on_line(
    line: &[u16],
    classifier: &WordClassifier,
    column: u32,
) -> Option<FoundWord> {
    let mut word_type = WordType::None;
    let mut ch_index = column.min(line.len() as u32) as i64 - 1;
    while ch_index >= 0 {
        let ch_class = classifier.get(line[ch_index as usize]);
        let next = ch_index as u32 + 1;
        match ch_class {
            WordCharClass::Regular => {
                if word_type == WordType::Separator {
                    return Some(create_word(
                        word_type,
                        ch_class,
                        next,
                        find_end_of_word(line, classifier, word_type, next),
                    ));
                }
                word_type = WordType::Regular;
            }
            WordCharClass::WordSeparator => {
                if word_type == WordType::Regular {
                    return Some(create_word(
                        word_type,
                        ch_class,
                        next,
                        find_end_of_word(line, classifier, word_type, next),
                    ));
                }
                word_type = WordType::Separator;
            }
            WordCharClass::Whitespace => {
                if word_type != WordType::None {
                    return Some(create_word(
                        word_type,
                        ch_class,
                        next,
                        find_end_of_word(line, classifier, word_type, next),
                    ));
                }
            }
        }
        ch_index -= 1;
    }
    if word_type != WordType::None {
        return Some(create_word(
            word_type,
            WordCharClass::Whitespace,
            0,
            find_end_of_word(line, classifier, word_type, 0),
        ));
    }
    None
}

fn create_word(
    word_type: WordType,
    next_char_class: WordCharClass,
    start: u32,
    end: u32,
) -> FoundWord {
    FoundWord {
        start,
        end,
        word_type,
        next_char_class,
    }
}

fn find_end_of_word(
    line: &[u16],
    classifier: &WordClassifier,
    word_type: WordType,
    start_index: u32,
) -> u32 {
    for (ch_index, &ch) in line.iter().enumerate().skip(start_index as usize) {
        let ch_class = classifier.get(ch);
        if ch_class == WordCharClass::Whitespace
            || (word_type == WordType::Regular && ch_class == WordCharClass::WordSeparator)
            || (word_type == WordType::Separator && ch_class == WordCharClass::Regular)
        {
            return ch_index as u32;
        }
    }
    line.len() as u32
}

fn find_next_word_on_line(
    classifier: &WordClassifier,
    model: &dyn CursorModel,
    position: Position,
) -> Option<FoundWord> {
    let line = line_units(model, position.row);
    do_find_next_word_on_line(&line, classifier, position.column)
}

fn do_find_next_word_on_line(
    line: &[u16],
    classifier: &WordClassifier,
    column: u32,
) -> Option<FoundWord> {
    let mut word_type = WordType::None;
    let len = line.len();
    for ch_index in column as usize..len {
        let ch_class = classifier.get(line[ch_index]);
        let make = |word_type| {
            create_word(
                word_type,
                ch_class,
                find_start_of_word(line, classifier, word_type, ch_index as i64 - 1),
                ch_index as u32,
            )
        };
        match ch_class {
            WordCharClass::Regular => {
                if word_type == WordType::Separator {
                    return Some(make(word_type));
                }
                word_type = WordType::Regular;
            }
            WordCharClass::WordSeparator => {
                if word_type == WordType::Regular {
                    return Some(make(word_type));
                }
                word_type = WordType::Separator;
            }
            WordCharClass::Whitespace => {
                if word_type != WordType::None {
                    return Some(make(word_type));
                }
            }
        }
    }
    if word_type != WordType::None {
        return Some(create_word(
            word_type,
            WordCharClass::Whitespace,
            find_start_of_word(line, classifier, word_type, len as i64 - 1),
            len as u32,
        ));
    }
    None
}

fn find_start_of_word(
    line: &[u16],
    classifier: &WordClassifier,
    word_type: WordType,
    start_index: i64,
) -> u32 {
    let mut ch_index = start_index;
    while ch_index >= 0 {
        let ch_class = classifier.get(line[ch_index as usize]);
        if ch_class == WordCharClass::Whitespace
            || (word_type == WordType::Regular && ch_class == WordCharClass::WordSeparator)
            || (word_type == WordType::Separator && ch_class == WordCharClass::Regular)
        {
            return (ch_index + 1) as u32;
        }
        ch_index -= 1;
    }
    0
}

/// Monaco `WordOperations.moveWordLeft`.
pub(crate) fn move_word_left(
    classifier: &WordClassifier,
    model: &dyn CursorModel,
    position: Position,
    nav: WordNavigationType,
    has_multicursor: bool,
) -> Position {
    let mut row = position.row;
    let mut column = position.column;
    if column == 0 && row > 0 {
        row -= 1;
        column = model.line_max_column(row);
    }
    let mut prev = find_previous_word_on_line(classifier, model, Position::new(row, column));
    match nav {
        WordNavigationType::WordStart => Position::new(row, prev.map_or(0, |w| w.start)),
        WordNavigationType::WordStartFast => {
            if let Some(w) = prev
                && !has_multicursor
                && w.word_type == WordType::Separator
                && w.end - w.start == 1
                && w.next_char_class == WordCharClass::Regular
            {
                // Skip over a word made up of one single separator and
                // followed by a regular character.
                prev = find_previous_word_on_line(classifier, model, Position::new(row, w.start));
            }
            Position::new(row, prev.map_or(0, |w| w.start))
        }
        WordNavigationType::WordAccessibility => {
            while let Some(w) = prev
                && w.word_type == WordType::Separator
            {
                prev = find_previous_word_on_line(classifier, model, Position::new(row, w.start));
            }
            Position::new(row, prev.map_or(0, |w| w.start))
        }
        WordNavigationType::WordEnd => {
            if let Some(w) = prev
                && column <= w.end
            {
                prev = find_previous_word_on_line(classifier, model, Position::new(row, w.start));
            }
            Position::new(row, prev.map_or(0, |w| w.end))
        }
    }
}

fn is_lower_ascii_letter(c: u16) -> bool {
    (b'a' as u16..=b'z' as u16).contains(&c)
}

fn is_upper_ascii_letter(c: u16) -> bool {
    (b'A' as u16..=b'Z' as u16).contains(&c)
}

fn is_ascii_digit(c: u16) -> bool {
    (b'0' as u16..=b'9' as u16).contains(&c)
}

const UNDERLINE: u16 = b'_' as u16;
const DASH: u16 = b'-' as u16;

/// Monaco `WordOperations._moveWordPartLeft`.
fn move_word_part_left_only(model: &dyn CursorModel, position: Position) -> Position {
    let row = position.row;
    if position.column == 0 {
        return if row > 0 {
            Position::new(row - 1, model.line_max_column(row - 1))
        } else {
            position
        };
    }
    let line = line_units(model, row);
    let len = line.len();
    let column = (position.column as usize).min(len);
    // `c` is a 0-based column; `left`/`right` are the characters around it.
    for c in (1..column).rev() {
        let left = line[c - 1];
        let right = line[c];
        if left == UNDERLINE && right != UNDERLINE {
            return Position::new(row, c as u32);
        }
        if left == DASH && right != DASH {
            return Position::new(row, c as u32);
        }
        if (is_lower_ascii_letter(left) || is_ascii_digit(left)) && is_upper_ascii_letter(right) {
            return Position::new(row, c as u32);
        }
        if is_upper_ascii_letter(left) && is_upper_ascii_letter(right) && c + 1 < len {
            let right_right = line[c + 1];
            if is_lower_ascii_letter(right_right) || is_ascii_digit(right_right) {
                return Position::new(row, c as u32);
            }
        }
    }
    Position::new(row, 0)
}

/// Monaco `WordOperations.moveWordRight`.
pub(crate) fn move_word_right(
    classifier: &WordClassifier,
    model: &dyn CursorModel,
    position: Position,
    nav: WordNavigationType,
) -> Position {
    let mut row = position.row;
    let mut column = position.column;
    let mut moved_down = false;
    if column == model.line_max_column(row) && row < last_row(model) {
        moved_down = true;
        row += 1;
        column = 0;
    }
    let mut next = find_next_word_on_line(classifier, model, Position::new(row, column));
    match nav {
        WordNavigationType::WordEnd => {
            if let Some(w) = next
                && w.word_type == WordType::Separator
                && w.end - w.start == 1
                && w.next_char_class == WordCharClass::Regular
            {
                // Skip over a word made up of one single separator and
                // followed by a regular character.
                next = find_next_word_on_line(classifier, model, Position::new(row, w.end));
            }
            column = next.map_or_else(|| model.line_max_column(row), |w| w.end);
        }
        WordNavigationType::WordAccessibility => {
            // Monaco pretends the cursor is before the first character after
            // moving down, so a word at column 0 is not skipped.
            let current: i64 = if moved_down { -1 } else { column as i64 };
            while let Some(w) = next
                && (w.word_type == WordType::Separator || (w.start as i64) <= current)
            {
                next = find_next_word_on_line(classifier, model, Position::new(row, w.end));
            }
            column = next.map_or_else(|| model.line_max_column(row), |w| w.start);
        }
        WordNavigationType::WordStart | WordNavigationType::WordStartFast => {
            if let Some(w) = next
                && !moved_down
                && column >= w.start
            {
                next = find_next_word_on_line(classifier, model, Position::new(row, w.end));
            }
            column = next.map_or_else(|| model.line_max_column(row), |w| w.start);
        }
    }
    Position::new(row, column)
}

/// Monaco `WordOperations._moveWordPartRight`.
fn move_word_part_right_only(model: &dyn CursorModel, position: Position) -> Position {
    let row = position.row;
    let max_column = model.line_max_column(row);
    if position.column == max_column {
        return if row < last_row(model) {
            Position::new(row + 1, 0)
        } else {
            position
        };
    }
    let line = line_units(model, row);
    let len = line.len();
    for c in position.column as usize + 1..len {
        let left = line[c - 1];
        let right = line[c];
        if left != UNDERLINE && right == UNDERLINE {
            return Position::new(row, c as u32);
        }
        if left != DASH && right == DASH {
            return Position::new(row, c as u32);
        }
        if (is_lower_ascii_letter(left) || is_ascii_digit(left)) && is_upper_ascii_letter(right) {
            return Position::new(row, c as u32);
        }
        if is_upper_ascii_letter(left) && is_upper_ascii_letter(right) && c + 1 < len {
            let right_right = line[c + 1];
            if is_lower_ascii_letter(right_right) || is_ascii_digit(right_right) {
                return Position::new(row, c as u32);
            }
        }
    }
    Position::new(row, max_column)
}

/// Monaco `WordPartOperations.moveWordPartLeft`.
pub(crate) fn move_word_part_left(
    classifier: &WordClassifier,
    model: &dyn CursorModel,
    position: Position,
    has_multicursor: bool,
) -> Position {
    let candidates = [
        move_word_left(
            classifier,
            model,
            position,
            WordNavigationType::WordStart,
            has_multicursor,
        ),
        move_word_left(
            classifier,
            model,
            position,
            WordNavigationType::WordEnd,
            has_multicursor,
        ),
        move_word_part_left_only(model, position),
    ];
    candidates.into_iter().max().unwrap_or(position)
}

/// Monaco `WordPartOperations.moveWordPartRight`.
pub(crate) fn move_word_part_right(
    classifier: &WordClassifier,
    model: &dyn CursorModel,
    position: Position,
) -> Position {
    let candidates = [
        move_word_right(classifier, model, position, WordNavigationType::WordStart),
        move_word_right(classifier, model, position, WordNavigationType::WordEnd),
        move_word_part_right_only(model, position),
    ];
    candidates.into_iter().min().unwrap_or(position)
}

// ---------------------------------------------------------------------------
// Deletion ranges

pub(crate) struct DeleteWordContext<'a> {
    pub classifier: &'a WordClassifier,
    pub model: &'a dyn CursorModel,
    pub selection: super::Selection,
    pub whitespace_heuristics: bool,
}

fn delete_word_left_whitespace(
    model: &dyn CursorModel,
    position: Position,
) -> Option<Range<Position>> {
    let line = line_units(model, position.row);
    let start_index = position.column as i64 - 1;
    let last_non_whitespace = if start_index < 0 {
        -1
    } else {
        strings::last_non_whitespace_index(&line, Some(start_index as usize))
            .map_or(-1, |i| i as i64)
    };
    if last_non_whitespace + 1 < start_index {
        return Some(Position::new(position.row, (last_non_whitespace + 1) as u32)..position);
    }
    None
}

/// Monaco `WordOperations.deleteWordLeft` (without auto-closing pair
/// deletion, which needs language configuration).
pub(crate) fn delete_word_left_range(
    ctx: &DeleteWordContext,
    nav: WordNavigationType,
) -> Option<Range<Position>> {
    let model = ctx.model;
    if !ctx.selection.is_empty() {
        return Some(ctx.selection.range());
    }
    let position = ctx.selection.head;
    let mut row = position.row;
    let mut column = position.column;
    if row == 0 && column == 0 {
        // Ignore deleting at beginning of file.
        return None;
    }
    if ctx.whitespace_heuristics
        && let Some(r) = delete_word_left_whitespace(model, position)
    {
        return Some(r);
    }
    let mut prev = find_previous_word_on_line(ctx.classifier, model, position);
    if nav != WordNavigationType::WordStart
        && let Some(w) = prev
        && column <= w.end
    {
        prev = find_previous_word_on_line(ctx.classifier, model, Position::new(row, w.start));
    }
    match prev {
        Some(w) => {
            column = if nav == WordNavigationType::WordStart {
                w.start
            } else {
                w.end
            };
        }
        None => {
            if column > 0 {
                column = 0;
            } else {
                row -= 1;
                column = model.line_max_column(row);
            }
        }
    }
    Some(Position::new(row, column)..position)
}

fn delete_word_right_whitespace(
    model: &dyn CursorModel,
    position: Position,
) -> Option<Range<Position>> {
    let line = line_units(model, position.row);
    let start_index = position.column as usize;
    let first_non_whitespace = line
        .iter()
        .enumerate()
        .skip(start_index)
        .find(|(_, c)| **c != b' ' as u16 && **c != b'\t' as u16)
        .map_or(line.len(), |(i, _)| i);
    if start_index < first_non_whitespace {
        return Some(position..Position::new(position.row, first_non_whitespace as u32));
    }
    None
}

/// Monaco `WordOperations.deleteWordRight`.
pub(crate) fn delete_word_right_range(
    ctx: &DeleteWordContext,
    nav: WordNavigationType,
) -> Option<Range<Position>> {
    let model = ctx.model;
    if !ctx.selection.is_empty() {
        return Some(ctx.selection.range());
    }
    let position = ctx.selection.head;
    let mut row = position.row;
    let mut column = position.column;
    let line_count_row = last_row(model);
    let max_column = model.line_max_column(row);
    if row == line_count_row && column == max_column {
        // Ignore deleting at end of file.
        return None;
    }
    if ctx.whitespace_heuristics
        && let Some(r) = delete_word_right_whitespace(model, position)
    {
        return Some(r);
    }
    let mut next = find_next_word_on_line(ctx.classifier, model, position);
    if nav != WordNavigationType::WordEnd
        && let Some(w) = next
        && column >= w.start
    {
        next = find_next_word_on_line(ctx.classifier, model, Position::new(row, w.end));
    }
    match next {
        Some(w) => {
            column = if nav == WordNavigationType::WordEnd {
                w.end
            } else {
                w.start
            };
        }
        None => {
            if column < max_column || row == line_count_row {
                column = max_column;
            } else {
                row += 1;
                let next = find_next_word_on_line(ctx.classifier, model, Position::new(row, 0));
                column = next.map_or_else(|| model.line_max_column(row), |w| w.start);
            }
        }
    }
    let other = Position::new(row, column);
    Some(position.min(other)..position.max(other))
}

/// Monaco `WordPartOperations.deleteWordPartLeft`.
pub(crate) fn delete_word_part_left_range(ctx: &DeleteWordContext) -> Option<Range<Position>> {
    let mut candidates: Vec<Range<Position>> = [
        delete_word_left_range(ctx, WordNavigationType::WordStart),
        delete_word_left_range(ctx, WordNavigationType::WordEnd),
        Some(if ctx.selection.is_empty() {
            let pos = ctx.selection.head;
            let to = move_word_part_left_only(ctx.model, pos);
            pos.min(to)..pos.max(to)
        } else {
            ctx.selection.range()
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    candidates.sort_by_key(|a| (a.end, a.start));
    candidates.get(2).cloned()
}

/// Monaco `WordPartOperations.deleteWordPartRight`.
pub(crate) fn delete_word_part_right_range(ctx: &DeleteWordContext) -> Option<Range<Position>> {
    let mut candidates: Vec<Range<Position>> = [
        delete_word_right_range(ctx, WordNavigationType::WordStart),
        delete_word_right_range(ctx, WordNavigationType::WordEnd),
        Some(if ctx.selection.is_empty() {
            let pos = ctx.selection.head;
            let to = move_word_part_right_only(ctx.model, pos);
            pos.min(to)..pos.max(to)
        } else {
            ctx.selection.range()
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    candidates.sort_by_key(|a| (a.start, a.end));
    candidates.first().cloned()
}

fn is_ws(line: &[u16], index: usize) -> bool {
    line.get(index)
        .is_some_and(|&c| c == b' ' as u16 || c == b'\t' as u16)
}

/// Monaco `WordOperations.deleteInsideWord`.
pub(crate) fn delete_inside_word_range(
    classifier: &WordClassifier,
    model: &dyn CursorModel,
    selection: super::Selection,
    only_word: bool,
) -> Range<Position> {
    if !selection.is_empty() {
        return selection.range();
    }
    let position = selection.head;
    let line = line_units(model, position.row);
    let len = line.len();
    let row = position.row;
    let col = position.column as usize;

    // _deleteInsideWordWhitespace
    if len > 0 {
        let mut left = col.saturating_sub(1);
        let mut right = col.min(len - 1);
        if is_ws(&line, left) && is_ws(&line, right) {
            while left > 0 && is_ws(&line, left - 1) {
                left -= 1;
            }
            while right + 1 < len && is_ws(&line, right + 1) {
                right += 1;
            }
            return Position::new(row, left as u32)..Position::new(row, right as u32 + 1);
        }
    }

    // _deleteInsideWordDetermineDeleteRange
    if len == 0 {
        if row > 0 {
            return Position::new(row - 1, model.line_max_column(row - 1))..Position::new(row, 0);
        } else if row < last_row(model) {
            return Position::new(row, 0)..Position::new(row + 1, 0);
        } else {
            return Position::new(row, 0)..Position::new(row, 0);
        }
    }
    let column = position.column;
    let touches_word = |w: &FoundWord| w.start <= column && column <= w.end;
    let with_position = |start: u32, end: u32| {
        Position::new(row, start.min(column))..Position::new(row, end.max(column))
    };
    let delete_word_and_adjacent_whitespace = |w: &FoundWord| {
        let mut start = w.start as usize;
        let mut end = w.end as usize;
        if only_word {
            return with_position(start as u32, end as u32);
        }
        let mut expanded_to_the_right = false;
        while end < len && is_ws(&line, end) {
            expanded_to_the_right = true;
            end += 1;
        }
        if !expanded_to_the_right {
            while start > 0 && is_ws(&line, start - 1) {
                start -= 1;
            }
        }
        with_position(start as u32, end as u32)
    };
    let prev = do_find_previous_word_on_line(&line, classifier, column);
    if let Some(w) = prev
        && touches_word(&w)
    {
        return delete_word_and_adjacent_whitespace(&w);
    }
    let next = do_find_next_word_on_line(&line, classifier, column);
    if let Some(w) = next
        && touches_word(&w)
    {
        return delete_word_and_adjacent_whitespace(&w);
    }
    match (prev, next) {
        (Some(p), Some(n)) => with_position(p.end, n.start),
        (Some(p), None) => with_position(p.start, p.end),
        (None, Some(n)) => with_position(n.start, n.end),
        (None, None) => with_position(0, len as u32),
    }
}

// ---------------------------------------------------------------------------
// Word selection (double click)

fn word_ranges(
    classifier: &WordClassifier,
    model: &dyn CursorModel,
    position: Position,
) -> (Option<FoundWord>, Option<FoundWord>) {
    (
        find_previous_word_on_line(classifier, model, position),
        find_next_word_on_line(classifier, model, position),
    )
}

/// Monaco `WordOperations.word`: the cursor state after selecting the word
/// at `position` (double click), or extending a word selection to it (drag
/// after double click, `in_selection_mode`).
pub(crate) fn word_selection(
    config: &CursorConfig,
    model: &dyn CursorModel,
    cursor: &SingleCursorState,
    in_selection_mode: bool,
    position: Position,
) -> SingleCursorState {
    let classifier = WordClassifier::new(&config.word_separators);
    let (prev_word, next_word) = word_ranges(&classifier, model, position);
    let col = position.column;
    let row = position.row;
    if !in_selection_mode {
        let touching = |w: Option<FoundWord>| {
            w.filter(|w| match w.word_type {
                WordType::Regular => w.start <= col && col <= w.end,
                // Stricter for separators: don't include the end boundary.
                WordType::Separator => w.start <= col && col < w.end,
                WordType::None => false,
            })
        };
        let (start_column, end_column) = if let Some(w) = touching(prev_word)
            .filter(|w| w.word_type == WordType::Regular)
            .or_else(|| touching(prev_word).filter(|w| w.word_type == WordType::Separator))
            .or_else(|| touching(next_word).filter(|w| w.word_type == WordType::Regular))
            .or_else(|| touching(next_word).filter(|w| w.word_type == WordType::Separator))
        {
            (w.start, w.end)
        } else {
            (
                prev_word.map_or(0, |w| w.end),
                next_word.map_or_else(|| model.line_max_column(row), |w| w.start),
            )
        };
        return SingleCursorState {
            selection_start: Position::new(row, start_column)..Position::new(row, end_column),
            selection_start_kind: SelectionStartKind::Word,
            selection_start_leftover_visible_columns: 0,
            position: Position::new(row, end_column),
            leftover_visible_columns: 0,
        };
    }

    let inside = |w: Option<FoundWord>| {
        w.filter(|w| w.word_type == WordType::Regular && w.start < col && col < w.end)
    };
    let (start_column, end_column) = match inside(prev_word).or_else(|| inside(next_word)) {
        Some(w) => (w.start, w.end),
        None => (col, col),
    };
    let selection_start = &cursor.selection_start;
    let contains = |p: Position| selection_start.start <= p && p <= selection_start.end;
    let column = if contains(position) {
        selection_start.end.column
    } else if position <= selection_start.start {
        if contains(Position::new(row, start_column)) {
            selection_start.end.column
        } else {
            start_column
        }
    } else if contains(Position::new(row, end_column)) {
        selection_start.start.column
    } else {
        end_column
    };
    cursor.move_to(true, Position::new(row, column), 0)
}

/// The range a double click at `p` selects (Monaco `WordOperations.word`):
/// the word or separator run touching `p`, or the whitespace between the
/// surrounding words. `None` if that range is empty (e.g. an empty line).
pub fn word_at(
    config: &CursorConfig,
    model: &impl CursorModel,
    p: Position,
) -> Option<Range<Position>> {
    let model: &dyn CursorModel = model;
    let p = super::validate_position(model, p);
    let state = word_selection(
        config,
        model,
        &SingleCursorState::from_position(p),
        false,
        p,
    );
    let range = state.selection_start;
    (range.start != range.end).then_some(range)
}

/// Moves every cursor by words. `left` picks the direction; `nav` picks the
/// boundaries (see [`WordNavigationType`]). This is Monaco's
/// `MoveWordCommand` family; [`super::move_cursors`] covers the common
/// variants through [`super::MoveCommand`].
pub fn move_word(
    config: &CursorConfig,
    model: &impl CursorModel,
    cursors: &mut CursorCollection,
    left: bool,
    nav: WordNavigationType,
    select: bool,
) {
    let model: &dyn CursorModel = model;
    let separators = if nav == WordNavigationType::WordAccessibility {
        USUAL_WORD_SEPARATORS
    } else {
        config.word_separators.as_str()
    };
    let classifier = WordClassifier::new(separators);
    let has_multicursor = cursors.all().len() > 1;
    move_selections_by(model, cursors, select, |position| {
        if left {
            move_word_left(&classifier, model, position, nav, has_multicursor)
        } else {
            move_word_right(&classifier, model, position, nav)
        }
    });
}

/// Monaco `MoveWordCommand.runEditorCommand`/`_moveTo`: moves each
/// selection's head to `f(head)`.
pub(crate) fn move_selections_by(
    model: &dyn CursorModel,
    cursors: &mut CursorCollection,
    select: bool,
    mut f: impl FnMut(Position) -> Position,
) {
    let states = cursors
        .selections()
        .into_iter()
        .map(|sel| {
            let to = f(sel.head);
            let selection = if select {
                super::Selection::new(sel.anchor, to)
            } else {
                super::Selection::collapsed(to)
            };
            SingleCursorState::from_selection(selection)
        })
        .collect();
    cursors.validated_states(model, states);
}
