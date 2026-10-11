//! Guessing a text's indentation (Monaco `indentationGuesser.ts`, MIT).

use crate::Rope;

const TAB: u16 = b'\t' as u16;
const SPACE: u16 = b' ' as u16;
const COMMA: u16 = b',' as u16;

/// Monaco `IGuessedIndentation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuessedIndentation {
    /// Columns per tab (and per indent), when indenting with spaces.
    pub tab_size: u32,
    /// Whether the text indents with spaces.
    pub insert_spaces: bool,
}

/// Monaco `SpacesDiffResult`.
#[derive(Default)]
struct SpacesDiff {
    spaces_diff: usize,
    looks_like_alignment: bool,
}

/// Monaco `spacesDiff`: how the indentation (`a_length` and `b_length`
/// UTF-16 units) of two lines differs.
fn spaces_diff(a: &[u16], a_length: usize, b: &[u16], b_length: usize, result: &mut SpacesDiff) {
    result.spaces_diff = 0;
    result.looks_like_alignment = false;

    // This can go both ways: "\t" and "\t    " count 1 tab and 4 spaces.
    let mut i = 0;
    while i < a_length && i < b_length && a[i] == b[i] {
        i += 1;
    }
    let count = |s: &[u16], len: usize| {
        let spaces = s[i..len].iter().filter(|&&c| c == SPACE).count();
        (spaces, len - i - spaces)
    };
    let (a_spaces, a_tabs) = count(a, a_length);
    let (b_spaces, b_tabs) = count(b, b_length);
    if (a_spaces > 0 && a_tabs > 0) || (b_spaces > 0 && b_tabs > 0) {
        return;
    }
    let tabs_diff = a_tabs.abs_diff(b_tabs);
    let spaces_diff = a_spaces.abs_diff(b_spaces);
    if tabs_diff == 0 {
        // The difference may be alignment, which is no hint.
        result.spaces_diff = spaces_diff;
        if spaces_diff > 0
            && b_spaces >= 1
            && b_spaces - 1 < a.len()
            && b_spaces < b.len()
            && b[b_spaces] != SPACE
            && a[b_spaces - 1] == SPACE
            && a.last() == Some(&COMMA)
        {
            // const a = b + c,
            //       d = b - c;
            result.looks_like_alignment = true;
        }
        return;
    }
    if spaces_diff % tabs_diff == 0 {
        result.spaces_diff = spaces_diff / tabs_diff;
    }
}

/// Monaco `guessIndentation`: whether the first 10 000 lines of `text`
/// indent with spaces, and with how many, falling back to the defaults.
pub fn guess_indentation(
    text: &Rope,
    default_tab_size: u32,
    default_insert_spaces: bool,
) -> GuessedIndentation {
    let line_count = text.line_count().min(10_000);
    // Lines with at least one tab, and with only spaces, in their indentation.
    let mut lines_with_tabs = 0;
    let mut lines_with_spaces = 0;
    // The last line with content and where its content starts.
    let mut previous_line: Vec<u16> = Vec::new();
    let mut previous_indentation = 0;
    // Prefer even guesses for the tab size, in [2, 8].
    const ALLOWED_TAB_SIZE_GUESSES: [usize; 7] = [2, 4, 6, 8, 3, 5, 7];
    const MAX_ALLOWED_TAB_SIZE_GUESS: usize = 8;
    let mut spaces_diff_count = [0usize; 9];
    let mut diff = SpacesDiff::default();

    for row in 0..line_count {
        let line: Vec<u16> = text.line(row).encode_utf16().collect();
        let mut has_content = false;
        let mut indentation = 0;
        let mut spaces = 0;
        let mut tabs = 0;
        for (j, &c) in line.iter().enumerate() {
            if c == TAB {
                tabs += 1;
            } else if c == SPACE {
                spaces += 1;
            } else {
                has_content = true;
                indentation = j;
                break;
            }
        }
        // Empty and whitespace-only lines are ignored.
        if !has_content {
            continue;
        }
        if tabs > 0 {
            lines_with_tabs += 1;
        } else if spaces > 1 {
            lines_with_spaces += 1;
        }

        spaces_diff(
            &previous_line,
            previous_indentation,
            &line,
            indentation,
            &mut diff,
        );
        // Alignment counts only when it matches the default spaces indent
        // (`- item` lists); otherwise the line is skipped.
        if diff.looks_like_alignment
            && !(default_insert_spaces && default_tab_size as usize == diff.spaces_diff)
        {
            continue;
        }
        if diff.spaces_diff <= MAX_ALLOWED_TAB_SIZE_GUESS {
            spaces_diff_count[diff.spaces_diff] += 1;
        }
        previous_line = line;
        previous_indentation = indentation;
    }

    let mut insert_spaces = default_insert_spaces;
    if lines_with_tabs != lines_with_spaces {
        insert_spaces = lines_with_tabs < lines_with_spaces;
    }
    let mut tab_size = default_tab_size;
    // The tab size is guessed only when indenting with spaces.
    if insert_spaces {
        let mut score = 0;
        for guess in ALLOWED_TAB_SIZE_GUESSES {
            if spaces_diff_count[guess] > score {
                score = spaces_diff_count[guess];
                tab_size = guess as u32;
            }
        }
        // 2 wins over 4 with at least 2/3 of 4's count (e.g. YAML nested
        // deeper in places).
        if tab_size == 4
            && spaces_diff_count[4] > 0
            && spaces_diff_count[2] > 0
            && spaces_diff_count[2] as f64 >= spaces_diff_count[4] as f64 * 2. / 3.
        {
            tab_size = 2;
        }
    }
    GuessedIndentation {
        tab_size,
        insert_spaces,
    }
}

/// Port of Monaco's `textModel.test.ts` guessing tests (`assertGuess`).
#[cfg(test)]
mod tests {
    use super::*;

    /// What a test expects of the tab size.
    enum Tab {
        /// It can't be guessed.
        Unknown,
        Known(u32),
        /// It's guessed only when defaulting to spaces.
        OnlyWithSpaces(u32),
    }

    fn check(lines: &[&str], default_spaces: bool, default_tab: u32, spaces: bool, tab: u32) {
        let text = Rope::from(lines.join("\n").as_str());
        let guessed = guess_indentation(&text, default_tab, default_spaces);
        assert_eq!(
            (guessed.insert_spaces, guessed.tab_size),
            (spaces, tab),
            "{lines:?} with defaults ({default_spaces}, {default_tab})"
        );
    }

    /// Monaco `assertGuess`.
    fn assert_guess(spaces: Option<bool>, tab: Tab, lines: &[&str]) {
        match (spaces, tab) {
            (None, Tab::Unknown) => {
                check(lines, true, 13370, true, 13370);
                check(lines, false, 13371, false, 13371);
            }
            (None, Tab::Known(n)) => {
                check(lines, true, 13370, true, n);
                check(lines, false, 13371, false, n);
            }
            (None, Tab::OnlyWithSpaces(n)) => {
                check(lines, true, 13370, true, n);
                check(lines, false, 13371, false, 13371);
            }
            (Some(spaces), Tab::Unknown) => {
                check(lines, true, 13370, spaces, 13370);
                check(lines, false, 13371, spaces, 13371);
            }
            (Some(spaces), Tab::Known(n)) => {
                check(lines, true, 13370, spaces, n);
                check(lines, false, 13371, spaces, n);
            }
            (Some(true), Tab::OnlyWithSpaces(n)) => {
                check(lines, true, 13370, true, n);
                check(lines, false, 13371, true, n);
            }
            (Some(false), Tab::OnlyWithSpaces(_)) => {
                check(lines, true, 13370, false, 13370);
                check(lines, false, 13371, false, 13371);
            }
        }
    }

    // Monaco: `guess indentation 1`.
    #[test]
    fn guess_indentation_1() {
        assert_guess(None, Tab::Unknown, &["x", "x", "x", "x", "x", "x", "x"]);
        assert_guess(
            Some(false),
            Tab::Unknown,
            &["\tx", "x", "x", "x", "x", "x", "x"],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &["  x", "x", "x", "x", "x", "x", "x"],
        );
        assert_guess(
            Some(false),
            Tab::Unknown,
            &["\tx", "\tx", "\tx", "\tx", "\tx", "\tx", "\tx"],
        );
        assert_guess(
            None,
            Tab::OnlyWithSpaces(2),
            &["\tx", "  x", "\tx", "  x", "\tx", "  x", "\tx", "  x"],
        );
        assert_guess(
            Some(false),
            Tab::Unknown,
            &["\tx", " x", "\tx", " x", "\tx", " x", "\tx", " x"],
        );
        assert_guess(
            Some(false),
            Tab::Unknown,
            &[
                "\tx", "\tx", "  x", "\tx", "  x", "\tx", "  x", "\tx", "  x",
            ],
        );
        assert_guess(
            Some(false),
            Tab::Unknown,
            &["\tx", "\tx", "x", "\tx", "x", "\tx", "x", "\tx", "  x"],
        );
        assert_guess(
            Some(false),
            Tab::Unknown,
            &["\tx", "\tx", "x", "\tx", "x", "\tx", "x", "\tx", "    x"],
        );
        assert_guess(
            Some(false),
            Tab::Unknown,
            &["\tx", "\tx", "x", "\tx", "x", "\tx", "  x", "\tx", "    x"],
        );
        assert_guess(
            None,
            Tab::Unknown,
            &["x", " x", " x", " x", " x", " x", " x", " x"],
        );
        assert_guess(
            Some(true),
            Tab::Unknown,
            &["x", "          x", " x", " x", " x", " x", " x", " x"],
        );
        assert_guess(
            None,
            Tab::Unknown,
            &[
                "",
                "  ",
                "    ",
                "      ",
                "        ",
                "          ",
                "            ",
                "              ",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(3),
            &[
                "x", "   x", "   x", "    x", "x", "   x", "   x", "    x", "x", "   x", "   x",
                "    x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(5),
            &[
                "x", "     x", "     x", "    x", "x", "     x", "     x", "    x", "x", "     x",
                "     x", "    x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(7),
            &[
                "x", "       x", "       x", "     x", "x", "       x", "       x", "    x", "x",
                "       x", "       x", "    x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "x", "  x", "  x", "  x", "  x", "x", "  x", "  x", "  x", "  x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "x", "  x", "  x", "x", "  x", "  x", "x", "  x", "  x", "x", "  x", "  x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "x", "  x", "    x", "x", "  x", "    x", "x", "  x", "    x", "x", "  x", "    x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "x", "  x", "  x", "    x", "x", "  x", "  x", "    x", "x", "  x", "  x", "    x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "x", "  x", "  x", "    x", "    x", "x", "  x", "  x", "    x", "    x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &["x", "  x", "    x", "    x", "x", "  x", "    x", "    x"],
        );
        assert_guess(
            Some(true),
            Tab::Known(4),
            &[
                "x", "    x", "    x", "x", "    x", "    x", "x", "    x", "    x", "x", "    x",
                "    x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "x", "  x", "    x", "    x", "      x", "x", "  x", "    x", "    x", "      x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "x",
                "  x",
                "    x",
                "    x",
                "      x",
                "      x",
                "        x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(4),
            &[
                "x",
                "    x",
                "    x",
                "    x",
                "     x",
                "        x",
                "x",
                "    x",
                "    x",
                "    x",
                "     x",
                "        x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(4),
            &[
                "x",
                "    x",
                "    x",
                "    x",
                "     x",
                "        x",
                "        x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(4),
            &[
                "x",
                "x",
                "    x",
                "    x",
                "     x",
                "        x",
                "        x",
                "x",
                "x",
                "    x",
                "    x",
                "     x",
                "        x",
                "        x",
            ],
        );
        assert_guess(
            Some(true),
            Tab::Known(3),
            &[
                "x", " x", " x", " x", " x", " x", "x", "   x", "    x", "    x",
            ],
        );
        assert_guess(Some(false), Tab::Unknown, &["\t x", " \t x", "\tx"]);
        assert_guess(Some(false), Tab::Unknown, &["\tx", "\t    x"]);
    }

    // Monaco: `issue #44991: Wrong indentation size auto-detection`.
    #[test]
    fn issue_44991_wrong_indentation_size_auto_detection() {
        assert_guess(
            Some(true),
            Tab::Known(4),
            &[
                "a = 10             # 0 space indent",
                "b = 5              # 0 space indent",
                "if a > 10:         # 0 space indent",
                "    a += 1         # 4 space indent      delta 4 spaces",
                "    if b > 5:      # 4 space indent",
                "        b += 1     # 8 space indent      delta 4 spaces",
                "        b += 1     # 8 space indent",
                "        b += 1     # 8 space indent",
                "# comment line 1   # 0 space indent      delta 8 spaces",
                "# comment line 2   # 0 space indent",
                "# comment line 3   # 0 space indent",
                "        b += 1     # 8 space indent      delta 8 spaces",
                "        b += 1     # 8 space indent",
                "        b += 1     # 8 space indent",
            ],
        );
    }

    // Monaco: `issue #55818: Broken indentation detection`.
    #[test]
    fn issue_55818_broken_indentation_detection() {
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "",
                "/* REQUIRE */",
                "",
                "const foo = require ( 'foo' ),",
                "      bar = require ( 'bar' );",
                "",
                "/* MY FN */",
                "",
                "function myFn () {",
                "",
                "  const asd = 1,",
                "        dsa = 2;",
                "",
                "  return bar ( foo ( asd ) );",
                "",
                "}",
                "",
                "/* EXPORT */",
                "",
                "module.exports = myFn;",
                "",
            ],
        );
    }

    // Monaco: `issue #70832: Broken indentation detection`.
    #[test]
    fn issue_70832_broken_indentation_detection() {
        assert_guess(
            Some(false),
            Tab::Unknown,
            &[
                "x", "x", "x", "x", "\tx", "\t\tx", "    x", "\t\tx", "\tx", "\t\tx", "\tx", "\tx",
                "\tx", "\tx", "x",
            ],
        );
    }

    // Monaco: `issue #62143: Broken indentation detection`.
    #[test]
    fn issue_62143_broken_indentation_detection() {
        assert_guess(Some(true), Tab::Known(2), &["x", "x", "  x", "  x"]);
        assert_guess(Some(true), Tab::Known(2), &["x", "  - item2", "  - item3"]);
    }

    // Monaco: `issue #84217: Broken indentation detection`.
    #[test]
    fn issue_84217_broken_indentation_detection() {
        assert_guess(
            Some(true),
            Tab::Known(4),
            &["def main():", "    print('hello')"],
        );
        assert_guess(
            Some(true),
            Tab::Known(4),
            &[
                "def main():",
                "    with open('foo') as fp:",
                "        print(fp.read())",
            ],
        );
    }

    // Monaco: `issue #65668: YAML file indented with 2 spaces`.
    #[test]
    fn issue_65668_yaml_file_indented_with_2_spaces() {
        assert_guess(
            Some(true),
            Tab::Known(2),
            &[
                "version: 2",
                "",
                "jobs:",
                "  build:",
                "    docker:",
                "      - circleci/golang:1.11",
                "",
                "  environment:",
                "    TEST_RESULTS: /tmp/test-results",
                "",
                "  steps:",
                "    - checkout",
                "    - run: mkdir -p $TEST_RESULTS",
                "",
                "    - restore_cache:",
                "        keys:",
                "          - v1-pkg-cache",
                "",
                "    - run:",
                "        name: dep ensure",
                "        command: dep ensure -v",
                "",
                "    - run:",
                "        name: Run unit tests",
                "        command: |",
                "          trap \"go-junit-report <${TEST_RESULTS}/go-test.out > ${TEST_RESULTS}/go-test-report.xml\" EXIT",
                "          go test -v ./... | tee ${TEST_RESULTS}/go-test.out",
                "",
                "    - run:",
                "        name: Build",
                "        command: go build -v",
                "",
                "    - save_cache:",
                "        key: v1-pkg-cache",
                "        paths:",
                "          - \"/go/pkg\"",
                "",
                "    - store_artifacts:",
                "        path: /tmp/test-results",
                "        destination: raw-test-output",
                "",
                "    - store_test_results:",
                "        path: /tmp/test-results",
            ],
        );
    }

    // Monaco: `issue #249040: 4-space indent should win over 2-space when predominant`.
    #[test]
    fn issue_249040_4_space_indent_should_win_over_2_space_when_predominant() {
        assert_guess(
            Some(true),
            Tab::Known(4),
            &[
                "function foo() {",
                "    let a = 1;",
                "    let b = 2;",
                "    if (true) {",
                "        console.log(a);",
                "        console.log(b);",
                "    }",
                "    const obj = {",
                "      x: 1,",
                "      y: 2",
                "    };",
                "    return obj;",
                "}",
            ],
        );
    }
}
