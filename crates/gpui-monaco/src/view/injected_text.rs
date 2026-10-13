//! Injected text in view columns: a port of Monaco's
//! `ModelLineProjectionData` (`src/vs/editor/common/modelLineProjectionData.ts`,
//! MIT), which maps a model line's offsets to its view lines once text is
//! injected into it and it's wrapped, and keeps the cursor to the stops
//! around injected text (`InjectedTextCursorStops`).
//!
//! Offsets are UTF-16 units. "Input" offsets are the model line's;
//! "offsets in input with injections" count the injected text too (Monaco's
//! `applyInjectedText`); output positions are a view line and an offset in
//! it, the view column. [`ViewLines`](super::view_lines::ViewLines) keeps
//! one projection per model row with injected text or wrap points.

use gpui::SharedString;

use super::line_breaks::LineBreakData;
use crate::decorations::InjectedTextCursorStops;

/// Monaco `PositionAffinity`: which way to go when a position is ambiguous
/// (at injected text or a wrap point).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionAffinity {
    /// Prefer the left.
    Left,
    /// Prefer the right.
    Right,
    /// No preference: honor the injected text's cursor stops.
    None,
    /// Before injected text, ignoring its cursor stops.
    LeftOfInjectedText,
    /// After injected text, ignoring its cursor stops.
    RightOfInjectedText,
}

/// Monaco `OutputPosition`: a view line of the model line and an offset in
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputPosition {
    /// The view line, from 0 for the model line's first.
    pub output_line_index: u32,
    /// The offset in the view line.
    pub output_offset: u32,
}

impl std::fmt::Display for OutputPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.output_line_index, self.output_offset)
    }
}

/// One injected text: its length and cursor stops (Monaco's
/// `InjectedTextOptions` as the projection reads them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Injection {
    /// The input offset it's injected at.
    pub offset: u32,
    /// Its length in UTF-16 units.
    pub length: u32,
    /// Where the cursor may stop around it.
    pub cursor_stops: InjectedTextCursorStops,
}

/// Text injected into a model line (Monaco `LineInjectedText`): a
/// decoration's `before` or `after` text, as the view lines see it.
#[derive(Clone, Debug, PartialEq)]
pub struct LineInjectedText {
    /// The UTF-16 column of the model line it's injected at.
    pub column: u32,
    /// The text.
    pub content: SharedString,
    /// Where the cursor may stop around it.
    pub cursor_stops: InjectedTextCursorStops,
    /// Token styles over parts of the text (Monaco's injected text
    /// `tokens`, which the minimap draws it in), as in
    /// [`InjectedText::tokens`](crate::InjectedText::tokens).
    pub tokens: Option<crate::decorations::InjectedTextTokens>,
}

impl LineInjectedText {
    /// Its length in UTF-16 units.
    pub fn len(&self) -> u32 {
        self.content.encode_utf16().count() as u32
    }

    /// Whether it has no text (Monaco leaves such text out,
    /// `occupiesHorizontalSpace`).
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Monaco `LineInjectedText.applyInjectedText`: `line` with `injected`
    /// (sorted by column) inserted.
    pub fn apply(line: &str, injected: &[LineInjectedText]) -> String {
        let mut result = String::with_capacity(
            line.len() + injected.iter().map(|t| t.content.len()).sum::<usize>(),
        );
        let mut last = 0;
        for text in injected {
            let at = super::view_lines::utf16_to_byte(line, text.column).max(last);
            result.push_str(&line[last..at]);
            last = at;
            result.push_str(&text.content);
        }
        result.push_str(&line[last..]);
        result
    }

    /// The projection's view of `injected`: offsets, lengths and stops.
    pub fn injections(injected: &[LineInjectedText]) -> Vec<Injection> {
        injected
            .iter()
            .map(|text| Injection {
                offset: text.column,
                length: text.len(),
                cursor_stops: text.cursor_stops,
            })
            .collect()
    }
}

/// Monaco `ModelLineProjectionData`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelLineProjectionData {
    /// The injected text, by offset (Monaco `injectionOffsets` and
    /// `injectionOptions`).
    injections: Vec<Injection>,
    /// Where each view line ends, in offsets in input with injections; the
    /// last is the length of the line with its injections.
    break_offsets: Vec<u32>,
    /// The visible column (with injected text, without continuation
    /// indentation) of each break offset (Monaco
    /// `breakOffsetsVisibleColumn`); empty when the line isn't wrapped.
    break_offsets_visible_column: Vec<u32>,
    /// The indent of wrapped view lines.
    wrapped_text_indent_length: u32,
}

/// An injected text found at an offset (Monaco `getInjectedTextAtOffset`).
struct InjectedTextAt {
    index: usize,
    offset_in_input_with_injections: u32,
    length: u32,
}

impl ModelLineProjectionData {
    /// A projection with `injections` (sorted by offset), `break_offsets`,
    /// `break_offsets_visible_column` and `wrapped_text_indent_length`.
    pub fn new(
        injections: Vec<Injection>,
        break_offsets: Vec<u32>,
        break_offsets_visible_column: Vec<u32>,
        wrapped_text_indent_length: u32,
    ) -> Self {
        Self {
            injections,
            break_offsets,
            break_offsets_visible_column,
            wrapped_text_indent_length,
        }
    }

    /// An unwrapped line of `line_length` UTF-16 units with `injections`.
    pub fn unwrapped(line_length: u32, injections: Vec<Injection>) -> Self {
        let total = line_length + injections.iter().map(|i| i.length).sum::<u32>();
        Self::new(injections, vec![total], Vec::new(), 0)
    }

    /// The projection of a line wrapped at `breaks`, whose offsets count
    /// `injections` (sorted by offset).
    pub fn from_line_breaks(injections: Vec<Injection>, breaks: LineBreakData) -> Self {
        Self::new(
            injections,
            breaks.break_offsets,
            breaks.break_offsets_visible_column,
            breaks.wrapped_text_indent_length,
        )
    }

    /// The injected text.
    pub fn injections(&self) -> &[Injection] {
        &self.injections
    }

    /// Where each view line ends, in offsets in input with injections.
    pub fn break_offsets(&self) -> &[u32] {
        &self.break_offsets
    }

    /// Monaco `wrappedTextIndentLength`: the indentation of continuation
    /// view lines.
    pub fn wrapped_text_indent_length(&self) -> u32 {
        self.wrapped_text_indent_length
    }

    /// The offsets in input with injections that view line
    /// `output_line_index` shows.
    pub fn output_range(&self, output_line_index: u32) -> std::ops::Range<u32> {
        let ix = output_line_index as usize;
        let start = if ix > 0 {
            self.break_offsets[ix - 1]
        } else {
            0
        };
        start..self.break_offsets[ix]
    }

    /// The visible column (injected text included) at which view line
    /// `output_line_index` starts, for tab stops.
    pub fn start_visible_column(&self, output_line_index: u32) -> u32 {
        match output_line_index {
            0 => 0,
            ix => self
                .break_offsets_visible_column
                .get(ix as usize - 1)
                .copied()
                .unwrap_or(0),
        }
    }

    /// The input offset of an offset in input with injections: inside
    /// injected text, the offset it's injected at.
    pub fn input_offset(&self, offset_in_input_with_injections: u32) -> u32 {
        let mut offset = offset_in_input_with_injections;
        for injection in &self.injections {
            if offset > injection.offset {
                if offset < injection.offset + injection.length {
                    return injection.offset;
                }
                offset -= injection.length;
            } else {
                break;
            }
        }
        offset
    }

    /// Monaco `getOutputLineCount`.
    pub fn output_line_count(&self) -> u32 {
        self.break_offsets.len() as u32
    }

    /// Monaco `getMinOutputOffset`.
    pub fn min_output_offset(&self, output_line_index: u32) -> u32 {
        if output_line_index > 0 {
            self.wrapped_text_indent_length
        } else {
            0
        }
    }

    /// Monaco `getLineLength`.
    pub fn line_length(&self, output_line_index: u32) -> u32 {
        let ix = output_line_index as usize;
        let start = if ix > 0 {
            self.break_offsets[ix - 1]
        } else {
            0
        };
        let mut length = self.break_offsets[ix] - start;
        if ix > 0 {
            length += self.wrapped_text_indent_length;
        }
        length
    }

    /// Monaco `getMaxOutputOffset`.
    pub fn max_output_offset(&self, output_line_index: u32) -> u32 {
        self.line_length(output_line_index)
    }

    /// Monaco `translateToInputOffset`: the model offset of a view position
    /// (inside injected text, the offset it's injected at).
    pub fn translate_to_input_offset(&self, output_line_index: u32, output_offset: u32) -> u32 {
        let output_offset = if output_line_index > 0 {
            output_offset.saturating_sub(self.wrapped_text_indent_length)
        } else {
            output_offset
        };
        let offset_in_input_with_injections = if output_line_index == 0 {
            output_offset
        } else {
            self.break_offsets[output_line_index as usize - 1] + output_offset
        };
        self.input_offset(offset_in_input_with_injections)
    }

    /// Monaco `translateToOutputPosition`: the view position of a model
    /// offset. At injected text, `Right` goes after it and anything else
    /// before it.
    pub fn translate_to_output_position(
        &self,
        input_offset: u32,
        affinity: PositionAffinity,
    ) -> OutputPosition {
        let mut offset_in_input_with_injections = input_offset;
        for injection in &self.injections {
            if input_offset < injection.offset {
                break;
            }
            if affinity != PositionAffinity::Right && input_offset == injection.offset {
                break;
            }
            offset_in_input_with_injections += injection.length;
        }
        self.offset_in_input_with_injections_to_output_position(
            offset_in_input_with_injections,
            affinity,
        )
    }

    fn offset_in_input_with_injections_to_output_position(
        &self,
        offset: u32,
        affinity: PositionAffinity,
    ) -> OutputPosition {
        let mut low: i64 = 0;
        let mut high: i64 = self.break_offsets.len() as i64 - 1;
        let mut mid: i64 = 0;
        let mut mid_start: u32 = 0;
        while low <= high {
            mid = low + (high - low) / 2;
            let mid_stop = self.break_offsets[mid as usize];
            mid_start = if mid > 0 {
                self.break_offsets[mid as usize - 1]
            } else {
                0
            };
            if affinity == PositionAffinity::Left {
                if offset <= mid_start {
                    high = mid - 1;
                } else if offset > mid_stop {
                    low = mid + 1;
                } else {
                    break;
                }
            } else if offset < mid_start {
                high = mid - 1;
            } else if offset >= mid_stop {
                low = mid + 1;
            } else {
                break;
            }
        }
        let mut output_offset = offset.saturating_sub(mid_start);
        if mid > 0 {
            output_offset += self.wrapped_text_indent_length;
        }
        OutputPosition {
            output_line_index: mid as u32,
            output_offset,
        }
    }

    /// Monaco `normalizeOutputPosition`: a view position moved out of
    /// injected text as `affinity` and the cursor stops say, and off a
    /// wrap point's ambiguous side.
    pub fn normalize_output_position(
        &self,
        output_line_index: u32,
        output_offset: u32,
        affinity: PositionAffinity,
    ) -> OutputPosition {
        if !self.injections.is_empty() {
            let offset = self.output_position_to_offset_in_input_with_injections(
                output_line_index,
                output_offset,
            );
            let normalized =
                self.normalize_offset_in_input_with_injections_around_injections(offset, affinity);
            if normalized != offset {
                // Injected text caused a change.
                return self
                    .offset_in_input_with_injections_to_output_position(normalized, affinity);
            }
        }
        if affinity == PositionAffinity::Left {
            if output_line_index > 0 && output_offset == self.min_output_offset(output_line_index) {
                return OutputPosition {
                    output_line_index: output_line_index - 1,
                    output_offset: self.max_output_offset(output_line_index - 1),
                };
            }
        } else if affinity == PositionAffinity::Right {
            let max_output_line_index = self.output_line_count() - 1;
            if output_line_index < max_output_line_index
                && output_offset == self.max_output_offset(output_line_index)
            {
                return OutputPosition {
                    output_line_index: output_line_index + 1,
                    output_offset: self.min_output_offset(output_line_index + 1),
                };
            }
        }
        OutputPosition {
            output_line_index,
            output_offset,
        }
    }

    fn output_position_to_offset_in_input_with_injections(
        &self,
        output_line_index: u32,
        output_offset: u32,
    ) -> u32 {
        let output_offset = if output_line_index > 0 {
            output_offset.saturating_sub(self.wrapped_text_indent_length)
        } else {
            output_offset
        };
        let start = if output_line_index > 0 {
            self.break_offsets[output_line_index as usize - 1]
        } else {
            0
        };
        start + output_offset
    }

    fn normalize_offset_in_input_with_injections_around_injections(
        &self,
        offset: u32,
        affinity: PositionAffinity,
    ) -> u32 {
        let Some(injected) = self.injected_text_at_offset(offset) else {
            return offset;
        };
        let stops = |index: usize| self.injections[index].cursor_stops;
        match affinity {
            PositionAffinity::None => {
                if offset == injected.offset_in_input_with_injections + injected.length
                    && stops(injected.index).has_right()
                {
                    return injected.offset_in_input_with_injections + injected.length;
                }
                let mut result = injected.offset_in_input_with_injections;
                if stops(injected.index).has_left() {
                    return result;
                }
                let mut index = injected.index;
                while index > 0
                    && self.injections[index - 1].offset == self.injections[injected.index].offset
                {
                    index -= 1;
                    if stops(index).has_right() {
                        break;
                    }
                    result -= self.injections[index].length;
                    if stops(index).has_left() {
                        break;
                    }
                }
                result
            }
            PositionAffinity::Right | PositionAffinity::RightOfInjectedText => {
                let mut result = injected.offset_in_input_with_injections + injected.length;
                let mut index = injected.index;
                // Through all the injected text touching it.
                while index + 1 < self.injections.len()
                    && self.injections[index + 1].offset == self.injections[index].offset
                {
                    result += self.injections[index + 1].length;
                    index += 1;
                }
                result
            }
            PositionAffinity::Left | PositionAffinity::LeftOfInjectedText => {
                let mut result = injected.offset_in_input_with_injections;
                let mut index = injected.index;
                // Through all the injected text touching it.
                while index > 0
                    && self.injections[index - 1].offset == self.injections[index].offset
                {
                    result -= self.injections[index - 1].length;
                    index -= 1;
                }
                result
            }
        }
    }

    /// Monaco `getInjectedText`: the injected text at a view position, by
    /// index into [`Self::injections`].
    pub fn injected_text(&self, output_line_index: u32, output_offset: u32) -> Option<usize> {
        let offset = self
            .output_position_to_offset_in_input_with_injections(output_line_index, output_offset);
        self.injected_text_at_offset(offset)
            .map(|injected| injected.index)
    }

    fn injected_text_at_offset(&self, offset: u32) -> Option<InjectedTextAt> {
        let mut total_injected_text_length_before = 0;
        for (index, injection) in self.injections.iter().enumerate() {
            let start = injection.offset + total_injected_text_length_before;
            let end = start + injection.length;
            if start > offset {
                // Injected text starts later, as all later injected text.
                break;
            }
            if offset <= end {
                // It ends after or at the offset (and starts at or before it).
                return Some(InjectedTextAt {
                    index,
                    offset_in_input_with_injections: start,
                    length: injection.length,
                });
            }
            total_injected_text_length_before += injection.length;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    //! Ports of Monaco's `lineBreakData.test.ts` (`Editor ViewModel -
    //! LineBreakData`), plus the cursor stops.

    use super::*;

    fn sequence(length: u32, start: u32) -> Vec<u32> {
        (start..start + length).collect()
    }

    fn injections(offsets: &[u32], texts: &[&str]) -> Vec<Injection> {
        offsets
            .iter()
            .zip(texts)
            .map(|(offset, text)| Injection {
                offset: *offset,
                length: text.encode_utf16().count() as u32,
                cursor_stops: InjectedTextCursorStops::Both,
            })
            .collect()
    }

    fn test_inverse(data: &ModelLineProjectionData) {
        for i in 0..100 {
            let output = data.translate_to_output_position(i, PositionAffinity::None);
            assert_eq!(
                data.translate_to_input_offset(output.output_line_index, output.output_offset),
                i
            );
        }
    }

    fn input_offsets(data: &ModelLineProjectionData, output_line_index: u32) -> Vec<u32> {
        sequence(20, 0)
            .into_iter()
            .map(|i| data.translate_to_input_offset(output_line_index, i))
            .collect()
    }

    fn output_offsets(data: &ModelLineProjectionData, affinity: PositionAffinity) -> Vec<String> {
        sequence(25, 0)
            .into_iter()
            .map(|i| data.translate_to_output_position(i, affinity).to_string())
            .collect()
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn basic() {
        let data = ModelLineProjectionData::new(Vec::new(), vec![100], Vec::new(), 10);
        assert_eq!(data.translate_to_input_offset(0, 50), 50);
        assert_eq!(data.translate_to_input_offset(1, 60), 150);
    }

    fn injected_text_1() -> ModelLineProjectionData {
        ModelLineProjectionData::new(
            injections(&[2, 3, 10], &["1", "22", "333"]),
            vec![10, 100],
            Vec::new(),
            10,
        )
    }

    #[test]
    fn injected_text_1_input_offset_of_output_position() {
        let data = injected_text_1();
        assert_eq!(
            input_offsets(&data, 0),
            [
                0, 1, 2, 2, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 10, 10, 10, 11, 12, 13
            ]
        );
        assert_eq!(
            input_offsets(&data, 1),
            [
                7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 9, 10, 10, 10, 10, 11, 12, 13
            ]
        );
    }

    #[test]
    fn injected_text_1_output_position_of_input_offset() {
        let data = injected_text_1();
        assert_eq!(
            output_offsets(&data, PositionAffinity::None),
            strings(&[
                "0:0", "0:1", "0:2", "0:4", "0:7", "0:8", "0:9", "1:10", "1:11", "1:12", "1:13",
                "1:17", "1:18", "1:19", "1:20", "1:21", "1:22", "1:23", "1:24", "1:25", "1:26",
                "1:27", "1:28", "1:29", "1:30",
            ])
        );
        assert_eq!(
            output_offsets(&data, PositionAffinity::Left),
            strings(&[
                "0:0", "0:1", "0:2", "0:4", "0:7", "0:8", "0:9", "0:10", "1:11", "1:12", "1:13",
                "1:17", "1:18", "1:19", "1:20", "1:21", "1:22", "1:23", "1:24", "1:25", "1:26",
                "1:27", "1:28", "1:29", "1:30",
            ])
        );
        assert_eq!(
            output_offsets(&data, PositionAffinity::Right),
            strings(&[
                "0:0", "0:1", "0:3", "0:6", "0:7", "0:8", "0:9", "1:10", "1:11", "1:12", "1:16",
                "1:17", "1:18", "1:19", "1:20", "1:21", "1:22", "1:23", "1:24", "1:25", "1:26",
                "1:27", "1:28", "1:29", "1:30",
            ])
        );
    }

    #[test]
    fn injected_text_1_inverse() {
        test_inverse(&injected_text_1());
    }

    #[test]
    fn injected_text_1_normalization() {
        let data = injected_text_1();
        let normalized: Vec<String> = sequence(25, 0)
            .into_iter()
            .map(|v| {
                data.normalize_output_position(1, v, PositionAffinity::Right)
                    .to_string()
            })
            .collect();
        assert_eq!(
            normalized,
            strings(&[
                "1:0", "1:1", "1:2", "1:3", "1:4", "1:5", "1:6", "1:7", "1:8", "1:9", "1:10",
                "1:11", "1:12", "1:16", "1:16", "1:16", "1:16", "1:17", "1:18", "1:19", "1:20",
                "1:21", "1:22", "1:23", "1:24",
            ])
        );
    }

    #[test]
    fn injected_text_2() {
        let data = ModelLineProjectionData::new(
            injections(&[2, 2, 6], &["1", "22", "333"]),
            vec![10, 100],
            Vec::new(),
            0,
        );
        assert_eq!(
            input_offsets(&data, 0),
            [
                0, 1, 2, 2, 2, 2, 3, 4, 5, 6, 6, 6, 6, 7, 8, 9, 10, 11, 12, 13
            ]
        );
        assert_eq!(
            input_offsets(&data, 1),
            [
                6, 6, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23
            ]
        );
        test_inverse(&data);
    }

    #[test]
    fn injected_text_3() {
        let data = ModelLineProjectionData::new(
            injections(&[2, 2, 7], &["1", "22", "333"]),
            vec![10, 100],
            Vec::new(),
            0,
        );
        assert_eq!(
            input_offsets(&data, 0),
            [
                0, 1, 2, 2, 2, 2, 3, 4, 5, 6, 7, 7, 7, 7, 8, 9, 10, 11, 12, 13
            ]
        );
        assert_eq!(
            input_offsets(&data, 1),
            [
                7, 7, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23
            ]
        );
        test_inverse(&data);
    }

    /// Monaco `viewModelImpl.test.ts`, "normalizePosition with multiple
    /// touching injected text": `just sobarbzme text` (1-based columns).
    #[test]
    fn normalize_position_with_multiple_touching_injected_text() {
        let data = ModelLineProjectionData::unwrapped(
            "just some text".len() as u32,
            injections(&[7, 7], &["bar", "bz"]),
        );
        let normalize = |column: u32, affinity| {
            data.normalize_output_position(0, column - 1, affinity)
                .output_offset
                + 1
        };
        let columns = [8, 9, 11, 12, 13];
        assert_eq!(
            columns.map(|c| normalize(c, PositionAffinity::None)),
            [8, 8, 11, 11, 13]
        );
        assert_eq!(
            columns.map(|c| normalize(c, PositionAffinity::Left)),
            [8, 8, 8, 8, 8]
        );
        assert_eq!(
            columns.map(|c| normalize(c, PositionAffinity::Right)),
            [13, 13, 13, 13, 13]
        );
    }

    /// `ab` with "XY" injected between the letters.
    fn stops(cursor_stops: InjectedTextCursorStops) -> ModelLineProjectionData {
        ModelLineProjectionData::unwrapped(
            2,
            vec![Injection {
                offset: 1,
                length: 2,
                cursor_stops,
            }],
        )
    }

    /// Monaco `normalizeOffsetInInputWithInjectionsAroundInjections` with
    /// `None`: inside or at the edges of the text, the cursor goes to a
    /// stop.
    #[test]
    fn cursor_stops_normalize() {
        use InjectedTextCursorStops::*;
        let normalize = |data: &ModelLineProjectionData, offset| {
            data.normalize_output_position(0, offset, PositionAffinity::None)
                .output_offset
        };
        // Both: either edge; inside, the left one.
        assert_eq!([1, 2, 3].map(|o| normalize(&stops(Both), o)), [1, 1, 3]);
        // Right: the right edge stays; the rest go left (no injected text
        // before to skip).
        assert_eq!([1, 2, 3].map(|o| normalize(&stops(Right), o)), [1, 1, 3]);
        // Left and None: the left edge.
        assert_eq!([1, 2, 3].map(|o| normalize(&stops(Left), o)), [1, 1, 1]);
        assert_eq!([1, 2, 3].map(|o| normalize(&stops(None), o)), [1, 1, 1]);
    }

    /// Where the cursor shows after Monaco's `MoveOperations.right` steps
    /// onto the injected text's offset: `rightPosition` normalizes with
    /// `Right`, then the cursor validates it (`normalizePosition` with
    /// `None`).
    #[test]
    fn cursor_after_moving_onto_injected_text() {
        use InjectedTextCursorStops::*;
        let after_right = |data: &ModelLineProjectionData| {
            let landed = data.normalize_output_position(0, 1, PositionAffinity::Right);
            data.normalize_output_position(0, landed.output_offset, PositionAffinity::None)
                .output_offset
        };
        for (stops_, expected) in [(Both, 3), (Right, 3), (Left, 1), (None, 1)] {
            assert_eq!(after_right(&stops(stops_)), expected, "{stops_:?}");
        }
        // Moving up lands before it.
        assert_eq!(
            stops(Both)
                .normalize_output_position(0, 2, PositionAffinity::LeftOfInjectedText)
                .output_offset,
            1
        );
        // A click in the text goes to a stop.
        let click = |data: &ModelLineProjectionData, offset| {
            data.normalize_output_position(0, offset, PositionAffinity::None)
                .output_offset
        };
        assert_eq!(click(&stops(Both), 2), 1);
        assert_eq!(click(&stops(Both), 3), 3);
        assert_eq!(click(&stops(Left), 3), 1);
    }

    /// Touching injected text (inlay hint parts): with `None` stops between
    /// the parts and `Right` after the last, the cursor skips to the end or
    /// the start of them all.
    #[test]
    fn cursor_stops_across_touching_injected_text() {
        use InjectedTextCursorStops::*;
        let data = ModelLineProjectionData::unwrapped(
            2,
            vec![
                Injection {
                    offset: 1,
                    length: 2,
                    cursor_stops: None,
                },
                Injection {
                    offset: 1,
                    length: 3,
                    cursor_stops: Right,
                },
            ],
        );
        let normalize = |offset, affinity| {
            data.normalize_output_position(0, offset, affinity)
                .output_offset
        };
        assert_eq!(normalize(1, PositionAffinity::Right), 6);
        assert_eq!(normalize(6, PositionAffinity::None), 6);
        // Between the parts goes back to before them.
        assert_eq!(normalize(3, PositionAffinity::None), 1);
        assert_eq!(normalize(5, PositionAffinity::None), 1);
    }

    /// Monaco `LineInjectedText.applyInjectedText`.
    #[test]
    fn apply_injected_text() {
        let text = |column, content: &str| LineInjectedText {
            column,
            content: content.to_string().into(),
            cursor_stops: InjectedTextCursorStops::Both,
            tokens: None,
        };
        assert_eq!(
            LineInjectedText::apply("just some text", &[text(7, "bar"), text(7, "bz")]),
            "just sobarbzme text"
        );
        assert_eq!(
            LineInjectedText::apply("a\u{1f600}b", &[text(3, "X"), text(4, "Y")]),
            "a\u{1f600}XbY"
        );
    }
}
