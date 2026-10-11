//! Ported from Monaco's `contrib/smartSelect/test/browser/smartSelect.test.ts`.
//!
//! Monaco's `provideSelectionRanges` with no registered providers runs the
//! bracket provider and then the word provider and merges their ranges; here
//! that is [`bracket_selection_ranges`] followed by [`word_selection_ranges`],
//! fed to [`merge_selection_ranges`]. Monaco's test language has no
//! tokenizer, so no bracket is in a string or comment.
//!
//! Expected ranges keep Monaco's 1-based numbers.

use std::ops::Range;

use syntax::LanguageConfiguration;
use syntax::fancy_regex::Regex;

use super::language_configs::brackets;
use crate::cursor::Position;
use crate::cursor::smart_select::{
    bracket_selection_ranges, merge_selection_ranges, word_selection_ranges,
};

/// Monaco's test `wordPattern`
/// `/(-?\d*\.\d\w*)|([^\`\~\!\@\#\$\%\^\&\*\(\)\=\+\[\{\]\}\\\;\:\'\"\,\.\<\>\/\?\s]+)/g`,
/// without the escapes Rust's regex syntax rejects inside a class.
const WORD_PATTERN: &str = r#"(-?\d*\.\d\w*)|([^`~!@#$%^&*()=+\[{\]}\\;:'",.<>/?\s]+)"#;

fn word_regex() -> Regex {
    Regex::new(WORD_PATTERN).expect("word pattern compiles")
}

fn config() -> LanguageConfiguration {
    LanguageConfiguration {
        brackets: Some(brackets(&[("(", ")"), ("{", "}"), ("[", "]")])),
        ..Default::default()
    }
}

/// A Monaco 1-based range.
fn r(start_line: u32, start_column: u32, end_line: u32, end_column: u32) -> Range<Position> {
    Position::new(start_line - 1, start_column - 1)..Position::new(end_line - 1, end_column - 1)
}

fn bracket_ranges(model: &text::Rope, position: Position) -> Vec<Range<Position>> {
    bracket_selection_ranges(model, &config(), position, &|_| false)
}

fn word_ranges(
    model: &text::Rope,
    position: Position,
    select_subwords: bool,
) -> Vec<Range<Position>> {
    word_selection_ranges(model, &word_regex(), position, select_subwords)
}

/// Monaco `assertGetRangesToPosition` with no registered providers.
/// `expected` is outside in, as in Monaco.
fn assert_get_ranges_to_position(
    lines: &[&str],
    line_number: u32,
    column: u32,
    expected: &[Range<Position>],
    select_leading_and_trailing_whitespace: bool,
) {
    let model = text::Rope::from(lines.join("\n").as_str());
    let position = Position::new(line_number - 1, column - 1);
    let mut raw = bracket_ranges(&model, position);
    raw.extend(word_ranges(&model, position, true));
    let actual = merge_selection_ranges(
        &model,
        raw,
        position,
        select_leading_and_trailing_whitespace,
    );
    let desired: Vec<_> = expected.iter().rev().cloned().collect();
    assert_eq!(actual, desired);
}

/// The text with the first `|` removed, and the position of that `|`.
fn split_cursor(value: &str) -> (text::Rope, Position) {
    let index = value.find('|').expect("value has a cursor");
    let before = &value[..index];
    let row = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = before[line_start..].encode_utf16().count() as u32;
    let text = value.replacen('|', "", 1);
    (text::Rope::from(text.as_str()), Position::new(row, column))
}

/// Monaco `assertRanges` with `BracketSelectionRangeProvider`.
fn assert_bracket_ranges(value: &str, expected: &[Range<Position>]) {
    let (model, position) = split_cursor(value);
    assert_eq!(bracket_ranges(&model, position), expected, "{value}");
}

/// Monaco `assertRanges` with `WordSelectionRangeProvider(select_subwords)`.
fn assert_word_ranges(value: &str, select_subwords: bool, expected: &[Range<Position>]) {
    let (model, position) = split_cursor(value);
    assert_eq!(
        word_ranges(&model, position, select_subwords),
        expected,
        "{value}"
    );
}

#[test]
fn get_ranges_to_position_1() {
    assert_get_ranges_to_position(
        &[
            "function a(bar, foo){",
            "\tif (bar) {",
            "\t\treturn (bar + (2 * foo))",
            "\t}",
            "}",
        ],
        3,
        20,
        &[
            r(1, 1, 5, 2),  // all
            r(1, 21, 5, 2), // {} outside
            r(1, 22, 5, 1), // {} inside
            r(2, 1, 4, 3),  // block
            r(2, 1, 4, 3),
            r(2, 2, 4, 3),
            r(2, 11, 4, 3),
            r(2, 12, 4, 2),
            r(3, 1, 3, 27),  // line w/ triva
            r(3, 3, 3, 27),  // line w/o triva
            r(3, 10, 3, 27), // () outside
            r(3, 11, 3, 26), // () inside
            r(3, 17, 3, 26), // () outside
            r(3, 18, 3, 25), // () inside
        ],
        true,
    );
}

#[test]
fn config_select_leading_and_trailing_whitespace() {
    assert_get_ranges_to_position(
        &["aaa", "\tbbb", ""],
        2,
        3,
        &[
            r(1, 1, 3, 1), // all
            r(2, 1, 2, 5), // line w/ triva
            r(2, 2, 2, 5), // bbb
        ],
        true,
    );

    assert_get_ranges_to_position(
        &["aaa", "\tbbb", ""],
        2,
        3,
        &[
            r(1, 1, 3, 1), // all
            r(2, 2, 2, 5), // () inside
        ],
        false,
    );
}

#[test]
fn get_ranges_to_position_56886_skip_empty_lines_correctly() {
    assert_get_ranges_to_position(
        &["function a(bar, foo){", "\tif (bar) {", "", "\t}", "}"],
        3,
        1,
        &[
            r(1, 1, 5, 2),
            r(1, 21, 5, 2),
            r(1, 22, 5, 1),
            r(2, 1, 4, 3),
            r(2, 1, 4, 3),
            r(2, 2, 4, 3),
            r(2, 11, 4, 3),
            r(2, 12, 4, 2),
        ],
        true,
    );
}

#[test]
fn get_ranges_to_position_56886_do_not_skip_lines_with_only_whitespaces() {
    assert_get_ranges_to_position(
        &["function a(bar, foo){", "\tif (bar) {", " ", "\t}", "}"],
        3,
        1,
        &[
            r(1, 1, 5, 2),  // all
            r(1, 21, 5, 2), // {} outside
            r(1, 22, 5, 1), // {} inside
            r(2, 1, 4, 3),
            r(2, 1, 4, 3),
            r(2, 2, 4, 3),
            r(2, 11, 4, 3),
            r(2, 12, 4, 2),
            r(3, 1, 3, 2), // block
            r(3, 1, 3, 2), // empty line
        ],
        true,
    );
}

#[test]
fn get_ranges_to_position_40658_cursor_at_first_position_inside_brackets_should_select_line_inside()
{
    assert_get_ranges_to_position(
        &[" [ ]", " { } ", "( ) "],
        2,
        3,
        &[
            r(1, 1, 3, 5),
            r(2, 1, 2, 6), // line w/ triava
            r(2, 2, 2, 5), // {} inside, line w/o triva
            r(2, 3, 2, 4), // {} inside
        ],
        true,
    );
}

#[test]
fn get_ranges_to_position_40658_cursor_in_empty_brackets_should_reveal_brackets_first() {
    assert_get_ranges_to_position(
        &[" [] ", " { } ", "  ( ) "],
        1,
        3,
        &[
            r(1, 1, 3, 7), // all
            r(1, 1, 1, 5), // line w/ trival
            r(1, 2, 1, 4), // [] outside, line w/o trival
            r(1, 3, 1, 3), // [] inside
        ],
        true,
    );
}

#[test]
fn get_ranges_to_position_40658_tokens_before_bracket_will_be_revealed_first() {
    assert_get_ranges_to_position(
        &["  [] ", " { } ", "selectthis( ) "],
        3,
        11,
        &[
            r(1, 1, 3, 15), // all
            r(3, 1, 3, 15), // line w/ trivia
            r(3, 1, 3, 14), // line w/o trivia
            r(3, 1, 3, 11), // word
        ],
        true,
    );
}

// -- bracket selections

#[test]
fn bracket_selection() {
    assert_bracket_ranges("(|)", &[r(1, 2, 1, 2), r(1, 1, 1, 3)]);

    assert_bracket_ranges(
        "[[[](|)]]",
        &[
            r(1, 6, 1, 6),
            r(1, 5, 1, 7), // ()
            r(1, 3, 1, 7),
            r(1, 2, 1, 8), // [[]()]
            r(1, 2, 1, 8),
            r(1, 1, 1, 9), // [[[]()]]
        ],
    );

    assert_bracket_ranges(
        "[a[](|)a]",
        &[r(1, 6, 1, 6), r(1, 5, 1, 7), r(1, 2, 1, 8), r(1, 1, 1, 9)],
    );

    // no bracket
    assert_bracket_ranges("fofof|fofo", &[]);

    // empty
    assert_bracket_ranges("[[[]()]]|", &[]);
    assert_bracket_ranges("|[[[]()]]", &[]);

    // edge
    assert_bracket_ranges("[|[[]()]]", &[r(1, 2, 1, 8), r(1, 1, 1, 9)]);
    assert_bracket_ranges("[[[]()]|]", &[r(1, 2, 1, 8), r(1, 1, 1, 9)]);

    assert_bracket_ranges(
        "aaa(aaa)bbb(b|b)ccc(ccc)",
        &[r(1, 13, 1, 15), r(1, 12, 1, 16)],
    );
    assert_bracket_ranges(
        "(aaa(aaa)bbb(b|b)ccc(ccc))",
        &[
            r(1, 14, 1, 16),
            r(1, 13, 1, 17),
            r(1, 2, 1, 25),
            r(1, 1, 1, 26),
        ],
    );
}

#[test]
fn bracket_with_leading_trailing() {
    assert_bracket_ranges(
        "for(a of b){\n  foo(|);\n}",
        &[
            r(2, 7, 2, 7),
            r(2, 6, 2, 8),
            r(1, 13, 3, 1),
            r(1, 12, 3, 2),
            r(1, 1, 3, 2),
            r(1, 1, 3, 2),
        ],
    );

    assert_bracket_ranges(
        "for(a of b)\n{\n  foo(|);\n}",
        &[
            r(3, 7, 3, 7),
            r(3, 6, 3, 8),
            r(2, 2, 4, 1),
            r(2, 1, 4, 2),
            r(1, 1, 4, 2),
            r(1, 1, 4, 2),
        ],
    );
}

#[test]
fn in_word_ranges() {
    assert_word_ranges(
        "f|ooBar",
        true,
        &[
            r(1, 1, 1, 4), // foo
            r(1, 1, 1, 7), // fooBar
            r(1, 1, 1, 7), // doc
        ],
    );

    assert_word_ranges(
        "f|oo_Ba",
        true,
        &[r(1, 1, 1, 4), r(1, 1, 1, 7), r(1, 1, 1, 7)],
    );

    assert_word_ranges(
        "f|oo-Ba",
        true,
        &[r(1, 1, 1, 4), r(1, 1, 1, 7), r(1, 1, 1, 7)],
    );
}

#[test]
fn in_word_ranges_with_select_subwords_false() {
    assert_word_ranges("f|ooBar", false, &[r(1, 1, 1, 7), r(1, 1, 1, 7)]);
    assert_word_ranges("f|oo_Ba", false, &[r(1, 1, 1, 7), r(1, 1, 1, 7)]);
    assert_word_ranges("f|oo-Ba", false, &[r(1, 1, 1, 7), r(1, 1, 1, 7)]);
}

/// Monaco runs this through `WordSelectionRangeProvider` directly; it does
/// not need the editor's expand/shrink state.
#[test]
fn default_selection_should_select_current_word_hump_first_in_camel_case_67493() {
    let cases = [
        ("Abs|tractSmartSelect", r(1, 1, 1, 9)),
        ("AbstractSma|rtSelect", r(1, 9, 1, 14)),
        ("Abstrac-Sma|rt-elect", r(1, 9, 1, 14)),
        ("Abstrac_Sma|rt_elect", r(1, 9, 1, 14)),
        ("Abstrac_Sma|rt-elect", r(1, 9, 1, 14)),
        ("Abstrac_Sma|rtSelect", r(1, 9, 1, 14)),
    ];
    for (value, part) in cases {
        assert_word_ranges(value, true, &[part, r(1, 1, 1, 20), r(1, 1, 1, 20)]);
    }
}

/// Monaco registers a provider, so `provideSelectionRanges` runs it and the
/// word provider, without the bracket provider.
#[test]
fn only_add_line_ranges_if_theyre_contained_by_the_next_range_73850() {
    let model = text::Rope::from(["type T = {", "\tx: number", "}"].join("\n").as_str());
    let position = Position::new(0, 9);
    let mut raw = vec![r(1, 10, 1, 11), r(1, 10, 3, 2), r(1, 1, 3, 2)];
    raw.extend(word_ranges(&model, position, true));
    let actual = merge_selection_ranges(&model, raw, position, true);
    let expected = [
        r(1, 1, 3, 2),   // all
        r(1, 10, 3, 2),  // { ... }
        r(1, 10, 1, 11), // {
    ];
    let desired: Vec<_> = expected.iter().rev().cloned().collect();
    assert_eq!(actual, desired);
}

#[test]
fn expand_selection_in_words_with_underscores_is_inconsistent_90589() {
    let cases = [
        ("Hel|lo_World", r(1, 1, 1, 6), 12),
        ("Hello_Wo|rld", r(1, 7, 1, 12), 12),
        ("Hello|_World", r(1, 1, 1, 6), 12),
        ("Hello_|World", r(1, 7, 1, 12), 12),
        ("Hello|-World", r(1, 1, 1, 6), 12),
        ("Hello-|World", r(1, 7, 1, 12), 12),
        ("Hello|World", r(1, 6, 1, 11), 11),
    ];
    for (value, part, end) in cases {
        assert_word_ranges(value, true, &[part, r(1, 1, 1, end), r(1, 1, 1, end)]);
    }
}
