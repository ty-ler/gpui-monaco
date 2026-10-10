//! Find/replace replacement strings: `$1`, `$&`, `\n`, `\U` and friends.
//!
//! Ports `replacePattern.ts` and `buildReplaceStringWithCasePreserved` from
//! `base/common/search.ts` (MIT).
//!
//! `matches` arguments are the captures of one regex match, as JavaScript's
//! `RegExp.exec` returns them: index 0 is the whole match, then one entry per
//! capture group, with `None` for a group that did not participate. With the
//! `regex` crate that is
//! `caps.iter().map(|m| m.map(|m| m.as_str())).collect::<Vec<_>>()`.
//!
//! Monaco walks strings in UTF-16 code units; this port walks `char`s. The
//! parser only reacts to ASCII, so parsing is identical, but the case
//! operators (`\u`, `\l`, ...) and case preservation act on whole scalar
//! values, so astral-plane letters are case-mapped where Monaco leaves their
//! surrogate halves untouched.

/// One case operator from a replace string (Monaco's `caseOps` entries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseOp {
    /// `\u`: upper-cases the next character.
    UpperNext,
    /// `\U`: upper-cases all remaining characters.
    UpperAll,
    /// `\l`: lower-cases the next character.
    LowerNext,
    /// `\L`: lower-cases all remaining characters.
    LowerAll,
}

/// A piece of a parsed replace string: literal text or a reference to a
/// capture (Monaco `ReplacePiece`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplacePiece {
    /// Literal text.
    Static(String),
    /// Capture `index` (0 = the whole match), with case operators applied.
    Match {
        /// The capture index, `0..=99`.
        index: usize,
        /// Case operators to apply to the captured text, in order.
        case_ops: Vec<CaseOp>,
    },
}

impl ReplacePiece {
    /// Literal text (Monaco `ReplacePiece.staticValue`).
    pub fn static_value(value: impl Into<String>) -> Self {
        Self::Static(value.into())
    }

    /// A capture reference (Monaco `ReplacePiece.matchIndex`).
    pub fn match_index(index: usize) -> Self {
        Self::case_ops(index, Vec::new())
    }

    /// A capture reference with case operators (Monaco `ReplacePiece.caseOps`).
    pub fn case_ops(index: usize, case_ops: Vec<CaseOp>) -> Self {
        Self::Match { index, case_ops }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Static(String),
    Dynamic(Vec<ReplacePiece>),
}

/// A parsed replace string (Monaco `ReplacePattern`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacePattern {
    state: State,
}

impl Default for ReplacePattern {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl ReplacePattern {
    /// A pattern that always produces `value` (Monaco
    /// `ReplacePattern.fromStaticValue`).
    pub fn from_static_value(value: impl Into<String>) -> Self {
        Self::new(vec![ReplacePiece::static_value(value)])
    }

    /// Builds a pattern from pieces; a lone literal piece (or none) becomes a
    /// static pattern (Monaco's `ReplacePattern` constructor).
    pub fn new(mut pieces: Vec<ReplacePiece>) -> Self {
        let state = match pieces.as_mut_slice() {
            [] => State::Static(String::new()),
            [ReplacePiece::Static(value)] => State::Static(std::mem::take(value)),
            _ => State::Dynamic(pieces),
        };
        Self { state }
    }

    /// Whether the pattern references captures (Monaco
    /// `hasReplacementPatterns`).
    pub fn has_replacement_patterns(&self) -> bool {
        matches!(self.state, State::Dynamic(_))
    }

    /// Produces the replacement text for one match (Monaco
    /// `buildReplaceString`). `matches` is described in the module docs;
    /// `preserve_case` only applies to static patterns, as in Monaco.
    pub fn build_replace_string(
        &self,
        matches: Option<&[Option<&str>]>,
        preserve_case: bool,
    ) -> String {
        let pieces = match &self.state {
            State::Static(value) if preserve_case => {
                return build_replace_string_with_case_preserved(matches, value);
            }
            State::Static(value) => return value.clone(),
            State::Dynamic(pieces) => pieces,
        };

        let mut result = String::new();
        for piece in pieces {
            let (index, case_ops) = match piece {
                ReplacePiece::Static(value) => {
                    result.push_str(value);
                    continue;
                }
                ReplacePiece::Match { index, case_ops } => (*index, case_ops),
            };
            let text = substitute(index, matches);
            if case_ops.is_empty() {
                result.push_str(&text);
                continue;
            }
            // Each "next" op consumes one character; an "all" op stays in
            // effect. Once the ops run out the rest is copied unchanged.
            let mut ops = case_ops.iter();
            let mut op = ops.next();
            for (offset, ch) in text.char_indices() {
                match op {
                    None => {
                        result.push_str(&text[offset..]);
                        break;
                    }
                    Some(CaseOp::UpperAll) => result.extend(ch.to_uppercase()),
                    Some(CaseOp::LowerAll) => result.extend(ch.to_lowercase()),
                    Some(CaseOp::UpperNext) => {
                        result.extend(ch.to_uppercase());
                        op = ops.next();
                    }
                    Some(CaseOp::LowerNext) => {
                        result.extend(ch.to_lowercase());
                        op = ops.next();
                    }
                }
            }
        }
        result
    }
}

/// Monaco's `_substitute`: `$nn` names group `nn` only if it exists,
/// otherwise it degrades to `$n` followed by the literal digit (JavaScript
/// `String.prototype.replace` semantics). Missing groups become `""`.
fn substitute(mut index: usize, matches: Option<&[Option<&str>]>) -> String {
    let Some(matches) = matches else {
        return String::new();
    };
    if index == 0 {
        return matches.first().copied().flatten().unwrap_or("").to_string();
    }
    let mut remainder = String::new();
    while index > 0 {
        if index < matches.len() {
            return format!("{}{remainder}", matches[index].unwrap_or(""));
        }
        remainder.insert_str(0, &(index % 10).to_string());
        index /= 10;
    }
    format!("${remainder}")
}

/// Accumulates pieces while parsing (Monaco `ReplacePieceBuilder`). Indices
/// are byte offsets into `source`; they always fall after ASCII bytes.
struct PieceBuilder<'a> {
    source: &'a str,
    last_index: usize,
    result: Vec<ReplacePiece>,
    current_static: String,
}

impl<'a> PieceBuilder<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            last_index: 0,
            result: Vec::new(),
            current_static: String::new(),
        }
    }

    fn emit_unchanged(&mut self, to_index: usize) {
        self.current_static
            .push_str(&self.source[self.last_index..to_index]);
        self.last_index = to_index;
    }

    fn emit_static(&mut self, value: &str, to_index: usize) {
        self.current_static.push_str(value);
        self.last_index = to_index;
    }

    fn flush_static(&mut self) {
        if !self.current_static.is_empty() {
            let value = std::mem::take(&mut self.current_static);
            self.result.push(ReplacePiece::Static(value));
        }
    }

    fn emit_match_index(&mut self, index: usize, to_index: usize, case_ops: &mut Vec<CaseOp>) {
        self.flush_static();
        self.result
            .push(ReplacePiece::case_ops(index, std::mem::take(case_ops)));
        self.last_index = to_index;
    }

    fn finalize(mut self) -> ReplacePattern {
        self.emit_unchanged(self.source.len());
        self.flush_static();
        ReplacePattern::new(self.result)
    }
}

/// Parses a replace string (Monaco `parseReplaceString`):
///
/// - `\n`, `\t`, `\\` insert LF, TAB and `\`;
/// - `\u` / `\l` upper-/lower-case the next character of the following
///   capture, `\U` / `\L` all of its remaining characters;
/// - `$$` inserts `$`; `$&` and `$0` the whole match; `$n` / `$nn` capture
///   `n` / `nn` (see [`ReplacePattern::build_replace_string`]).
///
/// Everything else, including a `\` before any other character, is kept as
/// typed.
pub fn parse_replace_string(replace_string: &str) -> ReplacePattern {
    if replace_string.is_empty() {
        return ReplacePattern::default();
    }

    let bytes = replace_string.as_bytes();
    let len = bytes.len();
    let mut case_ops = Vec::new();
    let mut builder = PieceBuilder::new(replace_string);

    let mut i = 0;
    while i < len {
        match bytes[i] {
            b'\\' => {
                i += 1;
                if i >= len {
                    break;
                }
                let escaped = match bytes[i] {
                    b'\\' => Some("\\"),
                    b'n' => Some("\n"),
                    b't' => Some("\t"),
                    b'u' | b'U' | b'l' | b'L' => {
                        case_ops.push(match bytes[i] {
                            b'u' => CaseOp::UpperNext,
                            b'U' => CaseOp::UpperAll,
                            b'l' => CaseOp::LowerNext,
                            _ => CaseOp::LowerAll,
                        });
                        Some("")
                    }
                    _ => None,
                };
                if let Some(value) = escaped {
                    builder.emit_unchanged(i - 1);
                    builder.emit_static(value, i + 1);
                }
            }
            b'$' => {
                i += 1;
                if i >= len {
                    break;
                }
                match bytes[i] {
                    b'$' => {
                        builder.emit_unchanged(i - 1);
                        builder.emit_static("$", i + 1);
                    }
                    b'0' | b'&' => {
                        builder.emit_unchanged(i - 1);
                        builder.emit_match_index(0, i + 1, &mut case_ops);
                    }
                    digit @ b'1'..=b'9' => {
                        let mut index = usize::from(digit - b'0');
                        if let Some(next @ b'0'..=b'9') = bytes.get(i + 1).copied() {
                            i += 1;
                            index = index * 10 + usize::from(next - b'0');
                            builder.emit_unchanged(i - 2);
                        } else {
                            builder.emit_unchanged(i - 1);
                        }
                        builder.emit_match_index(index, i + 1, &mut case_ops);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        i += 1;
    }

    builder.finalize()
}

/// Adapts `pattern` to the casing of the matched text `matches[0]`: all
/// upper, all lower, or a capitalized / uncapitalized first letter, applied
/// per `-` or `_` separated segment when both sides have the same number of
/// segments (Monaco `buildReplaceStringWithCasePreserved`).
pub fn build_replace_string_with_case_preserved(
    matches: Option<&[Option<&str>]>,
    pattern: &str,
) -> String {
    let matched = matches
        .and_then(|matches| matches.first().copied().flatten())
        .unwrap_or("");
    let Some(first) = matched.chars().next() else {
        return pattern.to_string();
    };

    let hyphens = has_matching_separators(matched, pattern, '-');
    let underscores = has_matching_separators(matched, pattern, '_');
    if hyphens && !underscores {
        return build_per_segment(matched, pattern, '-');
    } else if !hyphens && underscores {
        return build_per_segment(matched, pattern, '_');
    }

    if matched.to_uppercase() == matched {
        pattern.to_uppercase()
    } else if matched.to_lowercase() == matched {
        pattern.to_lowercase()
    } else if changes_case(first, str::to_lowercase) {
        // `containsUppercaseCharacter(first)`: lower-casing changes it.
        map_first_char(pattern, |c| c.to_uppercase().collect())
    } else if changes_case(first, str::to_uppercase) {
        // Upper-casing changes it.
        map_first_char(pattern, |c| c.to_lowercase().collect())
    } else {
        // We don't understand its pattern yet.
        pattern.to_string()
    }
}

/// Whether `f` changes `ch`, as in Monaco's `x.toLowerCase() !== x` checks.
fn changes_case(ch: char, f: fn(&str) -> String) -> bool {
    let mut buf = [0; 4];
    let s: &str = ch.encode_utf8(&mut buf);
    f(s) != s
}

fn map_first_char(pattern: &str, f: impl Fn(char) -> String) -> String {
    let mut chars = pattern.chars();
    match chars.next() {
        Some(first) => f(first) + chars.as_str(),
        None => String::new(),
    }
}

/// Monaco `validateSpecificSpecialCharacter`.
fn has_matching_separators(matched: &str, pattern: &str, separator: char) -> bool {
    matched.contains(separator)
        && pattern.contains(separator)
        && matched.split(separator).count() == pattern.split(separator).count()
}

/// Monaco `buildReplaceStringForSpecificSpecialCharacter`.
fn build_per_segment(matched: &str, pattern: &str, separator: char) -> String {
    pattern
        .split(separator)
        .zip(matched.split(separator))
        .map(|(pattern, matched)| {
            build_replace_string_with_case_preserved(Some(&[Some(matched)]), pattern)
        })
        .collect::<Vec<_>>()
        .join(&separator.to_string())
}

#[cfg(test)]
mod tests {
    //! Ports `editor/contrib/find/test/browser/replacePattern.test.ts`.

    use super::*;
    use regex::Regex;

    use CaseOp::{LowerAll, LowerNext, UpperAll, UpperNext};

    fn test_parse(input: &str, expected: Vec<ReplacePiece>) {
        assert_eq!(
            parse_replace_string(input),
            ReplacePattern::new(expected),
            "Parsing {input:?}"
        );
    }

    fn st(value: &str) -> ReplacePiece {
        ReplacePiece::static_value(value)
    }

    fn mi(index: usize) -> ReplacePiece {
        ReplacePiece::match_index(index)
    }

    /// `parseReplaceString(replace).buildReplaceString(search.exec(target))`.
    fn replace(target: &str, search: &str, replace_string: &str) -> String {
        let captures = Regex::new(search).unwrap().captures(target);
        let matches: Option<Vec<Option<&str>>> =
            captures.map(|caps| caps.iter().map(|m| m.map(|m| m.as_str())).collect());
        parse_replace_string(replace_string).build_replace_string(matches.as_deref(), false)
    }

    fn assert_replace(target: &str, search: &str, replace_string: &str, expected: &str) {
        assert_eq!(
            replace(target, search, replace_string),
            expected,
            "{target:?}.replace(/{search}/, {replace_string:?})"
        );
    }

    #[test]
    fn parse_replace_string_basics() {
        // No backslash => no treatment.
        test_parse("hello", vec![st("hello")]);

        // \t => TAB
        test_parse(r"\thello", vec![st("\thello")]);
        test_parse(r"h\tello", vec![st("h\tello")]);
        test_parse(r"hello\t", vec![st("hello\t")]);

        // \n => LF
        test_parse(r"\nhello", vec![st("\nhello")]);

        // \\t => \t
        test_parse(r"\\thello", vec![st(r"\thello")]);
        test_parse(r"h\\tello", vec![st(r"h\tello")]);
        test_parse(r"hello\\t", vec![st(r"hello\t")]);

        // \\\t => \TAB
        test_parse(r"\\\thello", vec![st("\\\thello")]);

        // \\\\t => \\t
        test_parse(r"\\\\thello", vec![st(r"\\thello")]);

        // \ at the end => no treatment
        test_parse(r"hello\", vec![st(r"hello\")]);

        // \ with unknown char => no treatment
        test_parse(r"hello\x", vec![st(r"hello\x")]);

        // \ with back reference => no treatment
        test_parse(r"hello\0", vec![st(r"hello\0")]);

        test_parse("hello$&", vec![st("hello"), mi(0)]);
        test_parse("hello$0", vec![st("hello"), mi(0)]);
        test_parse("hello$02", vec![st("hello"), mi(0), st("2")]);
        test_parse("hello$1", vec![st("hello"), mi(1)]);
        test_parse("hello$2", vec![st("hello"), mi(2)]);
        test_parse("hello$9", vec![st("hello"), mi(9)]);
        test_parse("$9hello", vec![mi(9), st("hello")]);

        test_parse("hello$12", vec![st("hello"), mi(12)]);
        test_parse("hello$99", vec![st("hello"), mi(99)]);
        test_parse("hello$99a", vec![st("hello"), mi(99), st("a")]);
        test_parse("hello$1a", vec![st("hello"), mi(1), st("a")]);
        test_parse("hello$100", vec![st("hello"), mi(10), st("0")]);
        test_parse("hello$100a", vec![st("hello"), mi(10), st("0a")]);
        test_parse("hello$10a0", vec![st("hello"), mi(10), st("a0")]);
        test_parse("hello$$", vec![st("hello$")]);
        test_parse("hello$$0", vec![st("hello$0")]);

        test_parse("hello$`", vec![st("hello$`")]);
        test_parse("hello$'", vec![st("hello$'")]);
    }

    #[test]
    fn parse_replace_string_with_case_modifiers() {
        let co = ReplacePiece::case_ops;

        test_parse(r"hello\U$1", vec![st("hello"), co(1, vec![UpperAll])]);
        assert_replace(
            "func privateFunc(",
            r"func (\w+)\(",
            r"func \U$1(",
            "func PRIVATEFUNC(",
        );

        test_parse(r"hello\u$1", vec![st("hello"), co(1, vec![UpperNext])]);
        assert_replace(
            "func privateFunc(",
            r"func (\w+)\(",
            r"func \u$1(",
            "func PrivateFunc(",
        );

        test_parse(r"hello\L$1", vec![st("hello"), co(1, vec![LowerAll])]);
        assert_replace(
            "func privateFunc(",
            r"func (\w+)\(",
            r"func \L$1(",
            "func privatefunc(",
        );

        test_parse(r"hello\l$1", vec![st("hello"), co(1, vec![LowerNext])]);
        assert_replace(
            "func PrivateFunc(",
            r"func (\w+)\(",
            r"func \l$1(",
            "func privateFunc(",
        );

        test_parse(
            r"hello$1\u\u\U$4goodbye",
            vec![
                st("hello"),
                mi(1),
                co(4, vec![UpperNext, UpperNext, UpperAll]),
                st("goodbye"),
            ],
        );
        assert_replace(
            "hellogooDbye",
            r"hello(\w+)",
            r"hello\u\u\l\l\U$1",
            "helloGOodBYE",
        );
    }

    #[test]
    fn replace_has_javascript_semantics() {
        // Expected values are what `'hi'.replace(search, replace)` returns.
        assert_replace("hi", "hi", "hello", "hello");
        assert_replace("hi", "hi", r"\t", "\t");
        assert_replace("hi", "hi", r"\n", "\n");
        assert_replace("hi", "hi", r"\\t", r"\t");
        assert_replace("hi", "hi", r"\\n", r"\n");

        // Implicit capture group 0.
        assert_replace("hi", "hi", "hello$&", "hellohi");
        assert_replace("hi", "hi", "hello$0", "hellohi");
        assert_replace("hi", "hi", "hello$&1", "hellohi1");
        assert_replace("hi", "hi", "hello$01", "hellohi1");

        // `$nn` is a capture group only if the regex has that many groups.
        assert_replace("hi", "(hi)", "hello$10", "hellohi0");
        assert_replace("hi", "(hi)()()()()()()()()()", "hello$10", "hello");
        assert_replace("hi", "(hi)", "hello$100", "hellohi00");
        assert_replace("hi", "(hi)", "hello$20", "hello$20");
    }

    #[test]
    fn replace_string_for_complete_match() {
        assert_replace("bla", "bla", "hello", "hello");
        assert_replace("bla", "(bla)", "hello", "hello");
        assert_replace("bla", "(bla)", "hello$0", "hellobla");

        let search = r#"let\s+(\w+)\s*=\s*require\s*\(\s*['"]([\w\.\-/]+)\s*['"]\s*\)\s*"#;
        assert_replace(
            "let fs = require('fs')",
            search,
            "import * as $1 from '$2';",
            "import * as fs from 'fs';",
        );
        assert_replace(
            "let something = require('fs')",
            search,
            "import * as $1 from '$2';",
            "import * as something from 'fs';",
        );
        assert_replace(
            "let something = require('fs')",
            search,
            "import * as $1 from '$1';",
            "import * as something from 'something';",
        );
        assert_replace(
            "let something = require('fs')",
            search,
            "import * as $2 from '$1';",
            "import * as fs from 'something';",
        );
        assert_replace(
            "let something = require('fs')",
            search,
            "import * as $0 from '$0';",
            "import * as let something = require('fs') from 'let something = require('fs')';",
        );
        assert_replace(
            "let fs = require('fs')",
            search,
            "import * as $1 from '$2';",
            "import * as fs from 'fs';",
        );
        assert_replace("for ()", "for(.*)", "cat$1", "cat ()");

        // Monaco issue #18111.
        assert_replace(
            "HRESULT OnAmbientPropertyChange(DISPID   dispid);",
            r"\b\s{3}\b",
            " ",
            " ",
        );
    }

    #[test]
    fn replace_string_for_sub_string_match() {
        // Adapted: the `regex` crate has no lookahead. The original searches
        // were `this(?=.*bla)`, `(th)is(?=.*bla)`, `bla(?=\stext$)` and
        // `b(la)(?=\stext$)`; a lookahead is zero-width and holds at the first
        // match in this target, so dropping it yields the same captures.
        let target = "this is a bla text";
        assert_replace(target, "bla", "hello", "hello");
        assert_replace(target, "this", "that", "that");
        assert_replace(target, "(th)is", "$1at", "that");
        assert_replace(target, "(th)is", "$1e", "the");
        assert_replace(target, "(th)is", "$1ere", "there");
        assert_replace(target, "(th)is", "$1", "th");
        assert_replace(target, "(th)is", "ma$1", "math");
        assert_replace(target, "(th)is", "ma$1s", "maths");
        assert_replace(target, "(th)is", "$0", "this");
        assert_replace(target, "(th)is", "$0$1", "thisth");
        assert_replace(target, "bla", "foo", "foo");
        assert_replace(target, "b(la)", "f$1", "fla");
        assert_replace(target, "b(la)", "f$0", "fbla");
        assert_replace(target, "b(la)", "$0ah", "blaah");
    }

    #[test]
    fn issue_19740_missing_group_is_empty_not_undefined() {
        assert_replace("abcd", "a(z)?", "a{$1}", "a{}");
    }

    const CASE_PRESERVED_CASES: &[(&[&str], &str, &str)] = &[
        (&["abc"], "Def", "def"),
        (&["Abc"], "Def", "Def"),
        (&["ABC"], "Def", "DEF"),
        (&["abc", "Abc"], "Def", "def"),
        (&["Abc", "abc"], "Def", "Def"),
        (&["ABC", "abc"], "Def", "DEF"),
        (&["aBc", "abc"], "Def", "def"),
        (&["AbC"], "Def", "Def"),
        (&["aBC"], "Def", "def"),
        (&["aBc"], "DeF", "deF"),
        (&["Foo-Bar"], "newfoo-newbar", "Newfoo-Newbar"),
        (
            &["Foo-Bar-Abc"],
            "newfoo-newbar-newabc",
            "Newfoo-Newbar-Newabc",
        ),
        (&["Foo-Bar-abc"], "newfoo-newbar", "Newfoo-newbar"),
        (&["foo-Bar"], "newfoo-newbar", "newfoo-Newbar"),
        (&["foo-BAR"], "newfoo-newbar", "newfoo-NEWBAR"),
        (&["foO-BAR"], "NewFoo-NewBar", "newFoo-NEWBAR"),
        (&["Foo_Bar"], "newfoo_newbar", "Newfoo_Newbar"),
        (
            &["Foo_Bar_Abc"],
            "newfoo_newbar_newabc",
            "Newfoo_Newbar_Newabc",
        ),
        (&["Foo_Bar_abc"], "newfoo_newbar", "Newfoo_newbar"),
        (&["Foo_Bar-abc"], "newfoo_newbar-abc", "Newfoo_newbar-abc"),
        (&["foo_Bar"], "newfoo_newbar", "newfoo_Newbar"),
    ];

    fn to_matches(target: &[&'static str]) -> Vec<Option<&'static str>> {
        target.iter().copied().map(Some).collect()
    }

    #[test]
    fn build_replace_string_with_case_preserved_cases() {
        let last = [(&["Foo_BAR"][..], "newfoo_newbar", "Newfoo_NEWBAR")];
        for &(target, pattern, expected) in CASE_PRESERVED_CASES.iter().chain(&last) {
            let matches = to_matches(target);
            assert_eq!(
                build_replace_string_with_case_preserved(Some(&matches), pattern),
                expected,
                "{target:?} / {pattern:?}"
            );
        }
    }

    #[test]
    fn preserve_case() {
        let last = [(&["foo_BAR"][..], "newfoo_newbar", "newfoo_NEWBAR")];
        for &(target, pattern, expected) in CASE_PRESERVED_CASES.iter().chain(&last) {
            let matches = to_matches(target);
            assert_eq!(
                parse_replace_string(pattern).build_replace_string(Some(&matches), true),
                expected,
                "{target:?} / {pattern:?}"
            );
        }
    }

    // Not in Monaco's suite: covers this port's own API and char handling.

    #[test]
    fn static_patterns_and_non_ascii() {
        assert!(!parse_replace_string("").has_replacement_patterns());
        assert!(!parse_replace_string(r"a\U").has_replacement_patterns());
        assert!(parse_replace_string("$1").has_replacement_patterns());
        assert_eq!(
            ReplacePattern::from_static_value("x"),
            parse_replace_string("x")
        );
        assert_eq!(
            parse_replace_string("$1").build_replace_string(None, false),
            ""
        );
        // A `\` before a multi-byte char is kept, and case ops act per char.
        assert_replace("éclair", "(.+)", r"\é\u$1", r"\éÉclair");
        assert_eq!(
            build_replace_string_with_case_preserved(Some(&[Some("Été")]), "hiver"),
            "Hiver"
        );
    }
}
