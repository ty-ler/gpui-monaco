//! Port of Monaco's `src/vs/editor/test/common/modes/supports/onEnter.test.ts`
//! (MIT), suite `OnEnter`, run against the onEnter logic of
//! `LanguageSupport`.

use syntax::{IndentAction, LanguageConfiguration, Pattern};

use super::language_configs::{brackets, javascript_on_enter_rules, on_enter};
use crate::cursor::AutoIndent;
use crate::cursor::LanguageContext;
use crate::cursor::language::LanguageSupport;

fn with_support(config: LanguageConfiguration, f: impl FnOnce(&LanguageSupport)) {
    let ctx = LanguageContext::untokenized(&config);
    let support = LanguageSupport::new(&ctx);
    f(&support);
}

#[test]
fn uses_brackets() {
    let config = LanguageConfiguration {
        brackets: brackets(&[("(", ")"), ("begin", "end")]),
        ..Default::default()
    };
    with_support(config, |support| {
        let test = |before: &str, after: &str, expected: Option<IndentAction>| {
            let actual = support
                .on_enter(AutoIndent::Advanced, "", before, after)
                .map(|r| r.indent_action);
            assert_eq!(actual, expected, "{before:?} | {after:?}");
        };
        use IndentAction::*;
        test("a", "", Option::None);
        test("", "b", Option::None);
        test("(", "b", Some(Indent));
        test("a", ")", Option::None);
        test("begin", "ending", Some(Indent));
        test("abegin", "end", Option::None);
        test("begin", ")", Some(Indent));
        test("begin", "end", Some(IndentOutdent));
        test("begin ", " end", Some(IndentOutdent));
        test(" begin", "end//as", Some(IndentOutdent));
        test("(", ")", Some(IndentOutdent));
        test("( ", ")", Some(IndentOutdent));
        test("a(", ")b", Some(IndentOutdent));

        test("(", "", Some(Indent));
        test("(", "foo", Some(Indent));
        test("begin", "foo", Some(Indent));
        test("begin", "", Some(Indent));
    });
}

#[track_caller]
fn check(
    support: &LanguageSupport,
    previous: &str,
    before: &str,
    after: &str,
    expected: Option<IndentAction>,
    append_text: Option<&str>,
    remove_text: u32,
) {
    let actual = support.on_enter(AutoIndent::Advanced, previous, before, after);
    match expected {
        None => assert!(actual.is_none(), "isNull:{before}"),
        Some(expected) => {
            let actual = actual.unwrap_or_else(|| panic!("isNotNull:{before}"));
            assert_eq!(actual.indent_action, expected, "indentAction:{before}");
            if let Some(append_text) = append_text {
                assert_eq!(
                    actual.append_text.as_deref(),
                    Some(append_text),
                    "appendText:{before}"
                );
            }
            if remove_text != 0 {
                assert_eq!(actual.remove_text, Some(remove_text), "removeText:{before}");
            }
        }
    }
}

#[test]
fn issue_121125_on_enter_rules_with_global_modifier() {
    let config = LanguageConfiguration {
        on_enter_rules: vec![on_enter(
            Pattern::new(r"^\s*\/{3}.*$", "gm"),
            None,
            None,
            IndentAction::Outdent,
            Some("/// "),
            None,
        )],
        ..Default::default()
    };
    with_support(config, |s| {
        check(
            s,
            "/// line",
            "/// line",
            "",
            Some(IndentAction::Outdent),
            Some("/// "),
            0,
        );
        check(
            s,
            "/// line",
            "/// line",
            "",
            Some(IndentAction::Outdent),
            Some("/// "),
            0,
        );
    });
}

#[test]
fn uses_reg_exp_rules() {
    let config = LanguageConfiguration {
        on_enter_rules: javascript_on_enter_rules(),
        ..Default::default()
    };
    with_support(config, |s| {
        use IndentAction::*;
        let none = Option::<IndentAction>::None;
        check(s, "", "\t/**", " */", Some(IndentOutdent), Some(" * "), 0);
        check(s, "", "\t/**", "", Some(None), Some(" * "), 0);
        check(s, "", "\t/** * / * / * /", "", Some(None), Some(" * "), 0);
        check(s, "", "\t/** /*", "", Some(None), Some(" * "), 0);
        check(s, "", "/**", "", Some(None), Some(" * "), 0);
        check(s, "", "\t/**/", "", none, Option::None, 0);
        check(s, "", "\t/***/", "", none, Option::None, 0);
        check(s, "", "\t/*******/", "", none, Option::None, 0);
        check(s, "", "\t/** * * * * */", "", none, Option::None, 0);
        check(s, "", "\t/** */", "", none, Option::None, 0);
        check(s, "", "\t/** asdfg */", "", none, Option::None, 0);
        check(s, "", "\t/* asdfg */", "", none, Option::None, 0);
        check(s, "", "\t/* asdfg */", "", none, Option::None, 0);
        check(s, "", "\t/** asdfg */", "", none, Option::None, 0);
        check(s, "", "*/", "", none, Option::None, 0);
        check(s, "", "\t/*", "", none, Option::None, 0);
        check(s, "", "\t*", "", none, Option::None, 0);

        check(s, "\t/**", "\t *", "", Some(None), Some("* "), 0);
        check(s, "\t * something", "\t *", "", Some(None), Some("* "), 0);
        check(s, "\t *", "\t *", "", Some(None), Some("* "), 0);

        check(s, "", "\t */", "", Some(None), Option::None, 1);
        check(s, "", "\t * */", "", Some(None), Option::None, 1);
        check(s, "", "\t * * / * / * / */", "", none, Option::None, 0);

        check(s, "\t/**", "\t * ", "", Some(None), Some("* "), 0);
        check(s, "\t * something", "\t * ", "", Some(None), Some("* "), 0);
        check(s, "\t *", "\t * ", "", Some(None), Some("* "), 0);

        check(s, "/**", " * ", "", Some(None), Some("* "), 0);
        check(s, " * something", " * ", "", Some(None), Some("* "), 0);
        check(s, " *", " * asdfsfagadfg", "", Some(None), Some("* "), 0);

        for before in [
            " * asdfsfagadfg * * * ",
            " * /*",
            " * asdfsfagadfg * / * / * /",
            " * asdfsfagadfg * / * / * /*",
        ] {
            check(s, "/**", before, "", Some(None), Some("* "), 0);
            check(s, " * something", before, "", Some(None), Some("* "), 0);
            check(s, " *", before, "", Some(None), Some("* "), 0);
        }

        check(s, "", " */", "", Some(None), Option::None, 1);
        check(s, " */", " * test() {", "", Some(Indent), Option::None, 0);
        check(s, "", "\t */", "", Some(None), Option::None, 1);
        check(s, "", "\t\t */", "", Some(None), Option::None, 1);
        check(s, "", "   */", "", Some(None), Option::None, 1);
        check(s, "", "     */", "", Some(None), Option::None, 1);
        check(s, "", "\t     */", "", Some(None), Option::None, 1);
        check(
            s,
            "",
            " *--------------------------------------------------------------------------------------------*/",
            "",
            Some(None),
            Option::None,
            1,
        );

        // issue #43469
        check(
            s,
            "class A {",
            "    * test() {",
            "",
            Some(Indent),
            Option::None,
            0,
        );
        check(s, "", "    * test() {", "", Some(Indent), Option::None, 0);
        check(
            s,
            "    ",
            "    * test() {",
            "",
            Some(Indent),
            Option::None,
            0,
        );
        check(
            s,
            "class A {",
            "  * test() {",
            "",
            Some(Indent),
            Option::None,
            0,
        );
        check(s, "", "  * test() {", "", Some(Indent), Option::None, 0);
        check(s, "  ", "  * test() {", "", Some(Indent), Option::None, 0);
    });
}

#[test]
fn issue_141816() {
    let config = LanguageConfiguration {
        on_enter_rules: javascript_on_enter_rules(),
        ..Default::default()
    };
    with_support(config, |s| {
        for before in [
            "const r = /{/;",
            "const r = /{[0-9]/;",
            "const r = /[a-zA-Z]{/;",
        ] {
            assert!(
                s.on_enter(AutoIndent::Advanced, "", before, "").is_none(),
                "{before}"
            );
        }
    });
}
