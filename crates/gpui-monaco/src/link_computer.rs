//! Detects `http://`, `https://` and `file://` links in plain text.
//!
//! This is a port of Monaco's `linkComputer.ts`
//! (`vs/editor/common/languages/linkComputer.ts`, MIT licensed,
//! Copyright (c) Microsoft Corporation), together with its tests from
//! `linkComputer.test.ts`.
//!
//! The scanner is the same: a small `StateMachine` recognises the scheme
//! prefix, after which the link runs until a character that a
//! `CharacterClassifier` marks as force-terminating, with Monaco's special
//! cases for brackets, quotes, `*` and spaces inside `[...]`. Trailing
//! `.`, `,`, `;` and `:` are trimmed, as is a closing bracket that matches an
//! opening one just before the link.
//!
//! Like Monaco, the scan works on UTF-16 code units, so every column in a
//! [`Link`] range is a UTF-16 offset.
//!
//! Deviations from Monaco:
//! - Input is an iterator of lines rather than an `ILinkComputerTarget`, so
//!   there is no "null model" case. An empty input returns no links.
//! - Ranges are 0-based [`text::PointUtf16`] ranges with an exclusive end,
//!   not 1-based `IRange`s.
//! - The custom `stateMachine` parameter of `LinkComputer.computeLinks` is
//!   not exposed. The built-in machine is always used.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

use text::PointUtf16;

/// A link found by [`compute_links`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Where the link sits in the document. Rows are 0-based line indices.
    /// Columns are 0-based UTF-16 code unit offsets, and the end is
    /// exclusive. Start and end are always on the same row.
    pub range: Range<PointUtf16>,
    /// The link text, the same text that `range` covers.
    pub url: String,
}

/// Finds every `http://`, `https://` and `file://` link in a document.
///
/// `lines` are the document's lines in order, without line endings. Links
/// never span lines. Results come back in document order.
///
/// This ports Monaco's `LinkComputer.computeLinks`. See the [module
/// docs](self) for how it differs.
pub fn compute_links(lines: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<Link> {
    let state_machine = &*STATE_MACHINE;
    let classifier = &*CLASSIFIER;

    let mut result = Vec::new();
    let mut line: Vec<u16> = Vec::new();
    for (row, text) in lines.into_iter().enumerate() {
        let row = u32::try_from(row).expect("line count exceeds u32::MAX");
        line.clear();
        line.extend(text.as_ref().encode_utf16());
        let len = line.len();

        let mut j = 0;
        let mut link_begin_index = 0;
        let mut link_begin_ch_code = 0;
        let mut state = State::START;
        let mut has_open_parens = false;
        let mut has_open_square_bracket = false;
        let mut in_square_brackets = false;
        let mut has_open_curly_bracket = false;

        while j < len {
            let mut reset_state_machine = false;
            let ch_code = line[j];

            if state == State::ACCEPT {
                let ch_class = match ch_code {
                    OPEN_PAREN => {
                        has_open_parens = true;
                        CharacterClass::None
                    }
                    CLOSE_PAREN => {
                        if has_open_parens {
                            CharacterClass::None
                        } else {
                            CharacterClass::ForceTermination
                        }
                    }
                    OPEN_SQUARE_BRACKET => {
                        in_square_brackets = true;
                        has_open_square_bracket = true;
                        CharacterClass::None
                    }
                    CLOSE_SQUARE_BRACKET => {
                        in_square_brackets = false;
                        if has_open_square_bracket {
                            CharacterClass::None
                        } else {
                            CharacterClass::ForceTermination
                        }
                    }
                    OPEN_CURLY_BRACE => {
                        has_open_curly_bracket = true;
                        CharacterClass::None
                    }
                    CLOSE_CURLY_BRACE => {
                        if has_open_curly_bracket {
                            CharacterClass::None
                        } else {
                            CharacterClass::ForceTermination
                        }
                    }
                    // The following three rules make it that ' or " or ` are
                    // allowed inside links only if the link is wrapped by some
                    // other quote character.
                    SINGLE_QUOTE | DOUBLE_QUOTE | BACK_TICK => {
                        if link_begin_ch_code == ch_code {
                            CharacterClass::ForceTermination
                        } else if matches!(
                            link_begin_ch_code,
                            SINGLE_QUOTE | DOUBLE_QUOTE | BACK_TICK
                        ) {
                            CharacterClass::None
                        } else {
                            CharacterClass::ForceTermination
                        }
                    }
                    // `*` terminates a link if the link began with `*`.
                    ASTERISK => {
                        if link_begin_ch_code == ASTERISK {
                            CharacterClass::ForceTermination
                        } else {
                            CharacterClass::None
                        }
                    }
                    // Allow spaces between [ and ].
                    SPACE => {
                        if in_square_brackets {
                            CharacterClass::None
                        } else {
                            CharacterClass::ForceTermination
                        }
                    }
                    _ => classifier.get(ch_code),
                };

                // Check if the character terminates the link.
                if ch_class == CharacterClass::ForceTermination {
                    result.push(create_link(classifier, &line, row, link_begin_index, j));
                    reset_state_machine = true;
                }
            } else if state == State::END {
                let ch_class = if ch_code == OPEN_SQUARE_BRACKET {
                    // Allow the authority part to contain IPv6 addresses,
                    // which contain [ and ].
                    has_open_square_bracket = true;
                    CharacterClass::None
                } else {
                    classifier.get(ch_code)
                };

                // Check if the character terminates the link.
                if ch_class == CharacterClass::ForceTermination {
                    reset_state_machine = true;
                } else {
                    state = State::ACCEPT;
                }
            } else {
                state = state_machine.next_state(state, ch_code);
                if state == State::INVALID {
                    reset_state_machine = true;
                }
            }

            if reset_state_machine {
                state = State::START;
                has_open_parens = false;
                has_open_square_bracket = false;
                has_open_curly_bracket = false;

                // Record where the link started.
                link_begin_index = j + 1;
                link_begin_ch_code = ch_code;
            }

            j += 1;
        }

        if state == State::ACCEPT {
            result.push(create_link(classifier, &line, row, link_begin_index, len));
        }
    }
    result
}

/// Builds the link for `line[link_begin_index..link_end_index]`, trimming
/// characters a link cannot end in and a closing bracket that matches the
/// character just before the link.
fn create_link(
    classifier: &CharacterClassifier,
    line: &[u16],
    row: u32,
    link_begin_index: usize,
    link_end_index: usize,
) -> Link {
    // Do not allow the link to end in certain characters.
    let mut last_included_char_index = link_end_index - 1;
    loop {
        let ch_class = classifier.get(line[last_included_char_index]);
        if ch_class != CharacterClass::CannotEndIn {
            break;
        }
        last_included_char_index -= 1;
        if last_included_char_index <= link_begin_index {
            break;
        }
    }

    // Handle links enclosed in parens, square brackets and curlies.
    if link_begin_index > 0 {
        let char_code_before_link = line[link_begin_index - 1];
        let last_char_code_in_link = line[last_included_char_index];

        // Do not end in ) if ( is before the link start, and likewise for
        // ] and }.
        if matches!(
            (char_code_before_link, last_char_code_in_link),
            (OPEN_PAREN, CLOSE_PAREN)
                | (OPEN_SQUARE_BRACKET, CLOSE_SQUARE_BRACKET)
                | (OPEN_CURLY_BRACE, CLOSE_CURLY_BRACE)
        ) {
            last_included_char_index -= 1;
        }
    }

    let end = last_included_char_index + 1;
    Link {
        range: PointUtf16::new(row, column(link_begin_index))..PointUtf16::new(row, column(end)),
        url: String::from_utf16_lossy(&line[link_begin_index..end]),
    }
}

fn column(index: usize) -> u32 {
    u32::try_from(index).expect("line length exceeds u32::MAX")
}

const SPACE: u16 = b' ' as u16;
const DOUBLE_QUOTE: u16 = b'"' as u16;
const SINGLE_QUOTE: u16 = b'\'' as u16;
const OPEN_PAREN: u16 = b'(' as u16;
const CLOSE_PAREN: u16 = b')' as u16;
const ASTERISK: u16 = b'*' as u16;
const OPEN_SQUARE_BRACKET: u16 = b'[' as u16;
const CLOSE_SQUARE_BRACKET: u16 = b']' as u16;
const BACK_TICK: u16 = b'`' as u16;
const OPEN_CURLY_BRACE: u16 = b'{' as u16;
const CLOSE_CURLY_BRACE: u16 = b'}' as u16;

/// A state of the scheme-recognising [`StateMachine`].
///
/// Monaco's `State` const enum. It is a newtype over `u8` rather than a Rust
/// enum because Monaco allows custom states past `LAST_KNOWN_STATE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct State(u8);

#[allow(dead_code, reason = "kept for parity with Monaco's State enum")]
impl State {
    const INVALID: Self = Self(0);
    const START: Self = Self(1);
    const H: Self = Self(2);
    const HT: Self = Self(3);
    const HTT: Self = Self(4);
    const HTTP: Self = Self(5);
    const F: Self = Self(6);
    const FI: Self = Self(7);
    const FIL: Self = Self(8);
    const BEFORE_COLON: Self = Self(9);
    const AFTER_COLON: Self = Self(10);
    const ALMOST_THERE: Self = Self(11);
    const END: Self = Self(12);
    const ACCEPT: Self = Self(13);
    /// Marker; custom states may follow.
    const LAST_KNOWN_STATE: Self = Self(14);
}

/// A transition: from a state, on a UTF-16 code unit, to a state.
type Edge = (State, u16, State);

/// A dense transition table, Monaco's `StateMachine` (backed by its
/// `Uint8Matrix`). Missing transitions lead to [`State::INVALID`].
struct StateMachine {
    states: Vec<u8>,
    max_char_code: usize,
}

impl StateMachine {
    fn new(edges: &[Edge]) -> Self {
        let mut max_char_code = 0;
        let mut max_state = State::INVALID.0;
        for &(from, ch_code, to) in edges {
            max_char_code = max_char_code.max(usize::from(ch_code));
            max_state = max_state.max(from.0).max(to.0);
        }
        let max_char_code = max_char_code + 1;
        let max_state = usize::from(max_state) + 1;

        let mut states = vec![State::INVALID.0; max_state * max_char_code];
        for &(from, ch_code, to) in edges {
            states[usize::from(from.0) * max_char_code + usize::from(ch_code)] = to.0;
        }
        Self {
            states,
            max_char_code,
        }
    }

    fn next_state(&self, current_state: State, ch_code: u16) -> State {
        let ch_code = usize::from(ch_code);
        if ch_code >= self.max_char_code {
            return State::INVALID;
        }
        State(self.states[usize::from(current_state.0) * self.max_char_code + ch_code])
    }
}

/// State machine for `http://`, `https://` and `file://`.
static STATE_MACHINE: LazyLock<StateMachine> = LazyLock::new(|| {
    let edge = |from, ch: u8, to| (from, u16::from(ch), to);
    StateMachine::new(&[
        edge(State::START, b'h', State::H),
        edge(State::START, b'H', State::H),
        edge(State::START, b'f', State::F),
        edge(State::START, b'F', State::F),
        edge(State::H, b't', State::HT),
        edge(State::H, b'T', State::HT),
        edge(State::HT, b't', State::HTT),
        edge(State::HT, b'T', State::HTT),
        edge(State::HTT, b'p', State::HTTP),
        edge(State::HTT, b'P', State::HTTP),
        edge(State::HTTP, b's', State::BEFORE_COLON),
        edge(State::HTTP, b'S', State::BEFORE_COLON),
        edge(State::HTTP, b':', State::AFTER_COLON),
        edge(State::F, b'i', State::FI),
        edge(State::F, b'I', State::FI),
        edge(State::FI, b'l', State::FIL),
        edge(State::FI, b'L', State::FIL),
        edge(State::FIL, b'e', State::BEFORE_COLON),
        edge(State::FIL, b'E', State::BEFORE_COLON),
        edge(State::BEFORE_COLON, b':', State::AFTER_COLON),
        edge(State::AFTER_COLON, b'/', State::ALMOST_THERE),
        edge(State::ALMOST_THERE, b'/', State::END),
    ])
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharacterClass {
    None,
    ForceTermination,
    CannotEndIn,
}

/// Maps UTF-16 code units to a [`CharacterClass`], Monaco's
/// `CharacterClassifier`: a table for code units below 256 and a map for the
/// rest.
struct CharacterClassifier {
    ascii: [CharacterClass; 256],
    others: HashMap<u16, CharacterClass>,
    default_value: CharacterClass,
}

impl CharacterClassifier {
    fn new(default_value: CharacterClass) -> Self {
        Self {
            ascii: [default_value; 256],
            others: HashMap::new(),
            default_value,
        }
    }

    fn set(&mut self, ch_code: u16, value: CharacterClass) {
        match usize::from(ch_code) {
            index @ 0..256 => self.ascii[index] = value,
            _ => {
                self.others.insert(ch_code, value);
            }
        }
    }

    fn get(&self, ch_code: u16) -> CharacterClass {
        match usize::from(ch_code) {
            index @ 0..256 => self.ascii[index],
            _ => self
                .others
                .get(&ch_code)
                .copied()
                .unwrap_or(self.default_value),
        }
    }
}

static CLASSIFIER: LazyLock<CharacterClassifier> = LazyLock::new(|| {
    let mut classifier = CharacterClassifier::new(CharacterClass::None);

    const FORCE_TERMINATION_CHARACTERS: &str =
        " \t<>'\"、。｡､，．：；‘〈「『〔（［｛｢｣｝］）〕』」〉’｀～…|";
    for ch_code in FORCE_TERMINATION_CHARACTERS.encode_utf16() {
        classifier.set(ch_code, CharacterClass::ForceTermination);
    }

    const CANNOT_END_WITH_CHARACTERS: &str = ".,;:";
    for ch_code in CANNOT_END_WITH_CHARACTERS.encode_utf16() {
        classifier.set(ch_code, CharacterClass::CannotEndIn);
    }

    classifier
});

#[cfg(test)]
mod tests {
    use super::*;

    /// Monaco's `assertLink`: `extracted_link` is `text` with everything but
    /// the expected link blanked out with spaces (or tabs). Columns are
    /// counted in UTF-16 code units, as JavaScript string indices are.
    #[track_caller]
    fn assert_link(text: &str, extracted_link: &str) {
        let units: Vec<u16> = extracted_link.encode_utf16().collect();
        let is_blank = |&ch: &u16| ch == u16::from(b' ') || ch == u16::from(b'\t');
        let start = units.iter().position(|ch| !is_blank(ch)).unwrap_or(0);
        let end = units
            .iter()
            .rposition(|ch| !is_blank(ch))
            .map_or(0, |i| i + 1);

        let links = compute_links([text]);
        assert_eq!(
            links,
            vec![Link {
                range: PointUtf16::new(0, column(start))..PointUtf16::new(0, column(end)),
                url: String::from_utf16(&units[start..end]).unwrap(),
            }]
        );
    }

    // Monaco's "Null model" test passes `null`, which this API cannot
    // express. The closest equivalent is a document with no lines.
    #[test]
    fn null_model() {
        assert_eq!(compute_links(Vec::<&str>::new()), vec![]);
    }

    #[test]
    fn parsing() {
        assert_link("x = \"http://foo.bar\";", "     http://foo.bar  ");
        assert_link("x = (http://foo.bar);", "     http://foo.bar  ");
        assert_link("x = [http://foo.bar];", "     http://foo.bar  ");
        assert_link("x = 'http://foo.bar';", "     http://foo.bar  ");
        assert_link("x =  http://foo.bar ;", "     http://foo.bar  ");
        assert_link("x = <http://foo.bar>;", "     http://foo.bar  ");
        assert_link("x = {http://foo.bar};", "     http://foo.bar  ");
        assert_link("(see http://foo.bar)", "     http://foo.bar  ");
        assert_link("[see http://foo.bar]", "     http://foo.bar  ");
        assert_link("{see http://foo.bar}", "     http://foo.bar  ");
        assert_link("<see http://foo.bar>", "     http://foo.bar  ");
        assert_link(
            "<url>http://mylink.com</url>",
            "     http://mylink.com      ",
        );
        assert_link(
            "// Click here to learn more. https://go.microsoft.com/fwlink/?LinkID=513275&clcid=0x409",
            "                             https://go.microsoft.com/fwlink/?LinkID=513275&clcid=0x409",
        );
        assert_link(
            "// Click here to learn more. https://msdn.microsoft.com/en-us/library/windows/desktop/aa365247(v=vs.85).aspx",
            "                             https://msdn.microsoft.com/en-us/library/windows/desktop/aa365247(v=vs.85).aspx",
        );
        assert_link(
            "// https://github.com/projectkudu/kudu/blob/master/Kudu.Core/Scripts/selectNodeVersion.js",
            "   https://github.com/projectkudu/kudu/blob/master/Kudu.Core/Scripts/selectNodeVersion.js",
        );
        assert_link(
            "<!-- !!! Do not remove !!!   WebContentRef(link:https://go.microsoft.com/fwlink/?LinkId=166007, area:Admin, updated:2015, nextUpdate:2016, tags:SqlServer)   !!! Do not remove !!! -->",
            "                                                https://go.microsoft.com/fwlink/?LinkId=166007                                                                                        ",
        );
        assert_link(
            "For instructions, see https://go.microsoft.com/fwlink/?LinkId=166007.</value>",
            "                      https://go.microsoft.com/fwlink/?LinkId=166007         ",
        );
        assert_link(
            "For instructions, see https://msdn.microsoft.com/en-us/library/windows/desktop/aa365247(v=vs.85).aspx.</value>",
            "                      https://msdn.microsoft.com/en-us/library/windows/desktop/aa365247(v=vs.85).aspx         ",
        );
        assert_link(
            "x = \"https://en.wikipedia.org/wiki/Zürich\";",
            "     https://en.wikipedia.org/wiki/Zürich  ",
        );
        assert_link(
            "請參閱 http://go.microsoft.com/fwlink/?LinkId=761051。",
            "    http://go.microsoft.com/fwlink/?LinkId=761051 ",
        );
        assert_link(
            "（請參閱 http://go.microsoft.com/fwlink/?LinkId=761051）",
            "     http://go.microsoft.com/fwlink/?LinkId=761051 ",
        );

        assert_link("x = \"file:///foo.bar\";", "     file:///foo.bar  ");
        assert_link("x = \"file://c:/foo.bar\";", "     file://c:/foo.bar  ");
        assert_link(
            "x = \"file://shares/foo.bar\";",
            "     file://shares/foo.bar  ",
        );
        assert_link(
            "x = \"file://shäres/foo.bar\";",
            "     file://shäres/foo.bar  ",
        );
        assert_link(
            "Some text, then http://www.bing.com.",
            "                http://www.bing.com ",
        );
        assert_link(
            "let url = `http://***/_api/web/lists/GetByTitle('Teambuildingaanvragen')/items`;",
            "           http://***/_api/web/lists/GetByTitle('Teambuildingaanvragen')/items  ",
        );
    }

    #[test]
    fn issue_7855() {
        assert_link(
            "7. At this point, ServiceMain has been called.  There is no functionality presently in ServiceMain, but you can consult the [MSDN documentation](https://msdn.microsoft.com/en-us/library/windows/desktop/ms687414(v=vs.85).aspx) to add functionality as desired!",
            "                                                                                                                                                 https://msdn.microsoft.com/en-us/library/windows/desktop/ms687414(v=vs.85).aspx                                  ",
        );
    }

    /// issue #62278: "Ctrl + click to follow link" for IPv6 URLs
    #[test]
    fn issue_62278_ipv6_urls() {
        assert_link(
            "let x = \"http://[::1]:5000/connect/token\"",
            "         http://[::1]:5000/connect/token  ",
        );
    }

    /// issue #70254: bold links dont open in markdown file using editor mode
    /// with ctrl + click
    #[test]
    fn issue_70254_bold_links() {
        assert_link(
            "2. Navigate to **https://portal.azure.com**",
            "                 https://portal.azure.com  ",
        );
    }

    /// issue #86358: URL wrong recognition pattern
    #[test]
    fn issue_86358_pipe_terminates() {
        assert_link(
            "POST|https://portal.azure.com|2019-12-05|",
            "     https://portal.azure.com            ",
        );
    }

    /// issue #67022: Space as end of hyperlink isn't always good idea
    #[test]
    fn issue_67022_space_in_square_brackets() {
        assert_link(
            "aa  https://foo.bar/[this is foo site]  aa",
            "    https://foo.bar/[this is foo site]    ",
        );
    }

    /// issue #100353: Link detection stops at ＆(double-byte)
    #[test]
    fn issue_100353_double_byte_ampersand() {
        assert_link(
            "aa  http://tree-mark.chips.jp/レーズン＆ベリーミックス  aa",
            "    http://tree-mark.chips.jp/レーズン＆ベリーミックス    ",
        );
    }

    /// issue #121438: Link detection stops at【...】
    #[test]
    fn issue_121438_lenticular_brackets() {
        assert_link(
            "aa  https://zh.wikipedia.org/wiki/【我推的孩子】 aa",
            "    https://zh.wikipedia.org/wiki/【我推的孩子】   ",
        );
    }

    /// issue #121438: Link detection stops at《...》
    #[test]
    fn issue_121438_double_angle_brackets() {
        assert_link(
            "aa  https://zh.wikipedia.org/wiki/《新青年》编辑部旧址 aa",
            "    https://zh.wikipedia.org/wiki/《新青年》编辑部旧址   ",
        );
    }

    /// issue #121438: Link detection stops at “...”
    #[test]
    fn issue_121438_curly_double_quotes() {
        assert_link(
            "aa  https://zh.wikipedia.org/wiki/“常凯申”误译事件 aa",
            "    https://zh.wikipedia.org/wiki/“常凯申”误译事件   ",
        );
    }

    /// issue #150905: Colon after bare hyperlink is treated as its part
    #[test]
    fn issue_150905_trailing_colon() {
        assert_link(
            "https://site.web/page.html: blah blah blah",
            "https://site.web/page.html                ",
        );
    }

    // Monaco's test for issue #151631 ("Link parsing stoped where comments
    // include a single quote") is commented out upstream because of #156875,
    // so it is not ported.

    /// issue #156875: Links include quotes
    #[test]
    fn issue_156875_links_include_quotes() {
        assert_link(
            "\"This file has been converted from https://github.com/jeff-hykin/better-c-syntax/blob/master/autogenerated/c.tmLanguage.json\",",
            "                                   https://github.com/jeff-hykin/better-c-syntax/blob/master/autogenerated/c.tmLanguage.json  ",
        );
    }

    /// issue #225513: Cmd-Click doesn't work on JSDoc {@link URL|LinkText}
    /// format
    #[test]
    fn issue_225513_jsdoc_link_pipe() {
        assert_link(
            " * {@link https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Promise/withResolvers|Promise.withResolvers}",
            "          https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Promise/withResolvers                       ",
        );
    }
}
