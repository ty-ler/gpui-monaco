//! A rope: text stored as small chunks in a [`SumTree`], so edits, slicing and
//! coordinate conversions are `O(log n)`.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Range;

use arrayvec::ArrayString;
use sum_tree::{Bias, ContextLessSummary, Dimension, Item, SeekTarget, SumTree};

use crate::point::{Location, OffsetUtf16, Point, PointUtf16, Target};

/// Maximum bytes per chunk. Chunks are split on char boundaries, so a chunk can
/// be up to 3 bytes shorter than this.
pub(crate) const CHUNK_MAX: usize = 256;

#[derive(Clone, Default)]
struct Chunk(ArrayString<CHUNK_MAX>);

impl Item for Chunk {
    type Summary = TextSummary;

    fn summary(&self, _: ()) -> TextSummary {
        TextSummary::from(self.0.as_str())
    }
}

/// Aggregate measurements of a span of text.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct TextSummary {
    /// The extent of the text in every coordinate space.
    pub extent: Location,
    /// Chars on the first line (before the first `\n`).
    pub first_line_chars: u32,
    /// Chars on the last line (after the last `\n`).
    pub last_line_chars: u32,
    /// Row (relative to the start of the span) of the longest line by chars.
    pub longest_row: u32,
    pub longest_row_chars: u32,
}

impl TextSummary {
    pub fn len(&self) -> usize {
        self.extent.offset
    }

    pub fn is_empty(&self) -> bool {
        self.extent.offset == 0
    }

    pub fn lines(&self) -> Point {
        self.extent.point
    }
}

impl From<&str> for TextSummary {
    fn from(text: &str) -> Self {
        let mut extent = Location::default();
        let mut first_line_chars = 0;
        let mut line_chars = 0u32;
        let mut longest_row = 0;
        let mut longest_row_chars = 0;
        for ch in text.chars() {
            if ch == '\n' {
                if extent.point.row == 0 {
                    first_line_chars = line_chars;
                }
                if line_chars > longest_row_chars {
                    longest_row = extent.point.row;
                    longest_row_chars = line_chars;
                }
                line_chars = 0;
            } else {
                line_chars += 1;
            }
            extent.advance(ch);
        }
        if extent.point.row == 0 {
            first_line_chars = line_chars;
        }
        if line_chars > longest_row_chars {
            longest_row = extent.point.row;
            longest_row_chars = line_chars;
        }
        Self {
            extent,
            first_line_chars,
            last_line_chars: line_chars,
            longest_row,
            longest_row_chars,
        }
    }
}

impl ContextLessSummary for TextSummary {
    fn zero() -> Self {
        Self::default()
    }

    fn add_summary(&mut self, other: &Self) {
        let joined_line_chars = self.last_line_chars + other.first_line_chars;
        if joined_line_chars > self.longest_row_chars {
            self.longest_row = self.extent.point.row;
            self.longest_row_chars = joined_line_chars;
        }
        if other.longest_row_chars > self.longest_row_chars {
            self.longest_row = self.extent.point.row + other.longest_row;
            self.longest_row_chars = other.longest_row_chars;
        }
        if self.extent.point.row == 0 {
            self.first_line_chars += other.first_line_chars;
        }
        if other.extent.point.row == 0 {
            self.last_line_chars += other.first_line_chars;
        } else {
            self.last_line_chars = other.last_line_chars;
        }
        let e = &mut self.extent;
        let o = &other.extent;
        e.offset += o.offset;
        e.offset_utf16 += o.offset_utf16;
        e.chars += o.chars;
        e.point += o.point;
        e.point_utf16 += o.point_utf16;
    }
}

macro_rules! extent_dimension {
    ($ty:ty, $field:ident) => {
        impl<'a> Dimension<'a, TextSummary> for $ty {
            fn zero(_: ()) -> Self {
                Default::default()
            }

            fn add_summary(&mut self, summary: &'a TextSummary, _: ()) {
                *self += summary.extent.$field;
            }
        }
    };
}

extent_dimension!(usize, offset);
extent_dimension!(Point, point);
extent_dimension!(PointUtf16, point_utf16);
extent_dimension!(OffsetUtf16, offset_utf16);

struct Seek<T>(T);

impl<'a, T: Target> SeekTarget<'a, TextSummary, TextSummary> for Seek<T> {
    fn cmp(&self, cursor_location: &TextSummary, _: ()) -> Ordering {
        self.0.cmp_location(&cursor_location.extent)
    }
}

/// An immutable-by-default, cheaply clonable text buffer.
#[derive(Clone, Default)]
pub struct Rope {
    chunks: SumTree<Chunk>,
}

impl Rope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn summary(&self) -> TextSummary {
        *self.chunks.summary()
    }

    pub fn len(&self) -> usize {
        self.chunks.summary().extent.offset
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len_utf16(&self) -> OffsetUtf16 {
        self.chunks.summary().extent.offset_utf16
    }

    /// The position just past the last character.
    pub fn max_point(&self) -> Point {
        self.chunks.summary().extent.point
    }

    pub fn max_point_utf16(&self) -> PointUtf16 {
        self.chunks.summary().extent.point_utf16
    }

    /// Number of lines. An empty rope has one (empty) line, like Monaco.
    pub fn line_count(&self) -> u32 {
        self.max_point().row + 1
    }

    /// Appends `text`.
    pub fn push(&mut self, mut text: &str) {
        if text.is_empty() {
            return;
        }
        // Top up the last chunk first so repeated small pushes stay compact.
        self.chunks.update_last(
            |last| {
                let room = CHUNK_MAX - last.0.len();
                if room > 0 {
                    let take = floor_char_boundary(text, room);
                    last.0.push_str(&text[..take]);
                    text = &text[take..];
                }
            },
            (),
        );
        if text.is_empty() {
            return;
        }
        let mut chunks = Vec::with_capacity(text.len() / CHUNK_MAX + 1);
        while !text.is_empty() {
            let take = floor_char_boundary(text, CHUNK_MAX);
            let mut chunk = ArrayString::new();
            chunk.push_str(&text[..take]);
            chunks.push(Chunk(chunk));
            text = &text[take..];
        }
        self.chunks.extend(chunks, ());
    }

    /// Appends another rope.
    pub fn append(&mut self, other: Rope) {
        let mut other_chunks = other.chunks;
        // Merge a small boundary pair so chunks don't degrade into slivers.
        if let (Some(last), Some(first)) = (self.chunks.last(), other_chunks.first())
            && last.0.len() + first.0.len() <= CHUNK_MAX
        {
            let first = first.0;
            self.chunks.update_last(|last| last.0.push_str(&first), ());
            let rest = {
                let mut cursor = other_chunks.cursor::<usize>(());
                cursor.seek(&first.len(), Bias::Right);
                cursor.suffix()
            };
            other_chunks = rest;
        }
        self.chunks.append(other_chunks, ());
    }

    /// The text in `range` (byte offsets, clamped to the rope) as a new rope.
    pub fn slice(&self, range: Range<usize>) -> Rope {
        let end = range.end.min(self.len());
        let start = range.start.min(end);
        let mut result = Rope::new();
        let mut cursor = self.chunks.cursor::<usize>(());
        cursor.seek(&start, Bias::Right);
        if let Some(chunk) = cursor.item() {
            let chunk_start = *cursor.start();
            let local_end = end.min(cursor.end()) - chunk_start;
            result.push(&chunk.0[start - chunk_start..local_end]);
            if end > cursor.end() {
                cursor.next();
                result.append(Rope {
                    chunks: cursor.slice(&end, Bias::Right),
                });
                if let Some(chunk) = cursor.item() {
                    result.push(&chunk.0[..end - cursor.start()]);
                }
            }
        }
        result
    }

    /// Replaces the bytes in `range` with `text`.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        let len = self.len();
        let mut result = self.slice(0..range.start);
        result.push(text);
        result.append(self.slice(range.end..len));
        *self = result;
    }

    /// Resolves `target` to a [`Location`] in every coordinate space.
    ///
    /// Targets past the end clamp to the end. Point targets with a column past
    /// the end of their row clamp to the end of that row. Targets that fall in
    /// the middle of a character clamp to the start of that character.
    pub fn locate<T: Target>(&self, target: T) -> Location {
        let mut cursor = self.chunks.cursor::<TextSummary>(());
        cursor.seek(&Seek(target), Bias::Left);
        let mut location = cursor.start().extent;
        if let Some(chunk) = cursor.item() {
            for ch in chunk.0.chars() {
                let mut next = location;
                next.advance(ch);
                match target.cmp_location(&next) {
                    Ordering::Less => break,
                    Ordering::Equal => {
                        location = next;
                        break;
                    }
                    Ordering::Greater => location = next,
                }
            }
        }
        location
    }

    pub fn offset_to_point(&self, offset: usize) -> Point {
        self.locate(offset).point
    }

    pub fn point_to_offset(&self, point: Point) -> usize {
        self.locate(point).offset
    }

    pub fn offset_to_point_utf16(&self, offset: usize) -> PointUtf16 {
        self.locate(offset).point_utf16
    }

    pub fn point_utf16_to_offset(&self, point: PointUtf16) -> usize {
        self.locate(point).offset
    }

    pub fn point_to_point_utf16(&self, point: Point) -> PointUtf16 {
        self.locate(point).point_utf16
    }

    pub fn point_utf16_to_point(&self, point: PointUtf16) -> Point {
        self.locate(point).point
    }

    pub fn offset_to_offset_utf16(&self, offset: usize) -> OffsetUtf16 {
        self.locate(offset).offset_utf16
    }

    pub fn offset_utf16_to_offset(&self, offset: OffsetUtf16) -> usize {
        self.locate(offset).offset
    }

    /// Moves `offset` onto a char boundary, clamped to the rope.
    pub fn clip_offset(&self, offset: usize, bias: Bias) -> usize {
        if offset >= self.len() {
            return self.len();
        }
        let left = self.locate(offset).offset;
        if left == offset || bias == Bias::Left {
            left
        } else {
            left + self.chars_at(left).next().map_or(0, char::len_utf8)
        }
    }

    /// Clamps `point` to a valid row, a column within that row and a char
    /// boundary.
    pub fn clip_point(&self, point: Point, bias: Bias) -> Point {
        let located = self.locate(point);
        if located.point == point || bias == Bias::Left {
            return located.point;
        }
        // Mid-char with right bias: step over the char unless at end of row.
        match self.chars_at(located.offset).next() {
            Some(ch) if ch != '\n' && located.point.row == point.row => Point::new(
                located.point.row,
                located.point.column + ch.len_utf8() as u32,
            ),
            _ => located.point,
        }
    }

    /// Clamps a UTF-16 point like [`Rope::clip_point`]. A column in the middle
    /// of a surrogate pair is clipped according to `bias`.
    pub fn clip_point_utf16(&self, point: PointUtf16, bias: Bias) -> PointUtf16 {
        let located = self.locate(point);
        if located.point_utf16 == point || bias == Bias::Left {
            return located.point_utf16;
        }
        match self.chars_at(located.offset).next() {
            Some(ch) if ch != '\n' && located.point_utf16.row == point.row => PointUtf16::new(
                located.point_utf16.row,
                located.point_utf16.column + ch.len_utf16() as u32,
            ),
            _ => located.point_utf16,
        }
    }

    /// Monaco's `validateRange`: the start clips left; a non-empty range's end
    /// clips right, so a range never splits a surrogate pair.
    pub fn clip_range_utf16(&self, range: Range<PointUtf16>) -> Range<PointUtf16> {
        let start = self.clip_point_utf16(range.start, Bias::Left);
        if range.start == range.end {
            return start..start;
        }
        let end = self.clip_point_utf16(range.end, Bias::Right).max(start);
        start..end
    }

    /// Byte length of `row`, excluding the newline. Rows past the end are 0.
    pub fn line_len(&self, row: u32) -> u32 {
        if row > self.max_point().row {
            return 0;
        }
        self.locate(Point::new(row, u32::MAX)).point.column
    }

    /// Length of `row` in UTF-16 code units, excluding the newline.
    pub fn line_len_utf16(&self, row: u32) -> u32 {
        if row > self.max_point().row {
            return 0;
        }
        self.locate(Point::new(row, u32::MAX)).point_utf16.column
    }

    /// Byte range of `row`, excluding the newline.
    pub fn line_range(&self, row: u32) -> Range<usize> {
        let start = self.point_to_offset(Point::new(row, 0));
        let end = self.point_to_offset(Point::new(row, u32::MAX));
        start..end
    }

    /// The text of `row`, excluding the newline.
    pub fn line(&self, row: u32) -> String {
        self.chunks_in_range(self.line_range(row)).collect()
    }

    /// Summary of the text in `range`.
    pub fn text_summary_for_range(&self, range: Range<usize>) -> TextSummary {
        let mut summary = TextSummary::default();
        for chunk in self.chunks_in_range(range) {
            ContextLessSummary::add_summary(&mut summary, &TextSummary::from(chunk));
        }
        summary
    }

    /// Iterates the rope's text as `&str` pieces covering `range`.
    pub fn chunks_in_range(&self, range: Range<usize>) -> Chunks<'_> {
        let end = range.end.min(self.len());
        let start = range.start.min(end);
        let mut cursor = self.chunks.cursor::<usize>(());
        cursor.seek(&start, Bias::Right);
        Chunks {
            cursor,
            range: start..end,
            reversed: false,
        }
    }

    /// Iterates `&str` pieces covering `range` from the end to the start.
    pub fn reversed_chunks_in_range(&self, range: Range<usize>) -> Chunks<'_> {
        let end = range.end.min(self.len());
        let start = range.start.min(end);
        let mut cursor = self.chunks.cursor::<usize>(());
        cursor.seek(&end, Bias::Left);
        Chunks {
            cursor,
            range: start..end,
            reversed: true,
        }
    }

    pub fn chunks(&self) -> Chunks<'_> {
        self.chunks_in_range(0..self.len())
    }

    /// Chars starting at byte `offset`.
    pub fn chars_at(&self, offset: usize) -> impl Iterator<Item = char> + '_ {
        self.chunks_in_range(offset..self.len())
            .flat_map(str::chars)
    }

    /// Chars before byte `offset`, nearest first.
    pub fn reversed_chars_at(&self, offset: usize) -> impl Iterator<Item = char> + '_ {
        self.reversed_chunks_in_range(0..offset)
            .flat_map(|chunk| chunk.chars().rev())
    }

    /// The text in `range` as a `String`.
    pub fn text_for_range(&self, range: Range<usize>) -> String {
        self.chunks_in_range(range).collect()
    }

    /// Iterates lines (without newlines).
    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        (0..self.line_count()).map(|row| self.line(row))
    }
}

/// Iterator over `&str` pieces of a rope. See [`Rope::chunks_in_range`].
pub struct Chunks<'a> {
    cursor: sum_tree::Cursor<'a, 'static, Chunk, usize>,
    range: Range<usize>,
    reversed: bool,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        loop {
            if self.range.is_empty() {
                return None;
            }
            let chunk = self.cursor.item()?;
            let chunk_start = *self.cursor.start();
            let chunk_end = self.cursor.end();
            let start = self.range.start.max(chunk_start);
            let end = self.range.end.min(chunk_end);
            if self.reversed {
                self.cursor.prev();
                self.range.end = start;
            } else {
                self.cursor.next();
                self.range.start = end;
            }
            if start < end {
                return Some(&chunk.0[start - chunk_start..end - chunk_start]);
            }
        }
    }
}

impl From<&str> for Rope {
    fn from(text: &str) -> Self {
        let mut rope = Rope::new();
        rope.push(text);
        rope
    }
}

impl From<String> for Rope {
    fn from(text: String) -> Self {
        Rope::from(text.as_str())
    }
}

impl fmt::Display for Rope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl fmt::Debug for Rope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_string(), f)
    }
}

impl PartialEq for Rope {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.chars_at(0).eq(other.chars_at(0))
    }
}

impl Eq for Rope {}

impl PartialEq<str> for Rope {
    fn eq(&self, other: &str) -> bool {
        self.len() == other.len() && self.chars_at(0).eq(other.chars())
    }
}

impl PartialEq<&str> for Rope {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

/// The largest char boundary `<= max` in `text`.
fn floor_char_boundary(text: &str, max: usize) -> usize {
    if max >= text.len() {
        return text.len();
    }
    let mut i = max;
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn reference_locate(text: &str, offset: usize) -> Location {
        let mut location = Location::default();
        for ch in text.chars() {
            if location.offset + ch.len_utf8() > offset {
                break;
            }
            location.advance(ch);
        }
        location
    }

    #[test]
    fn empty_rope() {
        let rope = Rope::new();
        assert_eq!(rope.len(), 0);
        assert_eq!(rope.line_count(), 1);
        assert_eq!(rope.line(0), "");
        assert_eq!(rope.max_point(), Point::zero());
        assert_eq!(rope.slice(0..10), "");
    }

    #[test]
    fn lines_and_points() {
        let rope = Rope::from("abc\n\ndéf\n𝄞x");
        assert_eq!(rope.line_count(), 4);
        assert_eq!(rope.line(2), "déf");
        assert_eq!(rope.line_len(2), 4);
        assert_eq!(rope.line_len_utf16(3), 3);
        assert_eq!(rope.point_to_offset(Point::new(2, 99)), 9);
        assert_eq!(rope.offset_to_point(9), Point::new(2, 4));
        assert_eq!(rope.point_utf16_to_offset(PointUtf16::new(3, 2)), 14);
        // Middle of the surrogate pair clips left, or right with Bias::Right.
        assert_eq!(
            rope.clip_point_utf16(PointUtf16::new(3, 1), Bias::Left),
            PointUtf16::new(3, 0)
        );
        assert_eq!(
            rope.clip_point_utf16(PointUtf16::new(3, 1), Bias::Right),
            PointUtf16::new(3, 2)
        );
        assert_eq!(rope.clip_offset(7, Bias::Left), 6);
        assert_eq!(rope.clip_offset(7, Bias::Right), 8);
    }

    #[test]
    fn longest_row() {
        let rope = Rope::from(&*format!("a\n{}\nbb", "x".repeat(600)));
        assert_eq!(rope.summary().longest_row, 1);
        assert_eq!(rope.summary().longest_row_chars, 600);
    }

    fn arb_text() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just("a".to_string()),
                Just("\n".to_string()),
                Just("é".to_string()),
                Just("𝄞".to_string()),
                Just("日本".to_string()),
                "[a-z ]{0,40}",
            ],
            0..120,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        #[test]
        fn matches_string_reference(
            text in arb_text(),
            edits in proptest::collection::vec((any::<usize>(), any::<usize>(), arb_text()), 0..8),
        ) {
            let mut rope = Rope::from(text.as_str());
            let mut reference = text.clone();
            for (a, b, insert) in edits {
                let mut start = a % (reference.len() + 1);
                let mut end = b % (reference.len() + 1);
                if start > end { std::mem::swap(&mut start, &mut end); }
                while !reference.is_char_boundary(start) { start -= 1; }
                while !reference.is_char_boundary(end) { end += 1; }
                rope.replace(start..end, &insert);
                reference.replace_range(start..end, &insert);
                prop_assert_eq!(rope.to_string(), reference.clone());
            }

            prop_assert_eq!(rope.summary(), TextSummary::from(reference.as_str()));
            let lines: Vec<&str> = reference.split('\n').collect();
            prop_assert_eq!(rope.line_count() as usize, lines.len());
            for (row, line) in lines.iter().enumerate() {
                prop_assert_eq!(&rope.line(row as u32), line);
            }
            for offset in 0..=reference.len() {
                let expected = reference_locate(&reference, offset);
                prop_assert_eq!(rope.locate(offset), expected);
                prop_assert_eq!(rope.locate(expected.point), expected);
                prop_assert_eq!(rope.locate(expected.point_utf16), expected);
                prop_assert_eq!(rope.locate(expected.offset_utf16), expected);
                let reversed: String = rope.reversed_chars_at(expected.offset).collect();
                let expected_reversed: String = reference[..expected.offset].chars().rev().collect();
                prop_assert_eq!(reversed, expected_reversed);
            }
            let len = reference.len();
            for start in (0..=len).step_by(7) {
                for end in (start..=len).step_by(13) {
                    if reference.is_char_boundary(start) && reference.is_char_boundary(end) {
                        prop_assert_eq!(rope.slice(start..end).to_string(), &reference[start..end]);
                        prop_assert_eq!(
                            rope.text_summary_for_range(start..end),
                            TextSummary::from(&reference[start..end])
                        );
                    }
                }
            }
        }
    }
}
