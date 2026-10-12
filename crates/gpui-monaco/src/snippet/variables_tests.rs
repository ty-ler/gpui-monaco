//! Port of Monaco's `snippetVariables.test.ts` (MIT, Copyright (c)
//! Microsoft Corporation). Test names are the TS test names in snake_case.
//!
//! Monaco composes one resolver per source; here one
//! [`SnippetVariableContext`] holds what they read. Paths are labelled as
//! the standalone label service labels them (plain paths), so the label
//! service mocks of "Path delimiters…" and "RELATIVE_FILEPATH" become the
//! context's path and workspace. "editor variables, selection" builds its
//! context from an editor, in `tests/snippet.rs`.

use chrono::{Local, TimeZone as _, Utc};

use super::parser::{Marker, SnippetParser, Variable};
use super::variables::{SnippetVariableContext, SnippetWorkspace};
use crate::languages::uri_path;

/// Monaco's setup: `this is line one…` at `file:///foo/files/text.txt`,
/// one cursor at (1, 1).
fn resolver() -> SnippetVariableContext {
    with_uri("file:///foo/files/text.txt", |cx| {
        cx.current_line = "this is line one".into();
        cx.current_word = Some("this".into());
        cx.cursor_count = 1;
    })
}

/// A `ModelBasedVariableResolver` for a model at `uri`.
fn with_uri(uri: &str, f: impl FnOnce(&mut SnippetVariableContext)) -> SnippetVariableContext {
    let mut cx = SnippetVariableContext {
        path: uri_path(uri),
        ..Default::default()
    };
    f(&mut cx);
    cx
}

/// Monaco `ClipboardBasedVariableResolver(() => text, selectionIdx,
/// selectionCount, spread)`.
fn clipboard(
    text: Option<&str>,
    index: usize,
    count: usize,
    spread: bool,
) -> SnippetVariableContext {
    SnippetVariableContext {
        clipboard: text.map(str::to_string),
        cursor_index: index,
        cursor_count: count,
        clipboard_spread: spread,
        ..Default::default()
    }
}

/// Monaco `assertVariableResolve`: `$name`'s value, `None` when it has none.
#[track_caller]
fn assert_variable_resolve(resolver: &SnippetVariableContext, name: &str, expected: Option<&str>) {
    let snippet = SnippetParser::new().parse(&format!("${name}"), false, false);
    let Marker::Variable(variable) = snippet.marker(snippet.children(snippet.root())[0]) else {
        panic!("`${name}` isn't a variable");
    };
    let value = variable.resolve(&|v: &Variable| resolver.resolve(&v.name, None));
    assert_eq!(value.as_deref(), expected, "{name}");
}

/// Monaco `assertVariableResolve2`: every variable resolves to `value`, or
/// its own name.
#[track_caller]
fn assert_variable_resolve2(input: &str, expected: &str, value: Option<&str>) {
    let mut snippet = SnippetParser::new().parse(input, false, false);
    snippet.resolve_variables(&|v: &Variable| {
        Some(value.map_or_else(|| v.name.clone(), str::to_string))
    });
    assert_eq!(snippet.to_string(), expected, "{input}");
}

#[test]
fn editor_variables_basics() {
    let resolver = resolver();
    assert_variable_resolve(&resolver, "TM_FILENAME", Some("text.txt"));
    assert_variable_resolve(&resolver, "something", None);
}

/// With the standalone label service, `TM_FILEPATH` is the URI's path (the
/// mock in Monaco's test swaps `/` for `|`).
#[test]
fn path_delimiters_in_code_snippet_variables_arent_specific_to_remote_os_76840() {
    let resolver = with_uri("foo:///foo/files/text.txt", |_| {});
    assert_variable_resolve(&resolver, "TM_FILEPATH", Some("/foo/files/text.txt"));
}

/// "editor variables, selection" with Monaco's `selectionIdx` 4; the rest
/// of it is in `tests/snippet.rs`.
#[test]
fn editor_variables_selection_cursor_index() {
    let resolver = SnippetVariableContext {
        cursor_index: 4,
        cursor_count: 5,
        ..Default::default()
    };
    assert_variable_resolve(&resolver, "CURSOR_INDEX", Some("4"));
    assert_variable_resolve(&resolver, "CURSOR_NUMBER", Some("5"));
    // An empty selection has no selected text.
    assert_variable_resolve(&resolver, "TM_SELECTED_TEXT", None);
    // Nor a word in whitespace.
    assert_variable_resolve(&resolver, "TM_CURRENT_WORD", None);
}

#[test]
fn textmate_snippet_resolve_variable() {
    let mut snippet = SnippetParser::new().parse("\"$TM_CURRENT_WORD\"", true, false);
    assert_eq!(snippet.to_string(), "\"\"");
    let resolver = resolver();
    snippet.resolve_variables(&|v: &Variable| resolver.resolve(&v.name, None));
    assert_eq!(snippet.to_string(), "\"this\"");
}

#[test]
fn textmate_snippet_resolve_variable_with_default() {
    let mut snippet = SnippetParser::new().parse("\"${TM_CURRENT_WORD:foo}\"", true, false);
    assert_eq!(snippet.to_string(), "\"foo\"");
    let resolver = resolver();
    snippet.resolve_variables(&|v: &Variable| resolver.resolve(&v.name, None));
    assert_eq!(snippet.to_string(), "\"this\"");
}

#[test]
fn more_useful_environment_variables_for_snippets_32737() {
    assert_variable_resolve(&resolver(), "TM_FILENAME_BASE", Some("text"));
    let resolver = with_uri("http://www.pb.o/abc/def/ghi", |_| {});
    assert_variable_resolve(&resolver, "TM_FILENAME_BASE", Some("ghi"));
    let resolver = with_uri("mem:.git", |_| {});
    assert_variable_resolve(&resolver, "TM_FILENAME_BASE", Some(".git"));
    let resolver = with_uri("mem:foo.", |_| {});
    assert_variable_resolve(&resolver, "TM_FILENAME_BASE", Some("foo"));
}

#[test]
fn variable_snippet_transform() {
    let mut snippet = SnippetParser::new().parse("name=${TM_FILENAME/(.*)\\..+$/$1/}", true, false);
    let resolver = resolver();
    snippet.resolve_variables(&|v: &Variable| resolver.resolve(&v.name, None));
    assert_eq!(snippet.to_string(), "name=text");

    assert_variable_resolve2("${ThisIsAVar/([A-Z]).*(Var)/$2/}", "Var", None);
    assert_variable_resolve2(
        "${ThisIsAVar/([A-Z]).*(Var)/$2-${1:/downcase}/}",
        "Var-t",
        None,
    );
    assert_variable_resolve2("${Foo/(.*)/${1:+Bar}/img}", "Bar", None);

    // https://github.com/microsoft/vscode/issues/33162
    assert_variable_resolve2(
        "export default class ${TM_FILENAME/(\\w+)\\.js/$1/g}",
        "export default class FooFile",
        Some("FooFile.js"),
    );

    // Global.
    assert_variable_resolve2("${foobarfoobar/(foo)/${1:+FAR}/g}", "FARbarFARbar", None);
    // First match.
    assert_variable_resolve2("${foobarfoobar/(foo)/${1:+FAR}/}", "FARbarfoobar", None);
    // No match, no else.
    assert_variable_resolve2("${foobarfoobar/(bazz)/${1:+FAR}/g}", "foobarfoobar", None);
    // Bad group reference.
    assert_variable_resolve2("${foobarfoobar/(foo)/${2:+FAR}/g}", "barbar", None);
}

#[test]
fn snippet_transforms_do_not_handle_regex_with_alternatives_or_optional_matches_36089() {
    const PASCAL: &str = "${TM_FILENAME/^(.)|(?:-(.))|(\\.js)/${1:/upcase}${2:/upcase}/g}";
    assert_variable_resolve2(PASCAL, "MyClass", Some("my-class.js"));
    // No hyphens.
    assert_variable_resolve2(PASCAL, "Myclass", Some("myclass.js"));
    // None matching suffix.
    assert_variable_resolve2(PASCAL, "Myclass.foo", Some("myclass.foo"));
    // More than one hyphen.
    assert_variable_resolve2(PASCAL, "ThisIsAFile", Some("this-is-a-file.js"));
    // KEBAB CASE.
    const KEBAB: &str =
        "${TM_FILENAME_BASE/([A-Z][a-z]+)([A-Z][a-z]+$)?/${1:/downcase}-${2:/downcase}/g}";
    assert_variable_resolve2(KEBAB, "capital-case", Some("CapitalCase"));
    assert_variable_resolve2(KEBAB, "capital-case-more", Some("CapitalCaseMore"));
}

#[test]
fn add_variable_to_insert_value_from_clipboard_to_a_snippet_40153() {
    assert_variable_resolve(&clipboard(None, 1, 0, true), "CLIPBOARD", None);
    assert_variable_resolve(&clipboard(Some(""), 1, 0, true), "CLIPBOARD", None);
    assert_variable_resolve(
        &clipboard(Some("foo"), 1, 0, true),
        "CLIPBOARD",
        Some("foo"),
    );
    assert_variable_resolve(&clipboard(Some("foo"), 1, 0, true), "foo", None);
    assert_variable_resolve(&clipboard(Some("foo"), 1, 0, true), "cLIPBOARD", None);
}

#[test]
fn add_variable_to_insert_value_from_clipboard_to_a_snippet_40153_2() {
    assert_variable_resolve(
        &clipboard(Some("line1"), 1, 2, true),
        "CLIPBOARD",
        Some("line1"),
    );
    assert_variable_resolve(
        &clipboard(Some("line1\nline2\nline3"), 1, 2, true),
        "CLIPBOARD",
        Some("line1\nline2\nline3"),
    );
    assert_variable_resolve(
        &clipboard(Some("line1\nline2"), 1, 2, true),
        "CLIPBOARD",
        Some("line2"),
    );
    assert_variable_resolve(
        &clipboard(Some("line1\nline2"), 0, 2, true),
        "CLIPBOARD",
        Some("line1"),
    );
    assert_variable_resolve(
        &clipboard(Some("line1\nline2"), 0, 2, false),
        "CLIPBOARD",
        Some("line1\nline2"),
    );
}

const TIME_VARIABLES: &[&str] = &[
    "CURRENT_YEAR",
    "CURRENT_YEAR_SHORT",
    "CURRENT_MONTH",
    "CURRENT_DATE",
    "CURRENT_HOUR",
    "CURRENT_MINUTE",
    "CURRENT_SECOND",
    "CURRENT_MILLISECOND",
    "CURRENT_DAY_NAME",
    "CURRENT_DAY_NAME_SHORT",
    "CURRENT_MONTH_NAME",
    "CURRENT_MONTH_NAME_SHORT",
    "CURRENT_SECONDS_UNIX",
    "CURRENT_MILLISECONDS_UNIX",
    "CURRENT_TIMEZONE_OFFSET",
    "CURRENT_TIMEZONE_NAME",
];

#[test]
fn add_time_variables_for_snippets_41631_43140() {
    let resolver = SnippetVariableContext::default();
    for name in TIME_VARIABLES {
        let snippet = SnippetParser::new().parse(&format!("${name}"), false, false);
        let Marker::Variable(variable) = snippet.marker(snippet.children(snippet.root())[0]) else {
            panic!("`${name}` isn't a variable");
        };
        assert!(
            variable
                .resolve(&|v: &Variable| resolver.resolve(&v.name, None))
                .is_some(),
            "{name} failed to resolve"
        );
    }
}

#[test]
fn time_based_snippet_variables_have_deterministic_millisecond_and_unix_values() {
    let now = Utc
        .with_ymd_and_hms(2024, 4, 15, 12, 34, 56)
        .unwrap()
        .checked_add_signed(chrono::TimeDelta::milliseconds(7))
        .unwrap();
    let expected = now.with_timezone(&Local);
    let resolver = SnippetVariableContext {
        now: Some(expected),
        ..Default::default()
    };
    use chrono::{Datelike as _, Timelike as _};
    let year = expected.year().to_string();
    assert_variable_resolve(&resolver, "CURRENT_YEAR", Some(&year));
    assert_variable_resolve(
        &resolver,
        "CURRENT_YEAR_SHORT",
        Some(&year[year.len() - 2..]),
    );
    let pad2 = |value: u32| format!("{value:02}");
    assert_variable_resolve(&resolver, "CURRENT_MONTH", Some(&pad2(expected.month())));
    assert_variable_resolve(&resolver, "CURRENT_DATE", Some(&pad2(expected.day())));
    assert_variable_resolve(&resolver, "CURRENT_HOUR", Some(&pad2(expected.hour())));
    assert_variable_resolve(&resolver, "CURRENT_MINUTE", Some(&pad2(expected.minute())));
    assert_variable_resolve(&resolver, "CURRENT_SECOND", Some(&pad2(expected.second())));
    assert_variable_resolve(&resolver, "CURRENT_MILLISECOND", Some("007"));
    let millis = now.timestamp_millis();
    assert_variable_resolve(
        &resolver,
        "CURRENT_SECONDS_UNIX",
        Some(&(millis / 1000).to_string()),
    );
    assert_variable_resolve(
        &resolver,
        "CURRENT_MILLISECONDS_UNIX",
        Some(&millis.to_string()),
    );
}

/// Monaco's resolver reads the clock once; the port's context holds the
/// time of the insertion.
#[test]
fn time_based_snippet_variables_resolve_to_the_same_values_even_as_time_progresses() {
    let snippet_text: String = TIME_VARIABLES
        .iter()
        .map(|name| format!("\n\t${name}"))
        .collect();
    let resolver = SnippetVariableContext {
        now: Some(Local::now()),
        ..Default::default()
    };
    let resolve = || {
        let mut snippet = SnippetParser::new().parse(&snippet_text, false, false);
        snippet.resolve_variables(&|v: &Variable| resolver.resolve(&v.name, None));
        snippet.to_string()
    };
    let first = resolve();
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert_eq!(
        first,
        resolve(),
        "Time-based snippet variables resolved differently"
    );
}

#[test]
fn creating_snippet_format_condition_doesnt_work_53617() {
    let mut snippet = SnippetParser::new().parse(
        "${TM_LINE_NUMBER/(10)/${1:?It is:It is not}/} line 10",
        true,
        false,
    );
    snippet.resolve_variables(&|_: &Variable| Some("10".to_string()));
    assert_eq!(snippet.to_string(), "It is line 10");
    snippet.resolve_variables(&|_: &Variable| Some("11".to_string()));
    assert_eq!(snippet.to_string(), "It is not line 10");
}

/// The host gives the workspace's name and folder
/// ([`SnippetVariableContext::workspace`]); Monaco derives them from the
/// workspace service.
#[test]
fn add_workspace_name_and_folder_variables_for_snippets_68261() {
    // Empty workspace.
    let resolver = SnippetVariableContext::default();
    assert_variable_resolve(&resolver, "WORKSPACE_NAME", None);
    assert_variable_resolve(&resolver, "WORKSPACE_FOLDER", None);
    // Single folder workspace without config.
    let resolver = SnippetVariableContext {
        workspace: Some(SnippetWorkspace {
            name: "folderName".into(),
            folder: "/folderName".into(),
        }),
        ..Default::default()
    };
    assert_variable_resolve(&resolver, "WORKSPACE_NAME", Some("folderName"));
    assert_variable_resolve(&resolver, "WORKSPACE_FOLDER", Some("/folderName"));
}

#[test]
fn add_relative_filepath_snippet_variable_114208() {
    // Empty workspace.
    let resolver = with_uri("file:///foo/files/text.txt", |_| {});
    assert_variable_resolve(&resolver, "RELATIVE_FILEPATH", Some("/foo/files/text.txt"));
    // Single folder workspace.
    let resolver = with_uri("file:///foo/files/text.txt", |cx| {
        cx.workspace = Some(SnippetWorkspace {
            name: "foo".into(),
            folder: "/foo".into(),
        });
    });
    assert_variable_resolve(&resolver, "RELATIVE_FILEPATH", Some("files/text.txt"));
}
