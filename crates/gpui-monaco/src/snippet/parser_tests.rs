//! Port of Monaco's `snippetParser.test.ts` (MIT, Copyright (c) Microsoft
//! Corporation). Test names are the TS test names in snake_case.

use std::cmp::Ordering;
use std::collections::HashSet;

use super::parser::*;

fn parse(value: &str) -> TextmateSnippet {
    SnippetParser::new().parse(value, false, false)
}

/// Stands in for `instanceof` checks.
fn kind(marker: &Marker) -> &'static str {
    match marker {
        Marker::Text(_) => "Text",
        Marker::Placeholder(_) => "Placeholder",
        Marker::Choice(_) => "Choice",
        Marker::Variable(_) => "Variable",
        Marker::TextmateSnippet => "TextmateSnippet",
    }
}

fn kinds(snippet: &TextmateSnippet, ids: &[MarkerId]) -> Vec<&'static str> {
    ids.iter().map(|&id| kind(snippet.marker(id))).collect()
}

fn top_kinds(snippet: &TextmateSnippet) -> Vec<&'static str> {
    kinds(snippet, snippet.children(snippet.root()))
}

fn assert_text(value: &str, expected: &str) {
    assert_eq!(
        SnippetParser::as_insert_text(value),
        expected,
        "input: {value}"
    );
}

fn assert_marker(value: &str, expected: &[&str]) {
    assert_eq!(top_kinds(&parse(value)), expected, "input: {value}");
}

fn assert_text_and_marker(value: &str, escaped: &str, expected: &[&str]) {
    assert_text(value, escaped);
    assert_marker(value, expected);
}

fn assert_escaped(value: &str, expected: &str) {
    assert_eq!(SnippetParser::escape(value), expected);
}

fn index(snippet: &TextmateSnippet, id: MarkerId) -> usize {
    snippet.placeholder(id).expect("a placeholder").index
}

fn transform(snippet: &TextmateSnippet, id: MarkerId) -> Option<&Transform> {
    match snippet.marker(id) {
        Marker::Placeholder(p) => p.transform.as_ref(),
        Marker::Variable(v) => v.transform.as_ref(),
        _ => None,
    }
}

fn format_string(part: &TransformPart) -> &FormatString {
    match part {
        TransformPart::FormatString(format) => format,
        TransformPart::Text(_) => panic!("expected a FormatString"),
    }
}

#[test]
fn scanner() {
    use TokenType::*;
    let mut scanner = Scanner::new();
    assert_eq!(scanner.next().kind, Eof);

    let cases: &[(&str, &[TokenType])] = &[
        ("abc", &[VariableName, Eof]),
        (
            "{{abc}}",
            &[
                CurlyOpen,
                CurlyOpen,
                VariableName,
                CurlyClose,
                CurlyClose,
                Eof,
            ],
        ),
        ("abc() ", &[VariableName, Format, Eof]),
        ("abc 123", &[VariableName, Format, Int, Eof]),
        ("$foo", &[Dollar, VariableName, Eof]),
        ("$foo_bar", &[Dollar, VariableName, Eof]),
        ("$foo-bar", &[Dollar, VariableName, Dash, VariableName, Eof]),
        (
            "${foo}",
            &[Dollar, CurlyOpen, VariableName, CurlyClose, Eof],
        ),
        (
            "${1223:foo}",
            &[Dollar, CurlyOpen, Int, Colon, VariableName, CurlyClose, Eof],
        ),
        (r"\${}", &[Backslash, Dollar, CurlyOpen, CurlyClose]),
        (
            "${foo/regex/format/option}",
            &[
                Dollar,
                CurlyOpen,
                VariableName,
                Forwardslash,
                VariableName,
                Forwardslash,
                VariableName,
                Forwardslash,
                VariableName,
                CurlyClose,
                Eof,
            ],
        ),
    ];
    for (text, expected) in cases {
        scanner.text(text);
        for kind in *expected {
            assert_eq!(scanner.next().kind, *kind, "scanning {text}");
        }
    }
}

#[test]
fn parser_escaped() {
    assert_escaped("foo$0", r"foo\$0");
    assert_escaped(r"foo\$0", r"foo\\\$0");
    assert_escaped("f$1oo$0", r"f\$1oo\$0");
    assert_escaped("${1:foo}$0", r"\${1:foo\}\$0");
    assert_escaped("$", r"\$");
}

#[test]
fn parser_text() {
    assert_text("$", "$");
    assert_text(r"\\$", r"\$");
    assert_text("{", "{");
    assert_text(r"\}", "}");
    assert_text(r"\abc", r"\abc");
    assert_text(r"foo${f:\}}bar", "foo}bar");
    assert_text(r"\{", r"\{");
    assert_text(r"I need \\\$", r"I need \$");
    assert_text(r"\", r"\");
    assert_text(r"\{{", r"\{{");
    assert_text("{{", "{{");
    assert_text("{{dd", "{{dd");
    assert_text("}}", "}}");
    assert_text("ff}}", "ff}}");

    assert_text("farboo", "farboo");
    assert_text("far{{}}boo", "far{{}}boo");
    assert_text("far{{123}}boo", "far{{123}}boo");
    assert_text(r"far\{{123}}boo", r"far\{{123}}boo");
    assert_text("far{{id:bern}}boo", "far{{id:bern}}boo");
    assert_text("far{{id:bern {{basel}}}}boo", "far{{id:bern {{basel}}}}boo");
    assert_text(
        "far{{id:bern {{id:basel}}}}boo",
        "far{{id:bern {{id:basel}}}}boo",
    );
    assert_text(
        "far{{id:bern {{id2:basel}}}}boo",
        "far{{id:bern {{id2:basel}}}}boo",
    );
}

#[test]
fn parser_tm_text() {
    assert_text_and_marker("foo${1:bar}}", "foobar}", &["Text", "Placeholder", "Text"]);
    assert_text_and_marker(
        "foo${1:bar}${2:foo}}",
        "foobarfoo}",
        &["Text", "Placeholder", "Placeholder", "Text"],
    );

    assert_text_and_marker(
        r"foo${1:bar\}${2:foo}}",
        "foobar}foo",
        &["Text", "Placeholder"],
    );

    let snippet = parse(r"foo${1:bar\}${2:foo}}");
    let placeholder = snippet.children(snippet.root())[1];
    let children = snippet.children(placeholder);

    assert_eq!(index(&snippet, placeholder), 1);
    assert!(matches!(snippet.marker(children[0]), Marker::Text(_)));
    assert_eq!(snippet.marker_to_string(children[0]), "bar}");
    assert!(matches!(
        snippet.marker(children[1]),
        Marker::Placeholder(_)
    ));
    assert_eq!(snippet.marker_to_string(children[1]), "foo");
}

#[test]
fn parser_placeholder() {
    assert_text_and_marker("farboo", "farboo", &["Text"]);
    assert_text_and_marker("far{{}}boo", "far{{}}boo", &["Text"]);
    assert_text_and_marker("far{{123}}boo", "far{{123}}boo", &["Text"]);
    assert_text_and_marker(r"far\{{123}}boo", r"far\{{123}}boo", &["Text"]);
}

#[test]
fn parser_literal_code() {
    assert_text_and_marker("far`123`boo", "far`123`boo", &["Text"]);
    assert_text_and_marker(r"far\`123\`boo", r"far\`123\`boo", &["Text"]);
}

#[test]
fn parser_variables_tabstop() {
    assert_text_and_marker("$far-boo", "-boo", &["Variable", "Text"]);
    assert_text_and_marker(r"\$far-boo", "$far-boo", &["Text"]);
    assert_text_and_marker("far$farboo", "far", &["Text", "Variable"]);
    assert_text_and_marker("far${farboo}", "far", &["Text", "Variable"]);
    assert_text_and_marker("$123", "", &["Placeholder"]);
    assert_text_and_marker("$farboo", "", &["Variable"]);
    assert_text_and_marker("$far12boo", "", &["Variable"]);
    assert_text_and_marker("000_${far}_000", "000__000", &["Text", "Variable", "Text"]);
    assert_text_and_marker(
        "FFF_${TM_SELECTED_TEXT}_FFF$0",
        "FFF__FFF",
        &["Text", "Variable", "Text", "Placeholder"],
    );
}

#[test]
fn parser_variables_placeholder_with_defaults() {
    assert_text_and_marker("${name:value}", "value", &["Variable"]);
    assert_text_and_marker("${1:value}", "value", &["Placeholder"]);
    assert_text_and_marker("${1:bar${2:foo}bar}", "barfoobar", &["Placeholder"]);

    assert_text_and_marker("${name:value", "${name:value", &["Text"]);
    assert_text_and_marker(
        "${1:bar${2:foobar}",
        "${1:barfoobar",
        &["Text", "Placeholder"],
    );
}

#[test]
fn parser_variable_transforms() {
    assert_text_and_marker("${foo///}", "", &["Variable"]);
    assert_text_and_marker("${foo/regex/format/gmi}", "", &["Variable"]);
    assert_text_and_marker("${foo/([A-Z][a-z])/format/}", "", &["Variable"]);

    // invalid regex
    assert_text_and_marker(
        "${foo/([A-Z][a-z])/format/GMI}",
        "${foo/([A-Z][a-z])/format/GMI}",
        &["Text"],
    );
    assert_text_and_marker(
        "${foo/([A-Z][a-z])/format/funky}",
        "${foo/([A-Z][a-z])/format/funky}",
        &["Text"],
    );
    assert_text_and_marker(
        "${foo/([A-Z][a-z]/format/}",
        "${foo/([A-Z][a-z]/format/}",
        &["Text"],
    );

    // tricky regex
    assert_text_and_marker(r"${foo/m\/atch/$1/i}", "", &["Variable"]);
    // The TS source writes `regex\/format`, which in a JS string is `regex/format`.
    assert_marker("${foo/regex/format/options}", &["Text"]);

    // incomplete
    assert_text_and_marker("${foo///", "${foo///", &["Text"]);
    assert_text_and_marker(
        "${foo/regex/format/options",
        "${foo/regex/format/options",
        &["Text"],
    );

    // format string
    assert_marker("${foo/.*/${0:fooo}/i}", &["Variable"]);
    assert_marker("${foo/.*/${1}/i}", &["Variable"]);
    assert_marker("${foo/.*/$1/i}", &["Variable"]);
    assert_marker("${foo/.*/This-$1-encloses/i}", &["Variable"]);
    assert_marker("${foo/.*/complex${1:else}/i}", &["Variable"]);
    assert_marker("${foo/.*/complex${1:-else}/i}", &["Variable"]);
    assert_marker("${foo/.*/complex${1:+if}/i}", &["Variable"]);
    assert_marker("${foo/.*/complex${1:?if:else}/i}", &["Variable"]);
    assert_marker("${foo/.*/complex${1:/upcase}/i}", &["Variable"]);
}

#[test]
fn parser_placeholder_transforms() {
    assert_text_and_marker("${1///}", "", &["Placeholder"]);
    assert_text_and_marker("${1/regex/format/gmi}", "", &["Placeholder"]);
    assert_text_and_marker("${1/([A-Z][a-z])/format/}", "", &["Placeholder"]);

    // tricky regex
    assert_text_and_marker(r"${1/m\/atch/$1/i}", "", &["Placeholder"]);
    // `regex\/format` in the TS source is `regex/format` at runtime.
    assert_marker("${1/regex/format/options}", &["Text"]);

    // incomplete
    assert_text_and_marker("${1///", "${1///", &["Text"]);
    assert_text_and_marker(
        "${1/regex/format/options",
        "${1/regex/format/options",
        &["Text"],
    );
}

#[test]
fn no_way_to_escape_forward_slash_in_snippet_regex_36715() {
    assert_marker(r"${TM_DIRECTORY/src\//$1/}", &["Variable"]);
}

#[test]
fn no_way_to_escape_forward_slash_in_snippet_format_section_37562() {
    assert_marker(r"${TM_SELECTED_TEXT/a/\/$1/g}", &["Variable"]);
    assert_marker(r"${TM_SELECTED_TEXT/a/in\/$1ner/g}", &["Variable"]);
    assert_marker(r"${TM_SELECTED_TEXT/a/end\//g}", &["Variable"]);
}

#[test]
fn parser_placeholder_with_choice() {
    assert_text_and_marker("${1|one,two,three|}", "one", &["Placeholder"]);
    assert_text_and_marker("${1|one|}", "one", &["Placeholder"]);
    assert_text_and_marker("${1|one1,two2|}", "one1", &["Placeholder"]);
    assert_text_and_marker(r"${1|one1\,two2|}", "one1,two2", &["Placeholder"]);
    assert_text_and_marker(r"${1|one1\|two2|}", "one1|two2", &["Placeholder"]);
    assert_text_and_marker(r"${1|one1\atwo2|}", r"one1\atwo2", &["Placeholder"]);
    assert_text_and_marker("${1|one,two,three,|}", "${1|one,two,three,|}", &["Text"]);
    assert_text_and_marker("${1|one,", "${1|one,", &["Text"]);

    let snippet = parse("${1|one,two,three|}");
    let mut visited = Vec::new();
    snippet.walk(|id| {
        visited.push(id);
        true
    });
    assert_eq!(kinds(&snippet, &visited), ["Placeholder", "Choice"]);
    let Marker::Choice(choice) = snippet.marker(visited[1]) else {
        unreachable!()
    };
    assert_eq!(choice.options.len(), 3);
}

#[test]
fn snippet_choices_unable_to_escape_comma_and_pipe_31521() {
    assert_text_and_marker(
        r"console.log(${1|not\, not, five, 5, 1   23|});",
        "console.log(not, not);",
        &["Text", "Placeholder", "Text"],
    );
}

#[test]
fn marker_to_textmate_string() {
    fn assert_textsnippet_string(input: &str, expected: &str) {
        assert_eq!(
            parse(input).to_textmate_string(),
            expected,
            "input: {input}"
        );
    }

    assert_textsnippet_string("$1", "$1");
    assert_textsnippet_string(r"\$1", r"\$1");
    assert_textsnippet_string(
        r"console.log(${1|not\, not, five, 5, 1   23|});",
        r"console.log(${1|not\, not, five, 5, 1   23|});",
    );
    assert_textsnippet_string(
        r"console.log(${1|not\, not, \| five, 5, 1   23|});",
        r"console.log(${1|not\, not, \| five, 5, 1   23|});",
    );
    assert_textsnippet_string(
        r"${1|cho\,ices,wi\|th,esc\\aping,chall\\\,enges|}",
        r"${1|cho\,ices,wi\|th,esc\\aping,chall\\\,enges|}",
    );
    assert_textsnippet_string("this is text", "this is text");
    assert_textsnippet_string(
        "this ${1:is ${2:nested with $var}}",
        "this ${1:is ${2:nested with ${var}}}",
    );
    assert_textsnippet_string(
        "this ${1:is ${2:nested with $var}}}",
        r"this ${1:is ${2:nested with ${var}}}\}",
    );
}

#[test]
fn marker_to_textmate_string_identity() {
    fn check_children(a: &TextmateSnippet, m1: MarkerId, b: &TextmateSnippet, m2: MarkerId) {
        assert_eq!(kind(a.marker(m1)), kind(b.marker(m2)));
        assert_eq!(a.children(m1).len(), b.children(m2).len());
        assert_eq!(a.marker_to_string(m1), b.marker_to_string(m2));
        for (&c1, &c2) in a.children(m1).iter().zip(b.children(m2)) {
            check_children(a, c1, b, c2);
        }
    }

    fn assert_ident(input: &str) {
        // full loop: (1) parse input, (2) generate textmate string, (3) parse,
        // (4) ensure both trees are equal
        let snippet = parse(input);
        let input2 = snippet.to_textmate_string();
        let snippet2 = parse(&input2);
        check_children(&snippet, snippet.root(), &snippet2, snippet2.root());
    }

    assert_ident("$1");
    assert_ident(r"\$1");
    assert_ident(r"console.log(${1|not\, not, five, 5, 1   23|});");
    assert_ident(r"console.log(${1|not\, not, \| five, 5, 1   23|});");
    assert_ident("this is text");
    assert_ident("this ${1:is ${2:nested with $var}}");
    assert_ident("this ${1:is ${2:nested with $var}}}");
    assert_ident("this ${1:is ${2:nested with $var}} and repeating $1");
}

#[test]
fn parser_choise_marker() {
    let snippet = parse("${1|one,two,three|}");
    let placeholders = snippet.placeholders();

    assert_eq!(placeholders.len(), 1);
    assert!(snippet.choice(placeholders[0]).is_some());
    let first_child = snippet.children(placeholders[0])[0];
    let Marker::Choice(choice) = snippet.marker(first_child) else {
        panic!("expected a Choice");
    };
    assert_eq!(choice.options.len(), 3);

    assert_text("${1|one,two,three|}", "one");
    assert_text(r"\${1|one,two,three|}", "${1|one,two,three|}");
    assert_text(r"${1\|one,two,three|}", r"${1\|one,two,three|}");
    assert_text("${1||}", "${1||}");
}

#[test]
fn backslash_character_escape_in_choice_tabstop_doesnt_work_58494() {
    let snippet = parse(r"${1|\,,},$,\|,\\|}");
    let placeholders = snippet.placeholders();
    assert_eq!(placeholders.len(), 1);
    assert!(snippet.choice(placeholders[0]).is_some());
}

#[test]
fn parser_only_textmate() {
    let mut p = SnippetParser::new();
    let mut assert = |value: &str, expected: &[&str]| {
        let snippet = p.parse(value, false, false);
        assert_eq!(top_kinds(&snippet), expected, "input: {value}");
    };
    assert("far{{}}boo", &["Text"]);
    assert("far{{123}}boo", &["Text"]);
    assert(r"far\{{123}}boo", &["Text"]);

    assert("far$0boo", &["Text", "Placeholder", "Text"]);
    assert("far${123}boo", &["Text", "Placeholder", "Text"]);
    assert(r"far\${123}boo", &["Text"]);
}

#[test]
fn parser_real_world() {
    let snippet = parse("console.warn(${1: $TM_SELECTED_TEXT })");
    let marker = snippet.children(snippet.root());

    assert_eq!(snippet.marker_to_string(marker[0]), "console.warn(");
    assert!(matches!(snippet.marker(marker[1]), Marker::Placeholder(_)));
    assert_eq!(snippet.marker_to_string(marker[2]), ")");

    let placeholder = marker[1];
    assert_eq!(index(&snippet, placeholder), 1);
    let children = snippet.children(placeholder);
    assert_eq!(kinds(&snippet, children), ["Text", "Variable", "Text"]);
    assert_eq!(snippet.marker_to_string(children[0]), " ");
    assert_eq!(snippet.marker_to_string(children[1]), "");
    assert_eq!(snippet.marker_to_string(children[2]), " ");

    let nested_variable = children[1];
    assert_eq!(
        snippet.variable(nested_variable).unwrap().name,
        "TM_SELECTED_TEXT"
    );
    assert_eq!(snippet.children(nested_variable).len(), 0);

    let snippet = parse("$TM_SELECTED_TEXT");
    assert_eq!(top_kinds(&snippet), ["Variable"]);
}

#[test]
fn parser_transform_example() {
    let snippet = parse("${1:name} : ${2:type}${3/\\s:=(.*)/${1:+ :=}${1}/};\n$0");
    let children = snippet.children(snippet.root());

    // ${1:name}
    assert!(matches!(
        snippet.marker(children[0]),
        Marker::Placeholder(_)
    ));
    assert_eq!(snippet.children(children[0]).len(), 1);
    assert_eq!(
        snippet.marker_to_string(snippet.children(children[0])[0]),
        "name"
    );
    assert!(transform(&snippet, children[0]).is_none());

    // :
    assert!(matches!(snippet.marker(children[1]), Marker::Text(_)));
    assert_eq!(snippet.marker_to_string(children[1]), " : ");

    // ${2:type}
    assert!(matches!(
        snippet.marker(children[2]),
        Marker::Placeholder(_)
    ));
    assert_eq!(snippet.children(children[2]).len(), 1);
    assert_eq!(
        snippet.marker_to_string(snippet.children(children[2])[0]),
        "type"
    );

    // ${3/\\s:=(.*)/${1:+ :=}${1}/}
    assert!(matches!(
        snippet.marker(children[3]),
        Marker::Placeholder(_)
    ));
    assert_eq!(snippet.children(children[3]).len(), 0);
    let transform = transform(&snippet, children[3]).expect("a transform");
    assert_eq!(transform.regexp.source, r"\s:=(.*)");
    assert_eq!(transform.regexp.flags, "");
    assert_eq!(transform.children.len(), 2);
    let first = format_string(&transform.children[0]);
    assert_eq!(first.index, 1);
    assert_eq!(first.if_value.as_deref(), Some(" :="));
    assert_eq!(format_string(&transform.children[1]).index, 1);
    assert!(matches!(snippet.marker(children[4]), Marker::Text(_)));
    assert_eq!(snippet.marker_to_string(children[4]), ";\n");
}

#[test]
fn parser_default_placeholder_values() {
    assert_marker(
        "errorContext: `${1:err}`, error: $1",
        &["Text", "Placeholder", "Text", "Placeholder"],
    );

    let snippet = parse("errorContext: `${1:err}`, error:$1");
    let (p1, p2) = (
        snippet.children(snippet.root())[1],
        snippet.children(snippet.root())[3],
    );

    assert_eq!(index(&snippet, p1), 1);
    assert_eq!(snippet.children(p1).len(), 1);
    assert_eq!(snippet.marker_to_string(snippet.children(p1)[0]), "err");

    assert_eq!(index(&snippet, p2), 1);
    assert_eq!(snippet.children(p2).len(), 1);
    assert_eq!(snippet.marker_to_string(snippet.children(p2)[0]), "err");
}

#[test]
fn parser_default_placeholder_values_and_one_transform() {
    assert_marker(
        "errorContext: `${1:err}`, error: ${1/err/ok/}",
        &["Text", "Placeholder", "Text", "Placeholder"],
    );

    let snippet = parse("errorContext: `${1:err}`, error:${1/err/ok/}");
    let (p3, p4) = (
        snippet.children(snippet.root())[1],
        snippet.children(snippet.root())[3],
    );

    assert_eq!(index(&snippet, p3), 1);
    assert_eq!(snippet.children(p3).len(), 1);
    assert_eq!(snippet.marker_to_string(snippet.children(p3)[0]), "err");
    assert!(transform(&snippet, p3).is_none());

    assert_eq!(index(&snippet, p4), 1);
    assert_eq!(snippet.children(p4).len(), 1);
    assert_eq!(snippet.marker_to_string(snippet.children(p4)[0]), "err");
    assert!(transform(&snippet, p4).is_some());
}

#[test]
fn repeated_snippet_placeholder_should_always_inherit_31040() {
    assert_text("${1:foo}-abc-$1", "foo-abc-foo");
    assert_text("${1:foo}-abc-${1}", "foo-abc-foo");
    assert_text("${1:foo}-abc-${1:bar}", "foo-abc-foo");
    assert_text("${1}-abc-${1:foo}", "foo-abc-foo");
}

#[test]
fn backspace_esapce_in_tm_only_16212() {
    assert_eq!(
        SnippetParser::as_insert_text(r"Foo \\${abc}bar"),
        r"Foo \bar"
    );
}

#[test]
fn colon_as_variable_placeholder_value_16717() {
    assert_eq!(
        SnippetParser::as_insert_text("${TM_SELECTED_TEXT:foo:bar}"),
        "foo:bar"
    );
    assert_eq!(SnippetParser::as_insert_text("${1:foo:bar}"), "foo:bar");
}

#[test]
fn incomplete_placeholder() {
    assert_text_and_marker("${1:}", "", &["Placeholder"]);
}

#[test]
fn marker_len() {
    fn assert_len(template: &str, lengths: &[usize]) {
        let snippet = SnippetParser::new().parse(template, true, false);
        let mut actual = Vec::new();
        snippet.walk(|id| {
            actual.push(snippet.marker(id).len());
            true
        });
        assert_eq!(actual, lengths, "template: {template}");
    }

    assert_len("text$0", &[4, 0]);
    assert_len("$1text$0", &[0, 4, 0]);
    assert_len("te$1xt$0", &[2, 0, 2, 0]);
    assert_len("errorContext: `${1:err}`, error: $0", &[15, 0, 3, 10, 0]);
    assert_len(
        "errorContext: `${1:err}`, error: $1$0",
        &[15, 0, 3, 10, 0, 3, 0],
    );
    assert_len("$TM_SELECTED_TEXT$0", &[0, 0]);
    assert_len("${TM_SELECTED_TEXT:def}$0", &[0, 3, 0]);
}

#[test]
fn parser_parent_node() {
    let snippet = SnippetParser::new().parse("This ${1:is ${2:nested}}$0", true, false);

    let placeholders = snippet.placeholders();
    assert_eq!(placeholders.len(), 3);
    let (first, second) = (placeholders[0], placeholders[1]);
    assert_eq!(index(&snippet, first), 1);
    assert_eq!(index(&snippet, second), 2);
    assert_eq!(snippet.parent(second), Some(first));
    assert_eq!(snippet.parent(first), Some(snippet.root()));

    let snippet = SnippetParser::new().parse("${VAR:default${1:value}}$0", true, false);
    let placeholders = snippet.placeholders();
    assert_eq!(placeholders.len(), 2);
    let first = placeholders[0];
    assert_eq!(index(&snippet, first), 1);

    let variable = snippet.children(snippet.root())[0];
    assert!(matches!(snippet.marker(variable), Marker::Variable(_)));
    assert_eq!(snippet.parent(first), Some(variable));
}

#[test]
fn textmate_snippet_enclosing_placeholders() {
    let snippet = SnippetParser::new().parse("This ${1:is ${2:nested}}$0", true, false);
    let placeholders = snippet.placeholders();
    let (first, second) = (placeholders[0], placeholders[1]);

    assert_eq!(snippet.enclosing_placeholders(first), []);
    assert_eq!(snippet.enclosing_placeholders(second), [first]);
}

#[test]
fn textmate_snippet_offset() {
    let snippet = SnippetParser::new().parse("te$1xt", true, false);
    let children = snippet.children(snippet.root());
    assert_eq!(snippet.offset(children[0]), Some(0));
    assert_eq!(snippet.offset(children[1]), Some(2));
    assert_eq!(snippet.offset(children[2]), Some(2));

    let mut snippet = SnippetParser::new().parse("${TM_SELECTED_TEXT:def}", true, false);
    let variable = snippet.children(snippet.root())[0];
    assert_eq!(snippet.offset(variable), Some(0));
    assert_eq!(snippet.offset(snippet.children(variable)[0]), Some(0));

    // forgein marker (Monaco returns -1)
    let foreign = snippet.create(Marker::Text(Text::new("foo")));
    assert_eq!(snippet.offset(foreign), None);
}

#[test]
fn textmate_snippet_placeholder() {
    let parse = |value| SnippetParser::new().parse(value, true, false);
    assert_eq!(parse("te$1xt$0").placeholders().len(), 2);
    assert_eq!(parse("te$1xt$1$0").placeholders().len(), 3);
    assert_eq!(parse("te$1xt$2$0").placeholders().len(), 3);
    assert_eq!(parse("${1:bar${2:foo}bar}$0").placeholders().len(), 3);
}

#[test]
fn textmate_snippet_replace_1_2() {
    let mut snippet = SnippetParser::new().parse("aaa${1:bbb${2:ccc}}$0", true, false);

    assert_eq!(snippet.placeholders().len(), 3);
    let second = snippet.placeholders()[1];
    assert_eq!(index(&snippet, second), 2);

    let enclosing = snippet.enclosing_placeholders(second);
    assert_eq!(enclosing.len(), 1);
    assert_eq!(index(&snippet, enclosing[0]), 1);

    let nested = SnippetParser::new().parse("ddd$1eee$0", true, false);
    snippet.replace_with_snippet(second, &nested);

    assert_eq!(snippet.to_string(), "aaabbbdddeee");
    let placeholders = snippet.placeholders();
    assert_eq!(placeholders.len(), 4);
    assert_eq!(index(&snippet, placeholders[0]), 1);
    assert_eq!(index(&snippet, placeholders[1]), 1);
    assert_eq!(index(&snippet, placeholders[2]), 0);
    assert_eq!(index(&snippet, placeholders[3]), 0);

    let new_enclosing = snippet.enclosing_placeholders(placeholders[1]);
    assert_eq!(new_enclosing[0], placeholders[0]);
    assert_eq!(new_enclosing.len(), 1);
    assert_eq!(index(&snippet, new_enclosing[0]), 1);
}

#[test]
fn textmate_snippet_replace_2_2() {
    let mut snippet = SnippetParser::new().parse("aaa${1:bbb${2:ccc}}$0", true, false);

    assert_eq!(snippet.placeholders().len(), 3);
    let second = snippet.placeholders()[1];
    assert_eq!(index(&snippet, second), 2);

    let nested = SnippetParser::new().parse("dddeee$0", true, false);
    snippet.replace_with_snippet(second, &nested);

    assert_eq!(snippet.to_string(), "aaabbbdddeee");
    assert_eq!(snippet.placeholders().len(), 3);
}

#[test]
fn snippet_order_for_placeholders_28185() {
    let ten = Placeholder::new(10);
    let two = Placeholder::new(2);
    assert_eq!(Placeholder::compare_by_index(&ten, &two), Ordering::Greater);
}

#[test]
fn maximum_call_stack_size_exceeded_28983() {
    parse("${1:${foo:${1}}}");
}

#[test]
fn snippet_can_freeze_the_editor_30407() {
    let assert_unique = |value: &str| {
        let snippet = parse(value);
        let mut seen = HashSet::new();
        snippet.walk(|id| {
            assert!(seen.insert(id), "visited {id:?} twice in {value}");
            true
        });
    };
    assert_unique(
        "class ${1:${TM_FILENAME/(?:\\A|_)([A-Za-z0-9]+)(?:\\.rb)?/(?2::\\u$1)/g}} < ${2:Application}Controller\n  $3\nend",
    );
    assert_unique("${1:${FOO:abc$1def}}");
}

#[test]
fn snippets_make_parser_ignore_0_choice_31599() {
    assert_text_and_marker("${0|foo,bar|}", "${0|foo,bar|}", &["Text"]);
    assert_text_and_marker("${1|foo,bar|}", "foo", &["Placeholder"]);
}

#[test]
fn transform_format_string_resolve() {
    let shorthand =
        |name: &str, value: &str| FormatString::new(1, Some(name), None, None).resolve(Some(value));

    // shorthand functions
    assert_eq!(shorthand("upcase", "foo"), "FOO");
    assert_eq!(shorthand("downcase", "FOO"), "foo");
    assert_eq!(shorthand("capitalize", "bar"), "Bar");
    assert_eq!(shorthand("capitalize", "bar no repeat"), "Bar no repeat");
    assert_eq!(shorthand("pascalcase", "bar-foo"), "BarFoo");
    assert_eq!(shorthand("pascalcase", "bar-42-foo"), "Bar42Foo");
    assert_eq!(
        shorthand("pascalcase", "snake_AndPascalCase"),
        "SnakeAndPascalCase"
    );
    assert_eq!(
        shorthand("pascalcase", "kebab-AndPascalCase"),
        "KebabAndPascalCase"
    );
    assert_eq!(shorthand("pascalcase", "_justPascalCase"), "JustPascalCase");
    assert_eq!(shorthand("camelcase", "bar-foo"), "barFoo");
    assert_eq!(shorthand("camelcase", "bar-42-foo"), "bar42Foo");
    assert_eq!(
        shorthand("camelcase", "snake_AndCamelCase"),
        "snakeAndCamelCase"
    );
    assert_eq!(
        shorthand("camelcase", "kebab-AndCamelCase"),
        "kebabAndCamelCase"
    );
    assert_eq!(shorthand("camelcase", "_JustCamelCase"), "justCamelCase");
    assert_eq!(shorthand("kebabcase", "barFoo"), "bar-foo");
    assert_eq!(shorthand("kebabcase", "BarFoo"), "bar-foo");
    assert_eq!(shorthand("kebabcase", "ABarFoo"), "a-bar-foo");
    assert_eq!(shorthand("kebabcase", "bar42Foo"), "bar42-foo");
    assert_eq!(
        shorthand("kebabcase", "snake_AndPascalCase"),
        "snake-and-pascal-case"
    );
    assert_eq!(
        shorthand("kebabcase", "kebab-AndCamelCase"),
        "kebab-and-camel-case"
    );
    assert_eq!(
        shorthand("kebabcase", "_justPascalCase"),
        "just-pascal-case"
    );
    assert_eq!(shorthand("kebabcase", "__UPCASE__"), "upcase");
    assert_eq!(shorthand("kebabcase", "__BAR_FOO__"), "bar-foo");
    assert_eq!(shorthand("snakecase", "bar-foo"), "bar_foo");
    assert_eq!(shorthand("snakecase", "bar-42-foo"), "bar_42_foo");
    assert_eq!(
        shorthand("snakecase", "snake_AndPascalCase"),
        "snake_and_pascal_case"
    );
    assert_eq!(
        shorthand("snakecase", "kebab-AndPascalCase"),
        "kebab_and_pascal_case"
    );
    assert_eq!(
        shorthand("snakecase", "_justPascalCase"),
        "_just_pascal_case"
    );
    assert_eq!(shorthand("notKnown", "input"), "input");

    // if
    let format = FormatString::new(1, None, Some("foo"), None);
    assert_eq!(format.resolve(None), "");
    assert_eq!(format.resolve(Some("")), "");
    assert_eq!(format.resolve(Some("bar")), "foo");

    // else
    let format = FormatString::new(1, None, None, Some("foo"));
    assert_eq!(format.resolve(None), "foo");
    assert_eq!(format.resolve(Some("")), "foo");
    assert_eq!(format.resolve(Some("bar")), "bar");

    // if-else
    let format = FormatString::new(1, None, Some("bar"), Some("foo"));
    assert_eq!(format.resolve(None), "foo");
    assert_eq!(format.resolve(Some("")), "foo");
    assert_eq!(format.resolve(Some("baz")), "bar");
}

#[test]
fn unicode_variable_transformations() {
    let resolver = |variable: &Variable| -> Option<String> {
        let value = match variable.name.as_str() {
            "RUSSIAN" => "одинДва",
            "GREEK" => "έναςΔύο",
            "TURKISH" => "istanbulLı",
            "JAPANESE" => "こんにちは",
            _ => return None,
        };
        Some(value.to_string())
    };

    let assert_transform = |transform_name: &str, var_name: &str, expected: &str| {
        let mut snippet = parse(&format!("${{{var_name}/(.*)/${{1:/{transform_name}}}/}}"));
        let variable = snippet.children(snippet.root())[0];
        snippet.resolve_variable(variable, &resolver);
        let resolved = snippet.marker_to_string(variable);
        assert_eq!(resolved, expected, "{transform_name} failed for {var_name}");
    };

    assert_transform("kebabcase", "RUSSIAN", "один-два");
    assert_transform("kebabcase", "GREEK", "ένας-δύο");
    assert_transform("snakecase", "RUSSIAN", "один_два");
    assert_transform("snakecase", "GREEK", "ένας_δύο");
    assert_transform("camelcase", "RUSSIAN", "одинДва");
    assert_transform("camelcase", "GREEK", "έναςΔύο");
    assert_transform("pascalcase", "RUSSIAN", "ОдинДва");
    assert_transform("pascalcase", "GREEK", "ΈναςΔύο");
    assert_transform("upcase", "RUSSIAN", "ОДИНДВА");
    assert_transform("downcase", "RUSSIAN", "одиндва");
    assert_transform("kebabcase", "TURKISH", "istanbul-lı");
    assert_transform("pascalcase", "TURKISH", "IstanbulLı");
    assert_transform("upcase", "JAPANESE", "こんにちは");
    assert_transform("kebabcase", "JAPANESE", "こんにちは");
}

#[test]
fn snippet_variable_transformation_doesnt_work_if_regex_is_complicated_and_snippet_body_contains_55627()
 {
    let snippet = parse(r#"const fileName = "${TM_FILENAME/(.*)\..+$/$1/}""#);
    assert_eq!(
        snippet.to_textmate_string(),
        r#"const fileName = "${TM_FILENAME/(.*)\..+$/${1}/}""#
    );
}

#[test]
fn bug_html_attribute_suggestions_snippet_session_does_not_have_end_position_set_33147() {
    let snippet = SnippetParser::new().parse(r#"src="$1""#, true, false);
    let placeholders = snippet.placeholders();

    assert_eq!(placeholders.len(), 2);
    assert_eq!(index(&snippet, placeholders[0]), 1);
    assert_eq!(index(&snippet, placeholders[1]), 0);
}

#[test]
fn snippet_optional_transforms_are_not_applied_correctly_when_reusing_the_same_variable_37702() {
    let mut transform = Transform::new();
    transform.append_child(TransformPart::FormatString(FormatString::new(
        1,
        Some("upcase"),
        None,
        None,
    )));
    transform.append_child(TransformPart::FormatString(FormatString::new(
        2,
        Some("upcase"),
        None,
        None,
    )));
    transform.regexp = syntax::Pattern::new("^(.)|-(.)", "g");

    assert_eq!(transform.resolve("my-file-name"), "MyFileName");

    let clone = transform.clone();
    assert_eq!(clone.resolve("my-file-name"), "MyFileName");
}

#[test]
fn problem_with_snippets_regex_40570() {
    assert_marker(r"${TM_DIRECTORY/.*src[\/](.*)/$1/}", &["Variable"]);
}

#[test]
fn variable_transformation_doesnt_work_if_undefined_variables_are_used_in_the_same_snippet_51769() {
    let mut transform = Transform::new();
    transform.append_child(TransformPart::Text(Text::new("bar")));
    transform.regexp = syntax::Pattern::new("foo", "gi");
    assert_eq!(transform.to_textmate_string(), "/foo/bar/ig");
}

#[test]
fn transform_serialization_joins_children_without_comma() {
    let mut transform = Transform::new();
    transform.append_child(TransformPart::FormatString(FormatString::new(
        1,
        Some("upcase"),
        None,
        None,
    )));
    transform.append_child(TransformPart::Text(Text::new("_")));
    transform.regexp = syntax::Pattern::new("foo", "g");
    let serialized = transform.to_textmate_string();
    assert_eq!(serialized, "/foo/${1:/upcase}_/g");

    let source = format!("${{TM_FILENAME{serialized}}}");
    assert_eq!(parse(&source).to_textmate_string(), source);
}

#[test]
fn snippet_parser_freeze_53144() {
    assert_marker("${1/(void$)|(.+)/${1:?-\treturn nil;}/}", &["Placeholder"]);
}

#[test]
fn snippets_variable_not_resolved_in_json_proposal_52931() {
    assert_text_and_marker(
        "FOO${1:/bin/bash}",
        "FOO/bin/bash",
        &["Text", "Placeholder"],
    );
}

#[test]
fn mirroring_sequence_of_nested_placeholders_not_selected_properly_on_backjumping_58736() {
    let snippet = parse("${3:nest1 ${1:nest2 ${2:nest3}}} $3");
    assert_eq!(top_kinds(&snippet), ["Placeholder", "Text", "Placeholder"]);

    fn assert_parent(snippet: &TextmateSnippet, marker: MarkerId) {
        for &child in snippet.children(marker) {
            assert_parent(snippet, child);
        }
        if snippet.placeholder(marker).is_none() {
            return;
        }
        let mut found = false;
        let mut m = Some(marker);
        while let Some(id) = m
            && !found
        {
            if snippet.parent(id) == Some(snippet.root()) {
                found = true;
            }
            m = snippet.parent(id);
        }
        assert!(found);
    }
    let clone = snippet.children(snippet.root())[2];
    assert_parent(&snippet, clone);
}

#[test]
fn backspace_cant_be_escaped_in_snippet_variable_transforms_65412() {
    assert_marker(
        r"namespace ${TM_DIRECTORY/[\/]/\\/g};",
        &["Text", "Variable", "Text"],
    );
}

#[test]
fn snippet_cannot_escape_closing_bracket_inside_conditional_insertion_variable_replacement_78883() {
    let snippet = parse(r"${TM_DIRECTORY/(.+)/${1:+import { hello \} from world}/}");
    assert_eq!(top_kinds(&snippet), ["Variable"]);
    let variable = snippet.children(snippet.root())[0];
    let transform = transform(&snippet, variable).expect("a transform");
    assert_eq!(transform.children.len(), 1);
    let format = format_string(&transform.children[0]);
    assert_eq!(
        format.if_value.as_deref(),
        Some("import { hello } from world")
    );
    assert_eq!(format.else_value, None);
}

#[test]
fn snippet_escape_backslashes_inside_conditional_insertion_variable_replacement_80394() {
    let snippet = parse(r"${CURRENT_YEAR/(.+)/${1:+\\}/}");
    assert_eq!(top_kinds(&snippet), ["Variable"]);
    let variable = snippet.children(snippet.root())[0];
    let transform = transform(&snippet, variable).expect("a transform");
    assert_eq!(transform.children.len(), 1);
    let format = format_string(&transform.children[0]);
    assert_eq!(format.if_value.as_deref(), Some(r"\"));
    assert_eq!(format.else_value, None);
}

#[test]
fn snippet_placeholder_empty_right_after_expansion_152553() {
    assert_eq!(
        parse("${1:prog}: ${2:$1.cc} - $2").to_string(),
        "prog: prog.cc - prog.cc"
    );

    assert_eq!(
        parse("${1:prog}: ${3:${2:$1.cc}.33} - $2 $3").to_string(),
        "prog: prog.cc.33 - prog.cc prog.cc.33"
    );

    // cyclic references of placeholders
    assert_eq!(
        parse("${1:$2.one} <> ${2:$1.two}").to_string(),
        ".two.one.two.one <> .one.two.one.two"
    );
}

#[test]
fn snippet_choices_are_incorrectly_escaped_applied_180132() {
    assert_text_and_marker(
        r"${1|aaa$aaa|}bbb\$bbb",
        "aaa$aaabbb$bbb",
        &["Placeholder", "Text"],
    );
}

// Extra coverage for API Monaco's suite exercises only indirectly.

#[test]
fn guess_needs_clipboard() {
    assert!(SnippetParser::guess_needs_clipboard("a $CLIPBOARD b"));
    assert!(SnippetParser::guess_needs_clipboard("${CLIPBOARD:x}"));
    assert!(!SnippetParser::guess_needs_clipboard("CLIPBOARD"));
}

#[test]
fn resolve_variables_and_utf16_offsets() {
    let mut snippet = SnippetParser::new().parse("😀${FOO}$1", false, false);
    snippet.resolve_variables(&|v: &Variable| (v.name == "FOO").then(|| "é".to_string()));
    assert_eq!(snippet.to_string(), "😀é");
    let placeholder = snippet.placeholders()[0];
    // 😀 is two UTF-16 code units, é one.
    assert_eq!(snippet.offset(placeholder), Some(3));
    let variable = snippet.children(snippet.root())[1];
    assert_eq!(snippet.full_len(variable), 1);
    assert_eq!(snippet.placeholder_info().last, Some(placeholder));
}

#[test]
fn transform_else_branch_runs_without_match() {
    let snippet = parse("${FOO/(x)/${1:-none}/}");
    let variable = snippet.children(snippet.root())[0];
    let transform = transform(&snippet, variable).unwrap();
    assert_eq!(transform.resolve("abc"), "none");
    assert_eq!(transform.resolve("axc"), "axc");
}
