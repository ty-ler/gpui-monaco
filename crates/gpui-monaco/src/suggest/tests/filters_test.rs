//! Port of the fuzzy-score tests of Monaco's
//! `src/vs/base/test/common/filters.test.ts` (MIT), suite `Filters`.
//!
//! Skipped (by reason):
//! - Filters this crate doesn't port (only `fuzzyScore` and friends are):
//!   `or`, `PrefixFilter - case sensitive`, `PrefixFilter - ignore case`,
//!   `CamelCaseFilter`, `CamelCaseFilter - #19256`,
//!   `matchesContiguousSubString`, `matchesBaseContiguousSubString`,
//!   `matchesSubString`, `matchesSubString performance (#35346)`,
//!   `WordFilter`, `matchesWords performance (#309582)`.
//! - `Bad completion ranking changes valid variable name to class name when
//!   pressing "." #187055`: `test.skip` upstream (fails in Monaco too).
//! - Assertions commented out upstream in `topScore - fuzzyScore` stay out.

use crate::suggest::fuzzy::{
    FuzzyScore, FuzzyScoreOptions, any_score, fuzzy_score, fuzzy_score_at,
    fuzzy_score_graceful_aggressive_at, fuzzy_score_graceful_at,
};

/// A Monaco `FuzzyScorer`: `(pattern, patternPos, word, wordPos, options)`.
type Filter = fn(&str, usize, &str, usize, FuzzyScoreOptions) -> Option<FuzzyScore>;

const FUZZY: Filter = fuzzy_score_at;
const GRACEFUL: Filter = fuzzy_score_graceful_at;
const AGGRESSIVE: Filter = fuzzy_score_graceful_aggressive_at;
const ANY: Filter =
    |pattern, pattern_pos, word, word_pos, _| Some(any_score(pattern, pattern_pos, word, word_pos));

#[derive(Default)]
struct Opts {
    pattern_pos: usize,
    word_pos: usize,
    first_match_can_be_weak: bool,
}

/// Monaco `assertMatches`: `decorated` marks each matched UTF-16 unit of
/// `word` with a preceding `^`; `None` means no match.
fn assert_matches(pattern: &str, word: &str, decorated: Option<&str>, filter: Filter) {
    assert_matches_with(pattern, word, decorated, filter, Opts::default());
}

fn assert_matches_with(
    pattern: &str,
    word: &str,
    decorated: Option<&str>,
    filter: Filter,
    opts: Opts,
) {
    let result = filter(
        pattern,
        opts.pattern_pos,
        word,
        opts.word_pos,
        FuzzyScoreOptions {
            first_match_can_be_weak: opts.first_match_can_be_weak,
            boost_full_match: true,
        },
    );
    assert_eq!(
        decorated.is_some(),
        result.is_some(),
        "{pattern:?} vs {word:?}: expected {decorated:?}, got {result:?}"
    );
    let (Some(result), Some(decorated)) = (result, decorated) else {
        return;
    };
    let units: Vec<u16> = word.encode_utf16().collect();
    let mut actual: Vec<u16> = Vec::new();
    let mut pos = 0;
    for m in &result.matches {
        actual.extend_from_slice(&units[pos..m.start]);
        for &unit in &units[m.start..m.end] {
            actual.push(u16::from(b'^'));
            actual.push(unit);
        }
        pos = m.end;
    }
    actual.extend_from_slice(&units[pos..]);
    assert_eq!(
        String::from_utf16_lossy(&actual),
        decorated,
        "{pattern:?} vs {word:?}"
    );
}

/// Monaco `assertTopScore` (with `FuzzyScoreOptions.default`).
fn assert_top_score(filter: Filter, pattern: &str, expected: usize, words: &[&str]) {
    let mut top_score = -(100 * 10);
    let mut top_idx = 0;
    for (i, word) in words.iter().enumerate() {
        if let Some(m) = filter(pattern, 0, word, 0, FuzzyScoreOptions::MONACO_DEFAULT)
            && m.score > top_score
        {
            top_score = m.score;
            top_idx = i;
        }
    }
    assert_eq!(
        top_idx, expected,
        "{pattern} -> actual={} <> expected={}",
        words[top_idx], words[expected]
    );
}

fn weak() -> Opts {
    Opts {
        first_match_can_be_weak: true,
        ..Opts::default()
    }
}

fn at(pattern_pos: usize) -> Opts {
    Opts {
        pattern_pos,
        ..Opts::default()
    }
}

#[test]
fn fuzzy_score_23215() {
    assert_matches("tit", "win.tit", Some("win.^t^i^t"), FUZZY);
    assert_matches("title", "win.title", Some("win.^t^i^t^l^e"), FUZZY);
    assert_matches(
        "WordCla",
        "WordCharacterClassifier",
        Some("^W^o^r^dCharacter^C^l^assifier"),
        FUZZY,
    );
    assert_matches(
        "WordCCla",
        "WordCharacterClassifier",
        Some("^W^o^r^d^Character^C^l^assifier"),
        FUZZY,
    );
}

#[test]
fn fuzzy_score_23332() {
    assert_matches("dete", "\"editor.quickSuggestionsDelay\"", None, FUZZY);
}

#[test]
fn fuzzy_score_23190() {
    assert_matches(
        "c:\\do",
        "& 'C:\\Documents and Settings'",
        Some("& '^C^:^\\^D^ocuments and Settings'"),
        FUZZY,
    );
    assert_matches(
        "c:\\do",
        "& 'c:\\Documents and Settings'",
        Some("& '^c^:^\\^D^ocuments and Settings'"),
        FUZZY,
    );
}

#[test]
fn fuzzy_score_23581() {
    assert_matches(
        "close",
        "css.lint.importStatement",
        Some("^css.^lint.imp^ort^Stat^ement"),
        FUZZY,
    );
    assert_matches(
        "close",
        "css.colorDecorators.enable",
        Some("^css.co^l^orDecorator^s.^enable"),
        FUZZY,
    );
    assert_matches(
        "close",
        "workbench.quickOpen.closeOnFocusOut",
        Some("workbench.quickOpen.^c^l^o^s^eOnFocusOut"),
        FUZZY,
    );
    assert_top_score(
        FUZZY,
        "close",
        2,
        &[
            "css.lint.importStatement",
            "css.colorDecorators.enable",
            "workbench.quickOpen.closeOnFocusOut",
        ],
    );
}

#[test]
fn fuzzy_score_23458() {
    assert_matches(
        "highlight",
        "editorHoverHighlight",
        Some("editorHover^H^i^g^h^l^i^g^h^t"),
        FUZZY,
    );
    assert_matches(
        "hhighlight",
        "editorHoverHighlight",
        Some("editor^Hover^H^i^g^h^l^i^g^h^t"),
        FUZZY,
    );
    assert_matches("dhhighlight", "editorHoverHighlight", None, FUZZY);
}

#[test]
fn fuzzy_score_23746() {
    assert_matches("-moz", "-moz-foo", Some("^-^m^o^z-foo"), FUZZY);
    assert_matches("moz", "-moz-foo", Some("-^m^o^z-foo"), FUZZY);
    assert_matches("moz", "-moz-animation", Some("-^m^o^z-animation"), FUZZY);
    assert_matches("moza", "-moz-animation", Some("-^m^o^z-^animation"), FUZZY);
}

#[test]
fn fuzzy_score_basic() {
    let cases: &[(&str, &str, Option<&str>)] = &[
        ("ab", "abA", Some("^a^bA")),
        ("ccm", "cacmelCase", Some("^ca^c^melCase")),
        ("bti", "the_black_knight", None),
        ("ccm", "camelCase", None),
        ("cmcm", "camelCase", None),
        ("BK", "the_black_knight", Some("the_^black_^knight")),
        ("KeyboardLayout=", "KeyboardLayout", None),
        (
            "LLL",
            "SVisualLoggerLogsList",
            Some("SVisual^Logger^Logs^List"),
        ),
        ("LLLL", "SVilLoLosLi", None),
        ("LLLL", "SVisualLoggerLogsList", None),
        ("TEdit", "TextEdit", Some("^Text^E^d^i^t")),
        ("TEdit", "TextEditor", Some("^Text^E^d^i^tor")),
        ("TEdit", "Textedit", Some("^Text^e^d^i^t")),
        ("TEdit", "text_edit", Some("^text_^e^d^i^t")),
        (
            "TEditDit",
            "TextEditorDecorationType",
            Some("^Text^E^d^i^tor^Decorat^ion^Type"),
        ),
        (
            "TEdit",
            "TextEditorDecorationType",
            Some("^Text^E^d^i^torDecorationType"),
        ),
        ("Tedit", "TextEdit", Some("^Text^E^d^i^t")),
        ("ba", "?AB?", None),
        ("bkn", "the_black_knight", Some("the_^black_^k^night")),
        ("bt", "the_black_knight", Some("the_^black_knigh^t")),
        ("ccm", "camelCasecm", Some("^camel^Casec^m")),
        ("fdm", "findModel", Some("^fin^d^Model")),
        ("fob", "foobar", Some("^f^oo^bar")),
        ("fobz", "foobar", None),
        ("foobar", "foobar", Some("^f^o^o^b^a^r")),
        (
            "form",
            "editor.formatOnSave",
            Some("editor.^f^o^r^matOnSave"),
        ),
        ("g p", "Git: Pull", Some("^Git:^ ^Pull")),
        ("g p", "Git: Pull", Some("^Git:^ ^Pull")),
        ("gip", "Git: Pull", Some("^G^it: ^Pull")),
        ("gip", "Git: Pull", Some("^G^it: ^Pull")),
        ("gp", "Git: Pull", Some("^Git: ^Pull")),
        ("gp", "Git_Git_Pull", Some("^Git_Git_^Pull")),
        ("is", "ImportStatement", Some("^Import^Statement")),
        ("is", "isValid", Some("^i^sValid")),
        ("lowrd", "lowWord", Some("^l^o^wWo^r^d")),
        ("myvable", "myvariable", Some("^m^y^v^aria^b^l^e")),
        ("no", "", None),
        ("no", "match", None),
        ("ob", "foobar", None),
        (
            "sl",
            "SVisualLoggerLogsList",
            Some("^SVisual^LoggerLogsList"),
        ),
        (
            "sllll",
            "SVisualLoggerLogsList",
            Some("^SVisua^l^Logger^Logs^List"),
        ),
        ("Three", "HTMLHRElement", None),
        ("Three", "Three", Some("^T^h^r^e^e")),
        ("fo", "barfoo", None),
        ("fo", "bar_foo", Some("bar_^f^oo")),
        ("fo", "bar_Foo", Some("bar_^F^oo")),
        ("fo", "bar foo", Some("bar ^f^oo")),
        ("fo", "bar.foo", Some("bar.^f^oo")),
        ("fo", "bar/foo", Some("bar/^f^oo")),
        ("fo", "bar\\foo", Some("bar\\^f^oo")),
    ];
    for &(pattern, word, decorated) in cases {
        assert_matches(pattern, word, decorated, FUZZY);
    }
}

#[test]
fn fuzzy_score_first_match_can_be_weak() {
    assert_matches_with(
        "Three",
        "HTMLHRElement",
        Some("H^TML^H^R^El^ement"),
        FUZZY,
        weak(),
    );
    assert_matches_with("tor", "constructor", Some("construc^t^o^r"), FUZZY, weak());
    assert_matches_with("ur", "constructor", Some("constr^ucto^r"), FUZZY, weak());
    assert_top_score(FUZZY, "tor", 2, &["constructor", "Thor", "cTor"]);
}

#[test]
fn fuzzy_score_many_matches() {
    let word = "a".repeat(273);
    let decorated = format!("^a^a^a^a^a^a{}", "a".repeat(267));
    assert_matches("aaaaaa", &word, Some(&decorated), FUZZY);
}

#[test]
fn freeze_when_fjfj_to_jfjf_91807() {
    assert_matches("jfjfj", "fjfjfjfjfjfjfjfjfjfjfj", None, FUZZY);
    assert_matches(
        "jfjfjfjfjfjfjfjfjfj",
        "fjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfj",
        None,
        FUZZY,
    );
    assert_matches(
        "jfjfjfjfjfjfjfjfjfjjfjfjfjfjfjfjfjfjfjjfjfjfjfjfjfjfjfjfjjfjfjfjfjfjfjfjfjfjjfjfjfjfjfjfjfjfjfjjfjfjfjfjfjfjfjfjfj",
        "fjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfj",
        None,
        FUZZY,
    );
    // strong match
    assert_matches(
        "jfjfjfjfjfjfjfjfjfj",
        "fJfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfj",
        Some("f^J^f^j^f^j^f^j^f^j^f^j^f^j^f^j^f^j^f^jfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfj"),
        FUZZY,
    );
    // any match
    assert_matches_with(
        "jfjfjfjfjfjfjfjfjfj",
        "fjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfj",
        Some("f^j^f^j^f^j^f^j^f^j^f^j^f^j^f^j^f^j^f^jfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfjfj"),
        FUZZY,
        weak(),
    );
}

#[test]
fn fuzzy_score_26423() {
    assert_matches("baba", "abababab", None, FUZZY);
    assert_matches("fsfsfs", "dsafdsafdsafdsafdsafdsafdsafasdfdsa", None, FUZZY);
    assert_matches(
        "fsfsfsfsfsfsfsf",
        "dsafdsafdsafdsafdsafdsafdsafasdfdsafdsafdsafdsafdsfdsafdsfdfdfasdnfdsajfndsjnafjndsajlknfdsa",
        None,
        FUZZY,
    );
}

#[test]
fn haxe_metadata_completion_26995() {
    assert_matches("f", ":Foo", Some(":^Foo"), FUZZY);
    assert_matches("f", ":foo", Some(":^foo"), FUZZY);
}

#[test]
fn separator_only_match_is_not_weak_79558() {
    assert_matches(".", "foo.bar", Some("foo^.bar"), FUZZY);
}

#[test]
fn long_pattern_does_not_explode_26511() {
    let word = "a".repeat(122);
    let pattern = "a".repeat(119);
    let _ = fuzzy_score(&pattern, &word, FuzzyScoreOptions::MONACO_DEFAULT);
}

#[test]
fn sort_text_from_language_server_26096() {
    assert_matches_with("  ", "  group", None, FUZZY, at(2));
    assert_matches_with("  g", "  group", Some("  ^group"), FUZZY, at(2));
    assert_matches("g", "  group", Some("  ^group"), FUZZY);
    assert_matches("g g", "  groupGroup", None, FUZZY);
    assert_matches("g g", "  group Group", Some("  ^group^ ^Group"), FUZZY);
    assert_matches_with(
        " g g",
        "  group Group",
        Some("  ^group^ ^Group"),
        FUZZY,
        at(1),
    );
    assert_matches("zz", "zzGroup", Some("^z^zGroup"), FUZZY);
    assert_matches("zzg", "zzGroup", Some("^z^z^Group"), FUZZY);
    assert_matches("g", "zzGroup", Some("zz^Group"), FUZZY);
}

#[test]
fn pattern_pos_79815() {
    assert_matches("p", "prop", Some("^prop"), FUZZY);
    assert_matches_with(":p", "prop", Some("^prop"), FUZZY, at(1));
    assert_matches_with(":p", "prop", None, FUZZY, at(2));
    assert_matches_with(
        ":p",
        "proP",
        Some("pro^P"),
        FUZZY,
        Opts {
            pattern_pos: 1,
            word_pos: 1,
            ..Opts::default()
        },
    );
    assert_matches_with(
        ":p",
        "aprop",
        Some("a^prop"),
        FUZZY,
        Opts {
            pattern_pos: 1,
            first_match_can_be_weak: true,
            ..Opts::default()
        },
    );
    assert_matches_with(":p", "aprop", None, FUZZY, at(1));
}

#[test]
fn top_score_fuzzy_score() {
    assert_top_score(
        FUZZY,
        "cons",
        2,
        &["ArrayBufferConstructor", "Console", "console"],
    );
    assert_top_score(FUZZY, "Foo", 1, &["foo", "Foo", "foo"]);

    // #24904
    assert_top_score(
        FUZZY,
        "onMess",
        1,
        &["onmessage", "onMessage", "onThisMegaEscape"],
    );

    assert_top_score(FUZZY, "CC", 1, &["camelCase", "CamelCase"]);
    assert_top_score(FUZZY, "cC", 0, &["camelCase", "CamelCase"]);

    assert_top_score(FUZZY, "p", 4, &["parse", "posix", "pafdsa", "path", "p"]);
    assert_top_score(FUZZY, "pa", 0, &["parse", "pafdsa", "path"]);

    // issue #14583
    assert_top_score(
        FUZZY,
        "log",
        3,
        &[
            "HTMLOptGroupElement",
            "ScrollLogicalPosition",
            "SVGFEMorphologyElement",
            "log",
            "logger",
        ],
    );
    assert_top_score(FUZZY, "e", 2, &["AbstractWorker", "ActiveXObject", "else"]);

    // issue #14446
    assert_top_score(
        FUZZY,
        "workbench.sideb",
        1,
        &[
            "workbench.editor.defaultSideBySideLayout",
            "workbench.sideBar.location",
        ],
    );

    // issue #11423
    assert_top_score(
        FUZZY,
        "editor.r",
        2,
        &[
            "diffEditor.renderSideBySide",
            "editor.overviewRulerlanes",
            "editor.renderControlCharacter",
            "editor.renderWhitespace",
        ],
    );

    assert_top_score(FUZZY, "-mo", 1, &["-ms-ime-mode", "-moz-columns"]);
    // dupe, issue #14861
    assert_top_score(
        FUZZY,
        "convertModelPosition",
        0,
        &[
            "convertModelPositionToViewPosition",
            "convertViewToModelPosition",
        ],
    );
    // dupe, issue #14942
    assert_top_score(FUZZY, "is", 0, &["isValidViewletId", "import statement"]);

    assert_top_score(
        FUZZY,
        "title",
        1,
        &["files.trimTrailingWhitespace", "window.title"],
    );

    assert_top_score(FUZZY, "const", 1, &["constructor", "const", "cuOnstrul"]);
}

#[test]
fn unexpected_suggestion_scoring_28791() {
    assert_top_score(FUZZY, "_lines", 1, &["_lineStarts", "_lines"]);
    assert_top_score(FUZZY, "_lines", 1, &["_lineS", "_lines"]);
    assert_top_score(FUZZY, "_lineS", 0, &["_lineS", "_lines"]);
}

#[test]
fn html_closing_tag_proposal_38880() {
    assert_matches_with("\t\t<", "\t\t</body>", Some("^\t^\t^</body>"), FUZZY, at(0));
    assert_matches_with("\t\t<", "\t\t</body>", Some("\t\t^</body>"), FUZZY, at(2));
    assert_matches_with("\t<", "\t</body>", Some("\t^</body>"), FUZZY, at(1));
}

#[test]
fn fuzzy_score_graceful() {
    assert_matches("rlut", "result", None, FUZZY);
    assert_matches("rlut", "result", Some("^res^u^l^t"), GRACEFUL);

    assert_matches("cno", "console", Some("^co^ns^ole"), FUZZY);
    assert_matches("cno", "console", Some("^co^ns^ole"), GRACEFUL);
    assert_matches("cno", "console", Some("^c^o^nsole"), AGGRESSIVE);
    assert_matches("cno", "co_new", Some("^c^o_^new"), GRACEFUL);
    assert_matches("cno", "co_new", Some("^c^o_^new"), AGGRESSIVE);
}

#[test]
fn list_highlight_all_characters_66923() {
    assert_matches(
        "foo",
        "barbarbarbarbarbarbarbarbarbarbarbarbarbarbarbar_foo",
        Some("barbarbarbarbarbarbarbarbarbarbarbarbarbarbarbar_^f^o^o"),
        FUZZY,
    );
}

#[test]
fn truncated_filter_text_74133() {
    assert_matches(
        "foo",
        "ffffffffffffffffffffffffffffbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbar_foo",
        Some(
            "ffffffffffffffffffffffffffffbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbar_^f^o^o",
        ),
        FUZZY,
    );
    assert_matches(
        "Aoo",
        "Affffffffffffffffffffffffffffbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbar_foo",
        Some(
            "^Affffffffffffffffffffffffffffbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbar_f^o^o",
        ),
        FUZZY,
    );
    assert_matches(
        "foo",
        "Gffffffffffffffffffffffffffffbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbarbar_foo",
        None,
        FUZZY,
    );
}

#[test]
fn go_to_symbol_exact_method_name_84787() {
    let options = FuzzyScoreOptions {
        first_match_can_be_weak: true,
        boost_full_match: true,
    };
    assert!(fuzzy_score_at(":get", 1, "get", 0, options).is_some());
}

#[test]
fn wrong_highlight_after_emoji_113404() {
    assert_matches(
        "di",
        "\u{2728}div classname=\"\"></div>",
        Some("\u{2728}^d^iv classname=\"\"></div>"),
        FUZZY,
    );
    assert_matches(
        "di",
        "adiv classname=\"\"></div>",
        Some("adiv classname=\"\"></^d^iv>"),
        FUZZY,
    );
}

#[test]
fn suggestion_is_highlighted_85826() {
    assert_matches(
        "SemanticTokens",
        "SemanticTokensEdits",
        Some("^S^e^m^a^n^t^i^c^T^o^k^e^n^sEdits"),
        FUZZY,
    );
    assert_matches(
        "SemanticTokens",
        "SemanticTokensEdits",
        Some("^S^e^m^a^n^t^i^c^T^o^k^e^n^sEdits"),
        AGGRESSIVE,
    );
}

#[test]
fn highlighting_text_in_front_of_cursor_115250() {
    assert_matches("lo", "log", Some("^l^og"), FUZZY);
    assert_matches(".lo", "log", Some("^l^og"), ANY);
    assert_matches(".", "log", Some("log"), ANY);
}

#[test]
fn any_score_does_not_require_strong_first_match() {
    assert_matches("bar", "foobAr", Some("foo^b^A^r"), ANY);
    assert_matches("bar", "foobar", Some("foo^b^a^r"), ANY);
}

#[test]
fn configurable_full_match_boost() {
    let prefix = "create";
    let a = "createModelServices";
    let b = "create";
    let boost = FuzzyScoreOptions {
        boost_full_match: true,
        first_match_can_be_weak: true,
    };

    let a_boost = fuzzy_score(prefix, a, boost).unwrap();
    let b_boost = fuzzy_score(prefix, b, boost).unwrap();
    assert!(a_boost.score < b_boost.score);

    // also works with wordStart > 0 (#187921)
    let word_prefix = "$(symbol-function) ";
    let a_boost = fuzzy_score_at(
        prefix,
        0,
        &format!("{word_prefix}{a}"),
        word_prefix.len(),
        boost,
    )
    .unwrap();
    let b_boost = fuzzy_score_at(
        prefix,
        0,
        &format!("{word_prefix}{b}"),
        word_prefix.len(),
        boost,
    )
    .unwrap();
    assert!(a_boost.score < b_boost.score);

    let no_boost = FuzzyScoreOptions {
        boost_full_match: false,
        first_match_can_be_weak: true,
    };
    let a_score = fuzzy_score(prefix, a, no_boost).unwrap();
    let b_score = fuzzy_score(prefix, b, no_boost).unwrap();
    assert_eq!(a_score.score, b_score.score);
}

#[test]
fn whole_word_match_over_first_letter_147423() {
    assert_matches("i", "machine/{id}", Some("machine/{^id}"), FUZZY);
    assert_matches("ok", "obobobf{ok}/user", Some("^obobobf{o^k}/user"), FUZZY);
}

// ----- not from Monaco ------------------------------------------------------------

#[test]
fn matches_are_utf16_ranges() {
    // `𝔸` is two UTF-16 units, so `b` sits at unit 3.
    let score = fuzzy_score("b", "𝔸_b", FuzzyScoreOptions::MONACO_DEFAULT).unwrap();
    assert_eq!(score.matches, vec![3..4]);
}
