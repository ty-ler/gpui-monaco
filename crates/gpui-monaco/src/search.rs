//! Text search, ported from Monaco's `common/model/textModelSearch.ts` (MIT).
//!
//! Regular expressions use the Rust `regex` crate rather than JavaScript's
//! engine: matching is linear-time, but lookaround and backreferences are not
//! supported (such patterns are reported as invalid).

use std::ops::Range;

use regex::{Regex, RegexBuilder};
use text::Rope;

/// What to search for (Monaco `SearchParams`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchParams {
    /// The text or regular expression to find.
    pub query: String,
    /// Treat `query` as a regular expression rather than literal text.
    pub is_regex: bool,
    /// Match case exactly; otherwise case-insensitive.
    pub match_case: bool,
    /// Word separators for whole-word matching; `None` matches anywhere.
    pub word_separators: Option<String>,
}

/// A compiled search (Monaco `SearchData`).
#[derive(Clone, Debug)]
pub struct SearchData {
    regex: Regex,
    word_separators: Option<WordSeparators>,
    multiline: bool,
}

#[derive(Clone, Debug)]
struct WordSeparators(String);

impl WordSeparators {
    /// Monaco `WordCharacterClass` != Regular.
    fn is_boundary(&self, ch: char) -> bool {
        ch == ' ' || ch == '\t' || ch == '\r' || ch == '\n' || self.0.contains(ch)
    }
}

/// Monaco `isMultilineRegexSource`: whether a regex can match across lines.
pub fn is_multiline_regex_source(source: &str) -> bool {
    let mut chars = source.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\n' => return true,
            '\\' => match chars.next() {
                Some('n' | 'r' | 'W') => return true,
                Some(_) => {}
                None => break,
            },
            _ => {}
        }
    }
    false
}

impl SearchParams {
    /// Search parameters from their parts.
    pub fn new(
        query: impl Into<String>,
        is_regex: bool,
        match_case: bool,
        word_separators: Option<&str>,
    ) -> Self {
        Self {
            query: query.into(),
            is_regex,
            match_case,
            word_separators: word_separators.map(Into::into),
        }
    }

    /// Compiles the search, or `None` for an empty query or invalid regex
    /// (Monaco `parseSearchRequest`).
    pub fn parse(&self) -> Option<SearchData> {
        if self.query.is_empty() {
            return None;
        }
        let multiline = if self.is_regex {
            is_multiline_regex_source(&self.query)
        } else {
            self.query.contains('\n')
        };
        let source = if self.is_regex {
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        let regex = RegexBuilder::new(&source)
            .case_insensitive(!self.match_case)
            .multi_line(true)
            .build()
            .ok()?;
        Some(SearchData {
            regex,
            word_separators: self.word_separators.clone().map(WordSeparators),
            multiline,
        })
    }
}

impl SearchData {
    /// True if matches can span lines (the regex can match a line break, or
    /// the literal query contains one), so the search runs over the whole
    /// text instead of line by line.
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// Monaco `isValidMatch`: whole-word boundaries on both sides.
    fn is_valid(&self, text: &str, range: &Range<usize>) -> bool {
        let Some(separators) = &self.word_separators else {
            return true;
        };
        let before = text[..range.start].chars().next_back();
        let first = text[range.clone()].chars().next();
        let after = text[range.end..].chars().next();
        let last = text[range.clone()].chars().next_back();
        let left = match before {
            None => true,
            Some(ch) => {
                separators.is_boundary(ch) || first.is_some_and(|c| separators.is_boundary(c))
            }
        };
        let right = match after {
            None => true,
            Some(ch) => {
                separators.is_boundary(ch) || last.is_some_and(|c| separators.is_boundary(c))
            }
        };
        left && right
    }

    /// Valid matches in `text` starting at or after byte `from`.
    fn matches_in<'a>(
        &'a self,
        text: &'a str,
        from: usize,
    ) -> impl Iterator<Item = Range<usize>> + 'a {
        let mut at = from;
        let mut prev_end: Option<usize> = None;
        std::iter::from_fn(move || {
            while at <= text.len() {
                // Like Monaco's `Searcher`, stop once a match reached the end.
                if prev_end == Some(text.len()) {
                    return None;
                }
                let m = self.regex.find_at(text, at)?;
                let range = m.range();
                prev_end = Some(range.end);
                at = if range.is_empty() {
                    // Step over the next char so empty matches advance.
                    range.end + text[range.end..].chars().next().map_or(1, char::len_utf8)
                } else {
                    range.end
                };
                if self.is_valid(text, &range) {
                    return Some(range);
                }
            }
            None
        })
    }
}

/// All matches (byte ranges) within `range` of `rope`, up to `limit`.
pub fn find_matches(
    rope: &Rope,
    data: &SearchData,
    range: Range<usize>,
    limit: usize,
) -> Vec<Range<usize>> {
    let range = range.start.min(rope.len())..range.end.min(rope.len());
    let mut result = Vec::new();
    if data.multiline {
        let text = rope.text_for_range(range.clone());
        for m in data.matches_in(&text, 0) {
            result.push(range.start + m.start..range.start + m.end);
            if result.len() >= limit {
                break;
            }
        }
        return result;
    }
    let first_row = rope.offset_to_point(range.start).row;
    let last_row = rope.offset_to_point(range.end).row;
    let lines = rope.line_range(first_row).start..rope.line_range(last_row).end;
    rope.for_each_line(lines, |line_start, line| {
        let from = range.start.saturating_sub(line_start).min(line.len());
        let until = range.end.saturating_sub(line_start);
        for m in data.matches_in(line, from) {
            if m.end > until || result.len() >= limit {
                break;
            }
            result.push(line_start + m.start..line_start + m.end);
        }
        result.len() < limit
    });
    result
}

/// The first match starting at or after `from`, wrapping to the start
/// (Monaco `findNextMatch`).
pub fn find_next(rope: &Rope, data: &SearchData, from: usize) -> Option<Range<usize>> {
    let from = from.min(rope.len());
    if data.multiline {
        let text = rope.to_string();
        return data
            .matches_in(&text, from)
            .next()
            .or_else(|| data.matches_in(&text, 0).next());
    }
    let start_row = rope.offset_to_point(from).row;
    let line_count = rope.line_count();
    for i in 0..=line_count {
        let row = (start_row + i) % line_count;
        let line_range = rope.line_range(row);
        let line = rope.text_for_range(line_range.clone());
        let from_col = if i == 0 { from - line_range.start } else { 0 };
        if let Some(m) = data.matches_in(&line, from_col).next() {
            if i == line_count && m.start >= from - line_range.start {
                // Wrapped back to the start row: only matches before `from`.
                return None;
            }
            return Some(line_range.start + m.start..line_range.start + m.end);
        }
    }
    None
}

/// The last match ending at or before `before`, wrapping to the end (Monaco
/// `findPreviousMatch`).
pub fn find_previous(rope: &Rope, data: &SearchData, before: usize) -> Option<Range<usize>> {
    let before = before.min(rope.len());
    if data.multiline {
        let text = rope.to_string();
        let all: Vec<Range<usize>> = data.matches_in(&text, 0).collect();
        return all
            .iter()
            .rev()
            .find(|m| m.end <= before)
            .or_else(|| all.last())
            .cloned();
    }
    let start_row = rope.offset_to_point(before).row;
    let line_count = rope.line_count();
    for i in 0..=line_count {
        let row = (start_row + line_count - i % line_count) % line_count;
        let line_range = rope.line_range(row);
        let line = rope.text_for_range(line_range.clone());
        let limit = if i == 0 {
            before - line_range.start
        } else {
            line.len()
        };
        // Monaco searches the line truncated at the start position.
        if let Some(m) = data.matches_in(&line[..limit], 0).last() {
            return Some(line_range.start + m.start..line_range.start + m.end);
        }
    }
    None
}

/// The text a match was found in and where it starts: the match's line, or
/// the whole text for multiline searches (as [`find_matches`] searches).
/// Pass it to [`captures`] so they see the same context.
pub fn match_context(rope: &Rope, data: &SearchData, matched: &Range<usize>) -> (String, usize) {
    if data.multiline {
        (rope.text_for_range(0..rope.len()), 0)
    } else {
        let row = rope.offset_to_point(matched.start).row;
        let line = rope.line_range(row);
        (rope.text_for_range(line.clone()), line.start)
    }
}

/// The captures of the match at `matched` (byte range) in `haystack`, the
/// text it was found in (see [`match_context`]), whole match first (Monaco's
/// `captureMatches`). Re-matching just the matched text could see different
/// captures when the pattern depends on its surroundings (`\b`, `^`, `$`).
pub fn captures(
    data: &SearchData,
    haystack: &str,
    matched: Range<usize>,
) -> Option<Vec<Option<String>>> {
    let captures = data
        .regex
        .captures_at(haystack, matched.start)
        .filter(|c| c.get(0).is_some_and(|m| m.range() == matched))?;
    Some(
        captures
            .iter()
            .map(|m| m.map(|m| m.as_str().to_string()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    //! Ported from Monaco's `test/common/model/textModelSearch.test.ts`.
    //! Skipped: capture-match tests (we don't return captures), `\r` tests
    //! (text is `\n`-normalized), and `\W`-crosses-newlines (Rust `\W` with
    //! line-by-line search matches Monaco only for multiline sources, which
    //! we treat the same).
    use super::*;
    use text::PointUtf16;

    const USUAL_WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

    fn regular_text() -> String {
        [
            "This is some foo - bar text which contains foo and bar - as in Barcelona.",
            "Now it begins a word fooBar and now it is caps Foo-isn't this great?",
            "And here's a dull line with nothing interesting in it",
            "It is also interesting if it's part of a word like amazingFooBar",
            "Again nothing interesting here",
        ]
        .join("\n")
    }

    fn to_monaco(rope: &Rope, range: &Range<usize>) -> (u32, u32, u32, u32) {
        let (a, b) = (
            rope.offset_to_point_utf16(range.start),
            rope.offset_to_point_utf16(range.end),
        );
        (a.row + 1, a.column + 1, b.row + 1, b.column + 1)
    }

    fn from_monaco(rope: &Rope, line: u32, column: u32) -> usize {
        rope.point_utf16_to_offset(PointUtf16::from_monaco(line, column))
    }

    fn assert_find_matches(
        text: &str,
        query: &str,
        is_regex: bool,
        match_case: bool,
        separators: Option<&str>,
        expected: &[(u32, u32, u32, u32)],
    ) {
        let rope = Rope::from(text);
        let data = SearchParams::new(query, is_regex, match_case, separators)
            .parse()
            .unwrap();
        let actual: Vec<_> = find_matches(&rope, &data, 0..rope.len(), 1000)
            .iter()
            .map(|m| to_monaco(&rope, m))
            .collect();
        assert_eq!(actual, expected, "findMatches {query:?}");

        // findNextMatch from the start and from each match's start.
        let next = find_next(&rope, &data, 0).map(|m| to_monaco(&rope, &m));
        assert_eq!(next, expected.first().copied(), "findNextMatch from start");
        for e in expected {
            let from = from_monaco(&rope, e.0, e.1);
            let next = find_next(&rope, &data, from).map(|m| to_monaco(&rope, &m));
            assert_eq!(next, Some(*e), "findNextMatch from {e:?}");
        }
        // findPreviousMatch from the end and from each match's end.
        let prev = find_previous(&rope, &data, rope.len()).map(|m| to_monaco(&rope, &m));
        assert_eq!(prev, expected.last().copied(), "findPrevMatch from end");
        for e in expected {
            let before = from_monaco(&rope, e.2, e.3);
            let prev = find_previous(&rope, &data, before).map(|m| to_monaco(&rope, &m));
            assert_eq!(prev, Some(*e), "findPrevMatch from {e:?}");
        }
    }

    #[test]
    fn simple_case_sensitive_and_whole_words() {
        let text = regular_text();
        assert_find_matches(
            &text,
            "foo",
            false,
            false,
            None,
            &[
                (1, 14, 1, 17),
                (1, 44, 1, 47),
                (2, 22, 2, 25),
                (2, 48, 2, 51),
                (4, 59, 4, 62),
            ],
        );
        assert_find_matches(
            &text,
            "foo",
            false,
            true,
            None,
            &[(1, 14, 1, 17), (1, 44, 1, 47), (2, 22, 2, 25)],
        );
        assert_find_matches(
            &text,
            "foo",
            false,
            false,
            Some(USUAL_WORD_SEPARATORS),
            &[(1, 14, 1, 17), (1, 44, 1, 47), (2, 48, 2, 51)],
        );
    }

    #[test]
    fn anchors() {
        let text = regular_text();
        assert_find_matches(
            &text,
            "^",
            true,
            false,
            None,
            &[
                (1, 1, 1, 1),
                (2, 1, 2, 1),
                (3, 1, 3, 1),
                (4, 1, 4, 1),
                (5, 1, 5, 1),
            ],
        );
        assert_find_matches(
            &text,
            "$",
            true,
            false,
            None,
            &[
                (1, 74, 1, 74),
                (2, 69, 2, 69),
                (3, 54, 3, 54),
                (4, 65, 4, 65),
                (5, 31, 5, 31),
            ],
        );
        assert_find_matches(
            &text,
            ".*",
            true,
            false,
            None,
            &[
                (1, 1, 1, 74),
                (2, 1, 2, 69),
                (3, 1, 3, 54),
                (4, 1, 4, 65),
                (5, 1, 5, 31),
            ],
        );
        let text = [
            "This is some foo - bar text which contains foo and bar - as in Barcelona.",
            "",
            "And here's a dull line with nothing interesting in it",
            "",
            "Again nothing interesting here",
        ]
        .join("\n");
        assert_find_matches(
            &text,
            "^$",
            true,
            false,
            None,
            &[(2, 1, 2, 1), (4, 1, 4, 1)],
        );
    }

    #[test]
    fn multiline_finds() {
        let text = [
            "Just some text text",
            "Just some text text",
            "some text again",
            "again some text",
        ]
        .join("\n");
        assert_find_matches(
            &text,
            "text\\n",
            true,
            false,
            None,
            &[(1, 16, 2, 1), (2, 16, 3, 1)],
        );
        assert_find_matches(&text, "text\\nJust", true, false, None, &[(1, 16, 2, 5)]);
        assert_find_matches(&text, "\\nagain", true, false, None, &[(3, 16, 4, 6)]);
        assert_find_matches(&text, ".*\\nJust.*\\n", true, false, None, &[(1, 1, 3, 1)]);
        let text = ["if", "else", "", "if", "else"].join("\n");
        assert_find_matches(
            &text,
            "^if\\nelse",
            true,
            false,
            None,
            &[(1, 1, 2, 5), (4, 1, 5, 5)],
        );
        let text = ["if", "", "else", "  ", "if", " ", "else"].join("\n");
        assert_find_matches(
            &text,
            "^\\s*$\\n",
            true,
            false,
            None,
            &[(2, 1, 3, 1), (4, 1, 5, 1), (6, 1, 7, 1)],
        );
        let text = ["a if b", "a", "ab", "eb"].join("\n");
        assert_find_matches(
            &text,
            "^a.*b$",
            true,
            false,
            None,
            &[(1, 1, 1, 7), (3, 1, 3, 3)],
        );
        let text = ["if", "else", "", "if", "elseif", "else"].join("\n");
        assert_find_matches(
            &text,
            "if\\nelse$",
            true,
            false,
            None,
            &[(1, 1, 2, 5), (5, 5, 6, 5)],
        );
        let text = [
            "Just some text text",
            "",
            "some text again",
            "",
            "again some text",
        ]
        .join("\n");
        assert_find_matches(
            &text,
            "^.*$",
            true,
            false,
            None,
            &[
                (1, 1, 1, 20),
                (2, 1, 2, 1),
                (3, 1, 3, 16),
                (4, 1, 4, 1),
                (5, 1, 5, 16),
            ],
        );
        let text = [
            "Just some text text",
            "some text text",
            "some text again",
            "again some text",
            "but not some",
        ]
        .join("\n");
        assert_find_matches(
            &text,
            "text\nsome",
            false,
            false,
            None,
            &[(1, 16, 2, 5), (2, 11, 3, 5)],
        );
    }

    #[test]
    fn whole_word_non_latin_and_regressions() {
        let text = ["this is a bla text", "bla text", "bla"].join("\n");
        assert_find_matches(
            &text,
            "bla",
            false,
            false,
            Some(USUAL_WORD_SEPARATORS),
            &[(1, 11, 1, 14), (2, 1, 2, 4), (3, 1, 3, 4)],
        );
        let text = ["我想吃饭", "看雨"].join("\n");
        assert_find_matches(
            &text,
            "吃饭",
            false,
            false,
            Some(USUAL_WORD_SEPARATORS),
            &[],
        );
        let text = ["this is a foo bar", "foo bar foo"].join("\n");
        assert_find_matches(
            &text,
            "foo bar",
            false,
            false,
            Some(USUAL_WORD_SEPARATORS),
            &[(1, 11, 1, 18), (2, 1, 2, 8)],
        );
    }

    #[test]
    fn is_multiline_regex_source_cases() {
        assert!(!is_multiline_regex_source("foo"));
        assert!(!is_multiline_regex_source(""));
        assert!(!is_multiline_regex_source("foo\\sbar"));
        assert!(!is_multiline_regex_source("\\\\notnewline"));
        assert!(is_multiline_regex_source("foo\\nbar"));
        assert!(is_multiline_regex_source("foo\\nbar\\s"));
        assert!(is_multiline_regex_source("foo\\r\\n"));
        assert!(is_multiline_regex_source("\\n"));
        assert!(is_multiline_regex_source("foo\\W"));
        assert!(is_multiline_regex_source("foo\n"));
        assert!(is_multiline_regex_source("foo\r\n"));
    }

    #[test]
    fn empty_matches_advance() {
        // issue #74715: \d* finds empty strings and must keep searching.
        let rope = Rope::from("10.243.30.10");
        let data = SearchParams::new("\\d*", true, false, None)
            .parse()
            .unwrap();
        let actual: Vec<_> = find_matches(&rope, &data, 0..rope.len(), 100)
            .iter()
            .map(|m| to_monaco(&rope, m))
            .collect();
        assert_eq!(
            actual,
            [
                (1, 1, 1, 3),
                (1, 3, 1, 3),
                (1, 4, 1, 7),
                (1, 7, 1, 7),
                (1, 8, 1, 10),
                (1, 10, 1, 10),
                (1, 11, 1, 13)
            ]
        );
        // issue #100134: zero-length matches step over surrogate pairs.
        assert_find_matches(
            "1\u{1F4BB}1",
            "()",
            true,
            false,
            None,
            &[(1, 1, 1, 1), (1, 2, 1, 2), (1, 4, 1, 4), (1, 5, 1, 5)],
        );
        assert_find_matches(
            "1\u{1F431}\u{200D}\u{1F4BB}1",
            "()",
            true,
            false,
            None,
            &[
                (1, 1, 1, 1),
                (1, 2, 1, 2),
                (1, 4, 1, 4),
                (1, 5, 1, 5),
                (1, 7, 1, 7),
                (1, 8, 1, 8),
            ],
        );
    }

    #[test]
    fn parse_search_request() {
        assert!(SearchParams::new("", false, false, None).parse().is_none());
        assert!(SearchParams::new("(", true, false, None).parse().is_none());
        assert!(SearchParams::new("(", false, false, None).parse().is_some());
    }

    #[test]
    fn replacement_captures() {
        let data = SearchParams::new("(\\w+)@(\\w+)", true, false, None)
            .parse()
            .unwrap();
        assert_eq!(
            captures(&data, "user@host", 0..9),
            Some(vec![
                Some("user@host".to_string()),
                Some("user".to_string()),
                Some("host".to_string())
            ])
        );
        // Captures come from the match in context: `^` only matches at the
        // line start, not at the start of the matched text.
        let data = SearchParams::new("(^a|b)(a)", true, true, None)
            .parse()
            .unwrap();
        assert_eq!(
            captures(&data, "xba", 1..3).unwrap()[1].as_deref(),
            Some("b")
        );
    }
}
