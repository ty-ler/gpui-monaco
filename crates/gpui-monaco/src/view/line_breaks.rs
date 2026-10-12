//! Soft-wrap break computation for monospace text.
//!
//! Ported from Monaco's `common/viewModel/monospaceLineBreaksComputer.ts`
//! (MIT) and `modelLineProjectionData.ts`. Offsets are UTF-16 code units
//! within the line (with its injected text, in [`create_line_breaks`]),
//! widths are visible columns (full-width characters take two).

use super::injected_text::{LineInjectedText, ModelLineProjectionData};
use crate::cursor::strings::is_full_width_character;

/// Monaco `wordWrapBreakBeforeCharacters` default.
pub const DEFAULT_BREAK_BEFORE: &str = "([{‘“〈《「『【〔（［｛｢£¥＄￡￥+＋";
/// Monaco `wordWrapBreakAfterCharacters` default.
pub const DEFAULT_BREAK_AFTER: &str = " \t})]?|/&.,;¢°′″‰℃、。｡､￠，．：；？！％・･ゝゞヽヾーァィゥェォッャュョヮヵヶぁぃぅぇぉっゃゅょゎゕゖㇰㇱㇲㇳㇴㇵㇶㇷㇸㇹㇺㇻㇼㇽㇾㇿ々〻ｧｨｩｪｫｬｭｮｯｰ”〉》」』】〕）］｝｣";

/// Monaco `wrappingIndent`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WrappingIndent {
    /// Continuation lines start at column 0.
    None,
    /// Continuation lines get the wrapped line's indentation.
    #[default]
    Same,
    /// Continuation lines get the wrapped line's indentation plus one tab
    /// stop.
    Indent,
    /// Continuation lines get the wrapped line's indentation plus two tab
    /// stops.
    DeepIndent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharClass {
    None,
    BreakBefore,
    BreakAfter,
    BreakIdeographic,
}

/// Classifies UTF-16 code units for wrapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrapClassifier {
    ascii: [CharClass; 256],
    other: Vec<(u32, CharClass)>,
}

impl Default for WrapClassifier {
    fn default() -> Self {
        Self::new(DEFAULT_BREAK_BEFORE, DEFAULT_BREAK_AFTER)
    }
}

impl WrapClassifier {
    /// A classifier that breaks before any character of `break_before` and
    /// after any character of `break_after` (Monaco
    /// `editor.wordWrapBreakBeforeCharacters`/`wordWrapBreakAfterCharacters`),
    /// and around CJK ideographs and kana. A character in both sets breaks
    /// after.
    pub fn new(break_before: &str, break_after: &str) -> Self {
        let mut ascii = [CharClass::None; 256];
        let mut other = Vec::new();
        let mut set = |code: u32, class: CharClass| {
            if code < 256 {
                ascii[code as usize] = class;
            } else {
                other.retain(|(c, _)| *c != code);
                other.push((code, class));
            }
        };
        for unit in break_before.encode_utf16() {
            set(unit as u32, CharClass::BreakBefore);
        }
        for unit in break_after.encode_utf16() {
            set(unit as u32, CharClass::BreakAfter);
        }
        Self { ascii, other }
    }

    fn get(&self, code: u32) -> CharClass {
        if code < 256 {
            return self.ascii[code as usize];
        }
        if (0x3040..=0x30FF).contains(&code)
            || (0x3400..=0x4DBF).contains(&code)
            || (0x4E00..=0x9FFF).contains(&code)
        {
            return CharClass::BreakIdeographic;
        }
        self.other
            .iter()
            .find(|(c, _)| *c == code)
            .map_or(CharClass::None, |(_, class)| *class)
    }
}

/// Where one model line wraps. `break_offsets` ends with the line length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBreakData {
    /// UTF-16 offset in the model line where each output line ends.
    pub break_offsets: Vec<u32>,
    /// Visible column in the model line of each entry of `break_offsets`
    /// (without continuation indentation).
    pub break_offsets_visible_column: Vec<u32>,
    /// Visible columns of indentation added to continuation lines.
    pub wrapped_text_indent_length: u32,
}

impl LineBreakData {
    /// Number of output (view) lines the model line wraps into.
    pub fn output_line_count(&self) -> usize {
        self.break_offsets.len()
    }

    /// Input (model) offsets covered by output line `index`.
    pub fn input_range(&self, index: usize) -> std::ops::Range<u32> {
        let start = if index == 0 {
            0
        } else {
            self.break_offsets[index - 1]
        };
        start..self.break_offsets[index]
    }

    /// The output line containing `input_offset`. An offset exactly at a
    /// break belongs to the next line, unless `prefer_previous`.
    pub fn output_line_for(&self, input_offset: u32, prefer_previous: bool) -> usize {
        let ix = if prefer_previous {
            self.break_offsets.partition_point(|&b| b < input_offset)
        } else {
            self.break_offsets.partition_point(|&b| b <= input_offset)
        };
        ix.min(self.break_offsets.len() - 1)
    }
}

fn is_high_surrogate(unit: u32) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

fn char_width(code: u32, visible_column: u32, tab_size: u32) -> u32 {
    if code == '\t' as u32 {
        return tab_size - visible_column % tab_size;
    }
    if is_full_width_character(code) || code < 32 {
        return 2;
    }
    1
}

fn can_break(prev_code: u32, prev: CharClass, code: u32, class: CharClass, keep_all: bool) -> bool {
    let _ = prev_code;
    code != ' ' as u32
        && ((prev == CharClass::BreakAfter && class != CharClass::BreakAfter)
            || (prev != CharClass::BreakBefore && class == CharClass::BreakBefore)
            || (!keep_all && prev == CharClass::BreakIdeographic && class != CharClass::BreakAfter)
            || (!keep_all
                && class == CharClass::BreakIdeographic
                && prev != CharClass::BreakBefore))
}

fn wrapped_text_indent_length(
    line: &[u16],
    tab_size: u32,
    first_break_column: u32,
    indent: WrappingIndent,
) -> u32 {
    if indent == WrappingIndent::None {
        return 0;
    }
    let Some(first_non_ws) = line
        .iter()
        .position(|&u| u != b' ' as u16 && u != b'\t' as u16)
    else {
        return 0;
    };
    let mut length = 0;
    for &unit in &line[..first_non_ws] {
        length += if unit == b'\t' as u16 {
            tab_size - length % tab_size
        } else {
            1
        };
    }
    let extra_tabs = match indent {
        WrappingIndent::DeepIndent => 2,
        WrappingIndent::Indent => 1,
        _ => 0,
    };
    for _ in 0..extra_tabs {
        length += tab_size - length % tab_size;
    }
    // Stick to the start if nothing but the indentation would fit.
    if length + 2 > first_break_column {
        length = 0;
    }
    length
}

/// Character widths for wrapping by measured text instead of monospace
/// columns (Monaco `wrappingStrategy: advanced`).
pub trait MeasureChar: Send + Sync {
    /// `ch`'s width in columns: its advance over the typical half-width
    /// character's.
    fn columns(&self, ch: char) -> f32;
    /// The widest printable ASCII character, in columns.
    fn max_ascii_columns(&self) -> f32;
}

/// Computes wrap points for `line` at `wrapping_column` visible columns, or
/// `None` if it fits (or wrapping is off: `wrapping_column == None`).
pub fn compute_line_breaks(
    classifier: &WrapClassifier,
    line: &str,
    tab_size: u32,
    wrapping_column: Option<u32>,
    wrapping_indent: WrappingIndent,
    keep_all: bool,
) -> Option<LineBreakData> {
    compute_measured_line_breaks(
        classifier,
        line,
        tab_size,
        wrapping_column,
        wrapping_indent,
        keep_all,
        false,
        None,
    )
}

/// Monaco `isEscapedLineBreakAtPosition`: whether `units[i]` follows a
/// literal `\n` (an `n` after an odd number of backslashes).
fn is_escaped_line_break_at(units: &[u16], i: usize) -> bool {
    if i < 2 || units[i - 1] != b'n' as u16 {
        return false;
    }
    let backslashes = units[..i - 1]
        .iter()
        .rev()
        .take_while(|&&u| u == b'\\' as u16)
        .count();
    // Monaco only counts backslashes with a character before them.
    backslashes < i - 1 && backslashes % 2 == 1
}

/// [`compute_line_breaks`], with widths from `measure` when given: Monaco's
/// monospace computer with the DOM computer's measured widths. Breaks
/// still follow the classifier, and visible columns stay monospace.
///
/// With `wrap_on_escaped_line_feeds` (Monaco `wrapOnEscapedLineFeeds`), a
/// line containing `"` and a literal `\n` also breaks after each `\n`.
#[allow(clippy::too_many_arguments)]
pub fn compute_measured_line_breaks(
    classifier: &WrapClassifier,
    line: &str,
    tab_size: u32,
    wrapping_column: Option<u32>,
    wrapping_indent: WrappingIndent,
    keep_all: bool,
    wrap_on_escaped_line_feeds: bool,
    measure: Option<&dyn MeasureChar>,
) -> Option<LineBreakData> {
    let wrap_on_escaped_line_feeds =
        wrap_on_escaped_line_feeds && line.contains('"') && line.contains("\\n");
    line_breaks(
        classifier,
        line,
        tab_size,
        wrapping_column?,
        wrapping_indent,
        keep_all,
        wrap_on_escaped_line_feeds,
        measure,
    )
}

/// Monaco `createLineBreaks`: where `line` wraps with `injected` text (sorted
/// by column) in it, as the projection of the line with its injected text
/// (break offsets count the injected text, which may wrap mid-text), or
/// `None` if it fits without any. Whether a literal `\n` breaks the line
/// (`wrap_on_escaped_line_feeds`) is decided on the model line, as in
/// Monaco. With `wrapping_column == None` only the injected text is
/// projected.
#[allow(clippy::too_many_arguments)]
pub fn create_line_breaks(
    classifier: &WrapClassifier,
    line: &str,
    injected: &[LineInjectedText],
    tab_size: u32,
    wrapping_column: Option<u32>,
    wrapping_indent: WrappingIndent,
    keep_all: bool,
    wrap_on_escaped_line_feeds: bool,
    measure: Option<&dyn MeasureChar>,
) -> Option<ModelLineProjectionData> {
    if injected.is_empty() {
        return compute_measured_line_breaks(
            classifier,
            line,
            tab_size,
            wrapping_column,
            wrapping_indent,
            keep_all,
            wrap_on_escaped_line_feeds,
            measure,
        )
        .map(|breaks| ModelLineProjectionData::from_line_breaks(Vec::new(), breaks));
    }
    let injections = LineInjectedText::injections(injected);
    let line_length = line.encode_utf16().count() as u32;
    let Some(wrapping_column) = wrapping_column else {
        return Some(ModelLineProjectionData::unwrapped(line_length, injections));
    };
    let wrap_on_escaped_line_feeds =
        wrap_on_escaped_line_feeds && line.contains('"') && line.contains("\\n");
    let text = LineInjectedText::apply(line, injected);
    match line_breaks(
        classifier,
        &text,
        tab_size,
        wrapping_column,
        wrapping_indent,
        keep_all,
        wrap_on_escaped_line_feeds,
        measure,
    ) {
        Some(breaks) => Some(ModelLineProjectionData::from_line_breaks(
            injections, breaks,
        )),
        None => Some(ModelLineProjectionData::unwrapped(line_length, injections)),
    }
}

/// [`compute_measured_line_breaks`] at `first_break_column`, with
/// `wrap_on_escaped_line_feeds` already decided for the line.
#[allow(clippy::too_many_arguments)]
fn line_breaks(
    classifier: &WrapClassifier,
    line: &str,
    tab_size: u32,
    first_break_column: u32,
    wrapping_indent: WrappingIndent,
    keep_all: bool,
    wrap_on_escaped_line_feeds: bool,
    measure: Option<&dyn MeasureChar>,
) -> Option<LineBreakData> {
    // Fast path for the common short line: without tabs or other control
    // characters (2 columns wide) no character is wider than its UTF-8
    // length (or the widest ASCII character), so it fits.
    let widest = measure.map_or(1., |m| m.max_ascii_columns().max(1.));
    if !wrap_on_escaped_line_feeds
        && line.len() as f32 * widest <= first_break_column as f32
        && line
            .bytes()
            .all(|b| (b' '..0x80).contains(&b) || (measure.is_none() && b >= b' '))
    {
        return None;
    }
    let units: Vec<u16> = line.encode_utf16().collect();
    let len = units.len();
    if len <= 1 {
        return None;
    }
    let tab_size = tab_size.max(1);
    let indent = wrapped_text_indent_length(&units, tab_size, first_break_column, wrapping_indent);
    // Monaco's DOM computer indents by space widths.
    let space = measure.map_or(1., |m| m.columns(' '));
    // A character's width: its columns, or its measured width (tabs and
    // control characters are spaces).
    let measured = |code: u32, columns: u32| -> f32 {
        match (measure, char::from_u32(code)) {
            (Some(m), Some(ch)) if code >= 32 => m.columns(ch),
            (Some(_), _) => columns as f32 * space,
            (None, _) => columns as f32,
        }
    };
    let wrapped_line_break_width = first_break_column as f32 - indent as f32 * space;

    let mut offsets: Vec<u32> = Vec::new();
    let mut offsets_visible: Vec<u32> = Vec::new();
    let mut break_offset = 0usize;
    let mut break_offset_visible = 0u32;
    let mut break_offset_width = 0f32;
    let mut breaking_width = first_break_column as f32;

    let mut prev_code = units[0] as u32;
    let mut visible = char_width(prev_code, 0, tab_size);
    let mut width = measured(prev_code, visible);
    let mut start = 1;
    if is_high_surrogate(prev_code) {
        let pair = char::decode_utf16([units[0], units[1]])
            .next()
            .and_then(Result::ok)
            .map_or(0, u32::from);
        visible += 1;
        width = measured(pair, 2);
        prev_code = units[1] as u32;
        start = 2;
    }
    let mut prev_class = classifier.get(prev_code);

    let mut i = start;
    while i < len {
        let char_start = i;
        let code = units[i] as u32;
        let (class, char_w, char_width_measured) = if is_high_surrogate(code) {
            let pair = char::decode_utf16([units[i], units.get(i + 1).copied().unwrap_or(0)])
                .next()
                .and_then(Result::ok)
                .map_or(0, u32::from);
            i += 1;
            (CharClass::None, 2, measured(pair, 2))
        } else {
            let char_w = char_width(code, visible, tab_size);
            (classifier.get(code), char_w, measured(code, char_w))
        };

        // A literal `\n` forces a break.
        let mut wrap_escaped_line_feed = false;
        if wrap_on_escaped_line_feeds && is_escaped_line_break_at(&units, char_start) {
            break_offset = char_start;
            break_offset_visible = visible;
            break_offset_width = width;
            wrap_escaped_line_feed = true;
        } else if can_break(prev_code, prev_class, code, class, keep_all) {
            break_offset = char_start;
            break_offset_visible = visible;
            break_offset_width = width;
        }

        visible += char_w;
        width += char_width_measured;

        if width > breaking_width || wrap_escaped_line_feed {
            if break_offset == 0 || width - break_offset_width > wrapped_line_break_width {
                break_offset = char_start;
                break_offset_visible = visible - char_w;
                break_offset_width = width - char_width_measured;
            }
            let line_start = offsets.last().copied().unwrap_or(0) as usize;
            if break_offset > line_start {
                offsets.push(break_offset as u32);
                offsets_visible.push(break_offset_visible);
                breaking_width = break_offset_width + wrapped_line_break_width;
                break_offset = 0;
            }
        }

        prev_code = code;
        prev_class = class;
        i += 1;
    }

    if offsets.is_empty() {
        return None;
    }
    offsets.push(len as u32);
    offsets_visible.push(visible);
    Some(LineBreakData {
        break_offsets: offsets,
        break_offsets_visible_column: offsets_visible,
        wrapped_text_indent_length: indent,
    })
}

#[cfg(test)]
mod tests {
    //! Ported from Monaco's
    //! `test/common/viewModel/monospaceLineBreaksComputer.test.ts`. Skipped:
    //! the injected-text cases, which are all about `widthInEm` (not
    //! ported; injected text with content is tested below), incremental
    //! re-wrapping (we always recompute), and fractional full-width ratios
    //! (we assume two-cell full-width characters).
    use super::*;

    fn assert_line_breaks(
        classifier: &WrapClassifier,
        tab_size: u32,
        break_after: i32,
        annotated: &str,
        indent: WrappingIndent,
        keep_all: bool,
    ) -> Option<LineBreakData> {
        let text: String = annotated.chars().filter(|&c| c != '|').collect();
        let column = (break_after >= 0).then_some(break_after as u32);
        let data = compute_line_breaks(classifier, &text, tab_size, column, indent, keep_all);
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut actual = String::new();
        let mut prev_line = 0;
        let mut ix = 0;
        while ix < units.len() {
            let ch_len = if is_high_surrogate(units[ix] as u32) {
                2
            } else {
                1
            };
            if let Some(data) = &data {
                let line = data.output_line_for(ix as u32, false);
                if line != prev_line {
                    prev_line = line;
                    actual.push('|');
                }
            }
            actual.push_str(&String::from_utf16_lossy(&units[ix..ix + ch_len]));
            ix += ch_len;
        }
        assert_eq!(actual, annotated);
        data
    }

    fn basic(annotated: &str) {
        let classifier = WrapClassifier::new("(", "\t).");
        assert_line_breaks(&classifier, 4, 5, annotated, WrappingIndent::None, false);
    }

    #[test]
    fn monospace_line_breaks_computer() {
        basic("");
        basic("aaa");
        basic("aaaaa");
        let classifier = WrapClassifier::new("(", "\t).");
        assert_line_breaks(
            &classifier,
            4,
            -1,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            WrappingIndent::None,
            false,
        );
        basic("aaaaa|a");
        basic("aaaaa|.");
        basic("aaaaa|a.|aaa.|aa");
        basic("aaaaa|a..|aaa.|aa");
        basic("aaaaa|a...|aaa.|aa");
        basic("aaaaa|a....|aaa.|aa");
        basic("\t");
        basic("\t|aaa");
        basic("\t|a\t|aa");
        basic("aa\ta");
        basic("aa\t|aa");
        basic("aaa.|aa");
        basic("aaa(.|aa");
        basic("aaa))|).aaa");
        basic("aaa))|).|aaaa");
        basic("aaa)|().|aaa");
        basic("aaa|(().|aaa");
        basic("aa.|(().|aaa");
        basic("aa.|(.).|aaa");
    }

    #[test]
    fn cjk_and_kinsoku_shori() {
        let c = WrapClassifier::new("(", "\t)");
        for text in [
            "aa \u{5b89}|\u{5b89}",
            "\u{3042} \u{5b89}|\u{5b89}",
            "\u{3042}\u{3042}|\u{5b89}\u{5b89}",
            "aa |\u{5b89})\u{5b89}|\u{5b89}",
            "aa \u{3042}|\u{5b89}\u{3042})|\u{5b89}",
            "aa |(\u{5b89}aa|\u{5b89}",
        ] {
            assert_line_breaks(&c, 4, 5, text, WrappingIndent::None, false);
        }
    }

    #[test]
    fn wrapping_indent_same() {
        let c = WrapClassifier::new("", "\t ");
        assert_line_breaks(
            &c,
            4,
            38,
            " *123456789012345678901234567890123456|7890",
            WrappingIndent::Same,
            false,
        );
    }

    #[test]
    fn issue_16332_scroll_bar_overlaying_on_top_of_text() {
        let c = WrapClassifier::new("", "\t ");
        assert_line_breaks(
            &c,
            4,
            24,
            "a/ very/long/line/of/tex|t/that/expands/beyon|d/your/typical/line/|of/code/",
            WrappingIndent::Indent,
            false,
        );
    }

    #[test]
    fn issue_35162_wrapping_indent_not_consistently_working() {
        let c = WrapClassifier::new("", "\t ");
        let data = assert_line_breaks(
            &c,
            4,
            24,
            "                t h i s |i s |a l |o n |g l |i n |e",
            WrappingIndent::Indent,
            false,
        )
        .unwrap();
        assert_eq!(data.wrapped_text_indent_length, 20);
    }

    #[test]
    fn issue_75494_surrogate_pairs() {
        let c = WrapClassifier::new("\t", " ");
        assert_line_breaks(
            &c,
            4,
            49,
            "🐇👬🌖🌞🏇🍼🐇👬🌖🌞🏇🍼🐇👬🌖🌞🏇🍼🐇👬🌖🌞🏇🍼|🐇👬🌖🌞🏇🍼🐇👬🌖🌞🏇🍼🐇👬🌖🌞🏇🍼🐇👬🌖🌞🏇🍼|🐇👬",
            WrappingIndent::Same,
            false,
        );
    }

    #[test]
    fn issue_75494_surrogate_pairs_overrun() {
        let c = WrapClassifier::default();
        assert_line_breaks(&c, 4, 4, "🐇👬|&|🌞🌖", WrappingIndent::Same, false);
        assert_line_breaks(
            &c,
            4,
            17,
            "factory, |\"xtxtfunc|(x\"🌞🏇🍼🌞🏇🍼🐇|&👬🌖🌞👬🌖🌞🏇🍼|🐇👬x\"",
            WrappingIndent::Same,
            false,
        );
    }

    #[test]
    fn wrapping_indent_deep_indent() {
        let c = WrapClassifier::new("", "\t ");
        let data = assert_line_breaks(
            &c,
            4,
            26,
            "        W e A r e T e s t |i n g D e |e p I n d |e n t a t |i o n",
            WrappingIndent::DeepIndent,
            false,
        )
        .unwrap();
        assert_eq!(data.wrapped_text_indent_length, 16);
    }

    #[test]
    fn issue_33366_and_152773_punctuation() {
        let c = WrapClassifier::default();
        assert_line_breaks(
            &c,
            4,
            23,
            "this is a line of |text, text that sits |on a line",
            WrappingIndent::Same,
            false,
        );
        assert_line_breaks(
            &c,
            4,
            24,
            "this is a line of |(text), text that sits |on a line",
            WrappingIndent::Same,
            false,
        );
    }

    #[test]
    fn issue_112382_control_characters() {
        let c = WrapClassifier::default();
        assert_line_breaks(
            &c,
            4,
            6,
            "\x06\x06\x06|\x06\x06\x06",
            WrappingIndent::Same,
            false,
        );
    }

    #[test]
    fn cjk_word_break_normal_and_keep_all() {
        let c = WrapClassifier::default();
        assert_line_breaks(&c, 4, 5, "你好|1111", WrappingIndent::Same, false);
        assert_line_breaks(&c, 4, 8, "你好1111", WrappingIndent::Same, true);
    }

    #[test]
    fn issue_95686_loop_forever() {
        let c = WrapClassifier::default();
        assert_line_breaks(
            &c,
            4,
            179,
            "\t\t\t\t\t\t<tr dmx-class:table-danger=\"(alt <= 50)\" dmx-class:table-warning=\"(alt <= 200)\" dmx-class:table-primary=\"(alt <= 400)\" dmx-class:table-info=\"(alt <= 800)\" |dmx-class:table-success=\"(alt >= 400)\">",
            WrappingIndent::Same,
            false,
        );
    }

    /// Narrow `i`, wide `W`, everything else a column.
    struct Proportional;

    impl MeasureChar for Proportional {
        fn columns(&self, ch: char) -> f32 {
            match ch {
                'i' => 0.5,
                'W' => 2.,
                _ => 1.,
            }
        }

        fn max_ascii_columns(&self) -> f32 {
            2.
        }
    }

    fn measured(line: &str, column: u32) -> Vec<u32> {
        compute_measured_line_breaks(
            &WrapClassifier::default(),
            line,
            4,
            Some(column),
            WrappingIndent::None,
            false,
            false,
            Some(&Proportional),
        )
        .map_or_else(Vec::new, |data| data.break_offsets)
    }

    /// `text` with `|` at each break, with `wrapOnEscapedLineFeeds`.
    fn escaped_line_feed_breaks(text: &str, column: u32) -> String {
        let data = compute_measured_line_breaks(
            &WrapClassifier::default(),
            text,
            4,
            Some(column),
            WrappingIndent::None,
            false,
            true,
            None,
        );
        let mut annotated = text.to_string();
        if let Some(data) = data {
            for &offset in data.break_offsets[..data.break_offsets.len() - 1]
                .iter()
                .rev()
            {
                annotated.insert(offset as usize, '|');
            }
        }
        annotated
    }

    #[test]
    fn issue_258022_wrap_on_escaped_line_feeds() {
        // Monaco: a literal `\n` triggers a soft wrap, whatever the width;
        // an escaped backslash before `n` doesn't.
        let text = r#""Short text with\nescaped newline and an escaped\\nbackslash""#;
        let at_30 = escaped_line_feed_breaks(text, 30);
        assert!(at_30.contains(r"with\n|"), "{at_30}");
        assert!(!at_30.contains(r"\\n|"), "{at_30}");
        let at_35 = escaped_line_feed_breaks(text, 35);
        assert!(at_35.contains(r"with\n|"), "{at_35}");
        // Short lines break too, but only with a quote on the line.
        assert_eq!(escaped_line_feed_breaks(r#""a\nb""#, 100), r#""a\n|b""#);
        assert_eq!(escaped_line_feed_breaks(r"a\nb", 100), r"a\nb");
        // Off: no break.
        assert_eq!(
            compute_line_breaks(
                &WrapClassifier::default(),
                r#""a\nb""#,
                4,
                Some(100),
                WrappingIndent::None,
                false
            ),
            None
        );
    }

    #[test]
    fn custom_break_characters() {
        // Monaco `wordWrapBreakBeforeCharacters`/`AfterCharacters`: with no
        // break characters, words break where they overflow.
        let none = WrapClassifier::new("", "");
        let data = compute_line_breaks(
            &none,
            "aaa bbb ccc",
            4,
            Some(5),
            WrappingIndent::None,
            false,
        )
        .unwrap();
        assert_eq!(data.break_offsets, vec![5, 10, 11]);
        let spaces = WrapClassifier::new("", " ");
        let data = compute_line_breaks(
            &spaces,
            "aaa bbb ccc",
            4,
            Some(5),
            WrappingIndent::None,
            false,
        )
        .unwrap();
        assert_eq!(data.break_offsets, vec![4, 8, 11]);
    }

    #[test]
    fn measured_widths_wrap_proportional_text() {
        // Twelve narrow characters fit in six columns.
        assert_eq!(measured("iiiiiiiiiiii", 6), Vec::<u32>::new());
        assert_eq!(measured("iiii iiii iiii", 6), vec![10, 14]);
        // Wide characters wrap sooner than their character count says.
        assert_eq!(measured("WW WW WW", 6), vec![3, 6, 8]);
        assert_eq!(measured("WWWW", 6), vec![3, 4]);
        // Monospace columns are unchanged.
        assert_eq!(
            compute_line_breaks(
                &WrapClassifier::default(),
                "iiiiiiiiiiii",
                4,
                Some(6),
                WrappingIndent::None,
                false
            )
            .map(|d| d.break_offsets),
            Some(vec![6, 12])
        );
    }

    fn injected(column: u32, content: &str) -> LineInjectedText {
        LineInjectedText {
            column,
            content: content.to_string().into(),
            cursor_stops: crate::decorations::InjectedTextCursorStops::Both,
        }
    }

    fn with_injected(
        line: &str,
        texts: &[LineInjectedText],
        column: Option<u32>,
        wrap_on_escaped_line_feeds: bool,
    ) -> Option<ModelLineProjectionData> {
        create_line_breaks(
            &WrapClassifier::default(),
            line,
            texts,
            4,
            column,
            WrappingIndent::None,
            false,
            wrap_on_escaped_line_feeds,
            None,
        )
    }

    /// Monaco `createLineBreaks` with injected text: it counts when
    /// wrapping, break offsets are in the line with its injected text, and
    /// a line with injected text is projected even when it fits.
    #[test]
    fn injected_text_counts_when_wrapping() {
        assert_eq!(with_injected("aaaa bbbb", &[], Some(10), false), None);
        let data = with_injected("aaaa bbbb", &[injected(4, ": hint")], Some(10), false).unwrap();
        // `aaaa: |hint bbbb`: `hint bbbb` doesn't fit after it.
        assert_eq!(data.break_offsets(), &[6, 15]);
        assert_eq!(data.injections().len(), 1);
        let data = with_injected("aaaa", &[injected(4, "!")], Some(10), false).unwrap();
        assert_eq!(data.break_offsets(), &[5]);
        // Unwrapped, it's just projected.
        let data = with_injected("aaaa bbbb", &[injected(4, ": hint")], None, false).unwrap();
        assert_eq!(data.break_offsets(), &[15]);
        // Long injected text wraps inside itself.
        let data = with_injected("ab", &[injected(1, "xxxxxxxxxxxxxxx")], Some(5), false).unwrap();
        assert_eq!(data.break_offsets(), &[5, 10, 15, 17]);
    }

    /// Monaco decides `wrapOnEscapedLineFeeds` on the model line, not the
    /// injected text.
    #[test]
    fn escaped_line_feeds_are_decided_on_the_model_line() {
        let quote = injected(0, "\"");
        assert_eq!(
            with_injected(r"a\nb", &[quote], Some(100), true)
                .unwrap()
                .break_offsets(),
            &[5]
        );
        let data = with_injected(r#""a\nb""#, &[injected(0, ">")], Some(100), true).unwrap();
        assert_eq!(data.break_offsets(), &[5, 7]);
    }
}
