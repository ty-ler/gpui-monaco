//! Monaco's `standalone/test/browser/monarch.test.ts` (MIT), plus tests of
//! the grammar features it doesn't cover and of [`MonarchHighlighter`].

use std::sync::Arc;

use serde_json::{Value, json};
use text::Buffer;

use super::*;
use crate::highlighter::Highlighter;
use crate::language::{HighlightId, Language, LanguageRegistry};

/// Monaco's `Token(offset, type, language)`, with UTF-16 offsets.
fn t(offset: usize, token_type: &str, language: &str) -> (usize, String, String) {
    (offset, token_type.to_string(), language.to_string())
}

/// Monaco's test `getTokens`: every line with `hasEOL`, from the initial
/// state.
fn get_tokens_with(
    tokenizer: &MonarchTokenizer,
    lines: &[&str],
    embedded: &dyn EmbeddedLanguages,
) -> Vec<Vec<(usize, String, String)>> {
    let mut state = tokenizer.initial_state();
    lines
        .iter()
        .map(|line| {
            let result = tokenizer
                .try_tokenize(line, true, &state, embedded)
                .unwrap_or_else(|e| panic!("{line:?}: {e}"));
            state = result.end_state;
            result
                .tokens
                .into_iter()
                .map(|token| {
                    (
                        line[..token.offset].encode_utf16().count(),
                        token.token_type,
                        token.language.to_string(),
                    )
                })
                .collect()
        })
        .collect()
}

fn get_tokens(tokenizer: &MonarchTokenizer, lines: &[&str]) -> Vec<Vec<(usize, String, String)>> {
    get_tokens_with(tokenizer, lines, &())
}

fn tokenizer(language_id: &str, grammar: Value) -> MonarchTokenizer {
    MonarchTokenizer::new(language_id, &grammar).unwrap_or_else(|e| panic!("{e}"))
}

/// A registry with `sql` tokenized by `[/./, 'token']`.
struct WithSql {
    registry: Arc<LanguageRegistry>,
}

impl WithSql {
    fn new() -> Self {
        let registry = Arc::new(LanguageRegistry::new());
        registry.register(
            Language::new("sql", "SQL")
                .with_monarch_grammar(&json!({ "tokenizer": { "root": [[".", "token"]] } }))
                .unwrap(),
        );
        Self { registry }
    }
}

impl EmbeddedLanguages for WithSql {
    fn language_id(&self, name: &str) -> Option<Arc<str>> {
        self.registry.language_for_name(name).map(|l| l.id.clone())
    }

    fn tokenizer(&self, language_id: &str) -> Option<Arc<MonarchTokenizer>> {
        let language = self.registry.language_for_id(language_id)?;
        MonarchHighlighter::tokenizer_of(&language, &self.registry)
    }
}

const SQL_QUERY_START: &str = "(SELECT|INSERT|UPDATE|DELETE|CREATE|REPLACE|ALTER|WITH)";

const SQL_LINES: [&str; 6] = [
    "mysql_query(\"\"\"SELECT * FROM table_name WHERE ds = '<DATEID>'\"\"\")",
    "mysql_query(\"\"\"",
    "SELECT *",
    "FROM table_name",
    "WHERE ds = '<DATEID>'",
    "\"\"\")",
];

fn sql_expected() -> Vec<Vec<(usize, String, String)>> {
    vec![
        vec![
            t(0, "source.test1", "test1"),
            t(12, "string.quote.test1", "test1"),
            t(15, "token.sql", "sql"),
            t(61, "string.quote.test1", "test1"),
            t(64, "source.test1", "test1"),
        ],
        vec![
            t(0, "source.test1", "test1"),
            t(12, "string.quote.test1", "test1"),
        ],
        vec![t(0, "token.sql", "sql")],
        vec![t(0, "token.sql", "sql")],
        vec![t(0, "token.sql", "sql")],
        vec![
            t(0, "string.quote.test1", "test1"),
            t(3, "source.test1", "test1"),
        ],
    ]
}

fn sql_grammar(end_string_with_sql: Value) -> Value {
    json!({
        "tokenizer": {
            "root": [
                [format!("(\"\"\"){SQL_QUERY_START}"), [
                    { "token": "string.quote" },
                    { "token": "@rematch", "next": "@endStringWithSQL", "nextEmbedded": "sql" },
                ]],
                ["(\"\"\")$", [{ "token": "string.quote", "next": "@maybeStringIsSQL" }]],
            ],
            "maybeStringIsSQL": [
                ["(.*)", {
                    "cases": {
                        format!("{SQL_QUERY_START}\\b.*"): {
                            "token": "@rematch", "next": "@endStringWithSQL", "nextEmbedded": "sql",
                        },
                        "@default": { "token": "@rematch", "switchTo": "@endDblDocString" },
                    }
                }],
            ],
            "endDblDocString": [
                ["[^']+", "string"],
                ["\\\\'", "string"],
                ["'''", "string", "@popall"],
                ["'", "string"],
            ],
            "endStringWithSQL": end_string_with_sql,
        }
    })
}

#[test]
fn rematch_and_next_embedded_can_be_used_together() {
    let sql = WithSql::new();
    let tokenizer = tokenizer(
        "test1",
        sql_grammar(json!([["\"\"\"", {
            "token": "string.quote", "next": "@popall", "nextEmbedded": "@pop",
        }]])),
    );
    assert_eq!(
        get_tokens_with(&tokenizer, &SQL_LINES, &sql),
        sql_expected()
    );
}

#[test]
fn next_embedded_pop_in_cases_statement() {
    let sql = WithSql::new();
    let tokenizer = tokenizer(
        "test1",
        sql_grammar(json!([["\"\"\"", {
            "cases": {
                "\"\"\"": {
                    "cases": {
                        "": { "token": "string.quote", "next": "@popall", "nextEmbedded": "@pop" }
                    }
                },
                "@default": "",
            }
        }]])),
    );
    assert_eq!(
        get_tokens_with(&tokenizer, &SQL_LINES, &sql),
        sql_expected()
    );
}

#[test]
fn empty_line_handling() {
    // microsoft/monaco-editor#1235
    let tokenizer = tokenizer(
        "test",
        json!({
            "tokenizer": {
                "root": [{ "include": "@comments" }],
                "comments": [
                    ["\\/\\/$", "comment"],
                    ["\\/\\/", "comment", "@comment_cpp"],
                ],
                "comment_cpp": [
                    ["(?:[^\\\\]|(?:\\\\.))+$", "comment", "@pop"],
                    [".+$", "comment"],
                    ["$", "comment", "@pop"],
                ],
            }
        }),
    );
    let lines = [
        "// This comment \\",
        "   continues on the following line",
        "",
        "// This comment does NOT continue \\\\",
        "   because the escape char was itself escaped",
        "",
        "// This comment DOES continue because \\\\\\",
        "   the 1st '\\' escapes the 2nd; the 3rd escapes EOL",
        "",
        "// This comment continues to the following line \\",
        "",
        "But the line was empty. This line should not be commented.",
    ];
    assert_eq!(
        get_tokens(&tokenizer, &lines),
        vec![
            vec![t(0, "comment.test", "test")],
            vec![t(0, "comment.test", "test")],
            vec![],
            vec![t(0, "comment.test", "test")],
            vec![t(0, "source.test", "test")],
            vec![],
            vec![t(0, "comment.test", "test")],
            vec![t(0, "comment.test", "test")],
            vec![],
            vec![t(0, "comment.test", "test")],
            vec![],
            vec![t(0, "source.test", "test")],
        ]
    );
}

#[test]
fn exit_a_state_at_end_of_line() {
    // microsoft/monaco-editor#2265
    let tokenizer = tokenizer(
        "test",
        json!({
            "includeLF": true,
            "tokenizer": {
                "root": [
                    ["^\\*", "", "@inner"],
                    ["\\:\\*", "", "@inner"],
                    ["[^*:]+", "string"],
                    ["[*:]", "string"],
                ],
                "inner": [
                    ["\\n", "", "@pop"],
                    ["\\d+", "number"],
                    ["[^\\d]+", ""],
                ],
            }
        }),
    );
    let lines = ["PRINT 10 * 20", "*FX200, 3", "PRINT 2*3:*FX200, 3"];
    assert_eq!(
        get_tokens(&tokenizer, &lines),
        vec![
            vec![t(0, "string.test", "test")],
            vec![
                t(0, "", "test"),
                t(3, "number.test", "test"),
                t(6, "", "test"),
                t(8, "number.test", "test"),
            ],
            vec![
                t(0, "string.test", "test"),
                t(9, "", "test"),
                t(13, "number.test", "test"),
                t(16, "", "test"),
                t(18, "number.test", "test"),
            ],
        ]
    );
}

#[test]
fn attribute_replacement_stops_after_five_levels_and_at_double_at() {
    // issue #115662: monarchCompile function need an extra option which can
    // control replacement
    let tokenizer1 = tokenizer(
        "test",
        json!({
            "ignoreCase": false,
            "uselessReplaceKey1": "@uselessReplaceKey2",
            "uselessReplaceKey2": "@uselessReplaceKey3",
            "uselessReplaceKey3": "@uselessReplaceKey4",
            "uselessReplaceKey4": "@uselessReplaceKey5",
            "uselessReplaceKey5": "@ham",
            "tokenizer": {
                "root": [{ "regex": "^@uselessReplaceKey1$", "action": { "token": "ham" } }],
            },
        }),
    );
    let tokenizer2 = tokenizer(
        "test",
        json!({
            "ignoreCase": false,
            "tokenizer": { "root": [{ "regex": "@@ham", "action": { "token": "ham" } }] },
        }),
    );
    assert_eq!(
        get_tokens(&tokenizer1, &["@ham"]),
        vec![vec![t(0, "ham.test", "test")]]
    );
    assert_eq!(
        get_tokens(&tokenizer2, &["@ham"]),
        vec![vec![t(0, "ham.test", "test")]]
    );
}

#[test]
fn allow_to_target_double_at() {
    // microsoft/monaco-editor#2424
    let tokenizer = tokenizer(
        "test",
        json!({
            "ignoreCase": false,
            "tokenizer": { "root": [{ "regex": "@@@@", "action": { "token": "ham" } }] },
        }),
    );
    assert_eq!(
        get_tokens(&tokenizer, &["@@"]),
        vec![vec![t(0, "ham.test", "test")]]
    );
}

#[test]
fn max_tokenization_line_length_is_checked_before_tokenizing() {
    // microsoft/monaco-editor#3025
    let tokenizer = tokenizer(
        "test",
        json!({ "tokenizer": { "root": [{ "regex": "ham", "action": { "token": "ham" } }] } }),
    )
    .with_max_tokenization_line_length(4);
    assert_eq!(
        get_tokens(&tokenizer, &["ham", "hamham"]),
        vec![vec![t(0, "ham.test", "test")], vec![t(0, "", "test")]]
    );
}

fn raw_string_grammar() -> Value {
    json!({
        "ignoreCase": false,
        "encoding": "u|u8|U|L",
        "tokenizer": {
            "root": [
                // C++ 11 Raw String
                ["@encoding?R\\\"(?:([^ ()\\\\\\t]*))\\(", { "token": "string.raw.begin", "next": "@raw.$1" }],
            ],
            "raw": [
                [".*\\)$S2\\\"", "string.raw", "@pop"],
                [".*", "string.raw"],
            ],
        },
    })
}

#[test]
fn allow_state_access_within_rules() {
    // microsoft/monaco-editor#3128
    let tokenizer = tokenizer("test", raw_string_grammar());
    let lines = [
        "int main(){",
        "",
        "\tauto s = R\"\"\"\"(",
        "\tHello World",
        "\t)\"\"\"\";",
        "",
        "\tstd::cout << \"hello\";",
        "",
        "}",
    ];
    assert_eq!(
        get_tokens(&tokenizer, &lines),
        vec![
            vec![t(0, "source.test", "test")],
            vec![],
            vec![
                t(0, "source.test", "test"),
                t(10, "string.raw.begin.test", "test"),
            ],
            vec![t(0, "string.raw.test", "test")],
            vec![t(0, "string.raw.test", "test"), t(6, "source.test", "test")],
            vec![],
            vec![t(0, "source.test", "test")],
            vec![],
            vec![t(0, "source.test", "test")],
        ]
    );
}

#[test]
fn raw_strings_in_cpp_can_break_monarch() {
    // microsoft/monaco-editor#4775
    let tokenizer = tokenizer("test", raw_string_grammar());
    assert_eq!(
        get_tokens(&tokenizer, &["R\"[())\""]),
        vec![vec![
            t(0, "string.raw.begin.test", "test"),
            t(4, "string.raw.test", "test"),
        ]]
    );
}

// ---- Beyond Monaco's tests ----

fn types(tokenizer: &MonarchTokenizer, line: &str) -> Vec<(usize, String)> {
    tokenizer
        .try_tokenize(line, true, &tokenizer.initial_state(), &())
        .unwrap()
        .tokens
        .into_iter()
        .map(|token| (token.offset, token.token_type))
        .collect()
}

fn ty(offset: usize, token_type: &str) -> (usize, String) {
    (offset, token_type.to_string())
}

#[test]
fn cases_keywords_and_brackets() {
    let tokenizer = tokenizer(
        "mini",
        json!({
            "keywords": ["fn", "let"],
            "typeKeywords": ["i32"],
            "operators": ["=", "=>"],
            "symbols": "[=><!~?:&|+\\-*\\/\\^%]+",
            "tokenizer": {
                "root": [
                    ["[a-z_$][\\w$]*", { "cases": {
                        "@typeKeywords": "keyword.type",
                        "@keywords": "keyword",
                        "@default": "identifier",
                    } }],
                    ["[{}()\\[\\]]", "@brackets"],
                    ["@symbols", { "cases": { "@operators": "operator", "@default": "" } }],
                    ["\\d+", "number"],
                    [" +", "white"],
                ],
            },
        }),
    );
    assert_eq!(
        types(&tokenizer, "let x: i32 = (1);"),
        vec![
            ty(0, "keyword.mini"),
            ty(3, "white.mini"),
            ty(4, "identifier.mini"),
            ty(5, ""),
            ty(6, "white.mini"),
            ty(7, "keyword.type.mini"),
            ty(10, "white.mini"),
            ty(11, "operator.mini"),
            ty(12, "white.mini"),
            ty(13, "delimiter.parenthesis.mini"),
            ty(14, "number.mini"),
            ty(15, "delimiter.parenthesis.mini"),
            ty(16, "source.mini"),
        ]
    );
}

#[test]
fn includes_dotted_states_and_substitutions() {
    let tokenizer = tokenizer(
        "test",
        json!({
            "tokenPostfix": ".t",
            "defaultToken": "invalid",
            "tokenizer": {
                "root": [
                    { "include": "@whitespace" },
                    ["([\"'])", { "token": "string.quote.$1", "next": "@string.$1" }],
                ],
                "whitespace": [[" +", "white"]],
                "string": [
                    // Only `{ token }` actions substitute, as in Monaco.
                    ["[^\"']+", { "token": "string.$S0" }],
                    ["([\"'])", { "cases": {
                        "$1==$S2": { "token": "string.quote", "next": "@pop" },
                        "@default": "string",
                    } }],
                ],
            },
        }),
    );
    let state = tokenizer.initial_state();
    let result = tokenizer
        .try_tokenize(" \"a'b\" x", true, &state, &())
        .unwrap();
    let tokens: Vec<_> = result
        .tokens
        .iter()
        .map(|t| (t.offset, t.token_type.as_str()))
        .collect();
    // Token names are sanitized: `"` becomes `-`.
    assert_eq!(
        tokens,
        vec![
            (0, "white.t"),
            (1, "string.quote.-.t"),
            (2, "string.string.-.t"),
            (3, "string.t"),
            (4, "string.string.-.t"),
            (5, "string.quote.t"),
            (6, "white.t"),
            (7, "invalid.t"),
        ]
    );
    assert_eq!(result.end_state.stack().collect::<Vec<_>>(), ["root"]);
}

#[test]
fn switch_to_go_back_and_groups() {
    let tokenizer = tokenizer(
        "test",
        json!({
            "tokenizer": {
                "root": [
                    ["(\\w+)(\\s*)(=)", ["variable", "", "operator"]],
                    ["#", { "token": "comment", "switchTo": "@comment" }],
                    ["ab", { "token": "ab", "goBack": 1 }],
                    ["b", "b"],
                ],
                "comment": [[".*", "comment"]],
            },
        }),
    );
    assert_eq!(
        types(&tokenizer, "x = abc"),
        vec![
            ty(0, "variable.test"),
            ty(1, ""),
            ty(2, "operator.test"),
            ty(3, "source.test"),
            ty(4, "ab.test"),
            ty(5, "b.test"),
            ty(6, "source.test"),
        ]
    );
    let result = tokenizer
        .try_tokenize("#x", true, &tokenizer.initial_state(), &())
        .unwrap();
    assert_eq!(result.end_state.stack().collect::<Vec<_>>(), ["comment"]);
}

#[test]
fn ignore_case_and_eos() {
    let tokenizer = tokenizer(
        "test",
        json!({
            "ignoreCase": true,
            "keywords": ["select"],
            "tokenizer": {
                "root": [
                    ["[a-z]+", { "cases": {
                        "@eos": "last",
                        "@keywords": "keyword",
                        "@default": "word",
                    } }],
                    [" ", ""],
                ],
            },
        }),
    );
    assert_eq!(
        types(&tokenizer, "SELECT Foo Bar"),
        vec![
            ty(0, "keyword.test"),
            ty(6, ""),
            ty(7, "word.test"),
            ty(10, ""),
            ty(11, "last.test"),
        ]
    );
}

#[test]
fn unknown_embedded_languages_get_empty_tokens() {
    let tokenizer = tokenizer(
        "test",
        json!({
            "tokenizer": {
                "root": [["<js>", { "token": "tag", "nextEmbedded": "text/javascript", "next": "@js" }]],
                "js": [["</js>", { "token": "tag", "nextEmbedded": "@pop", "next": "@pop" }]],
            },
        }),
    );
    let lines = ["<js>let x", "y</js>z"];
    assert_eq!(
        get_tokens(&tokenizer, &lines),
        vec![
            vec![t(0, "tag.test", "test"), t(4, "", "text/javascript"),],
            vec![
                t(0, "", "text/javascript"),
                t(1, "tag.test", "test"),
                t(6, "source.test", "test"),
            ],
        ]
    );
}

#[test]
fn runtime_errors_give_null_tokens() {
    let tokenizer = tokenizer(
        "test",
        json!({ "tokenizer": { "root": [["a", { "token": "a", "next": "@pop" }]] } }),
    );
    let state = tokenizer.initial_state();
    let error = tokenizer.try_tokenize("a", true, &state, &()).unwrap_err();
    assert!(
        error.0.starts_with("test: trying to pop an empty stack"),
        "{error}"
    );
    let result = tokenizer.tokenize("a", true, &state, &());
    assert_eq!(
        result.tokens,
        vec![MonarchToken {
            offset: 0,
            token_type: String::new(),
            language: "test".into(),
        }]
    );
    assert_eq!(result.end_state, state);

    let no_progress = tokenizer_for("test", json!({ "tokenizer": { "root": [["", "x"]] } }));
    assert!(
        no_progress
            .try_tokenize("a", true, &no_progress.initial_state(), &())
            .unwrap_err()
            .0
            .contains("no progress")
    );
}

fn tokenizer_for(language_id: &str, grammar: Value) -> MonarchTokenizer {
    tokenizer(language_id, grammar)
}

#[test]
fn invalid_grammars_are_errors() {
    let error = |grammar: Value| MonarchTokenizer::new("test", &grammar).unwrap_err().0;
    assert_eq!(
        error(json!(1)),
        "Monarch: expecting a language definition object"
    );
    assert!(error(json!({})).contains("'tokenizer' attribute"));
    assert!(
        error(json!({ "tokenizer": { "root": [["@nope", "x"]] } })).contains("attribute 'nope'")
    );
    assert!(
        error(json!({ "tokenizer": { "root": [["a", "x", "@nowhere"]] } }))
            .contains("the next state '@nowhere' is not defined")
    );
    assert!(
        error(json!({ "tokenizer": { "root": [{ "include": "@missing" }] } }))
            .contains("include target 'missing'")
    );
    assert!(error(json!({ "tokenizer": { "root": [["(", "x"]] } })).contains("invalid regular"));
    assert!(
        error(json!({ "tokenizer": { "root": [["a", { "cases": { "@kw": "x" } }]] } }))
            .contains("'kw' is not defined")
    );
    assert!(
        error(json!({ "brackets": [["(", "(", "x"]], "tokenizer": { "root": [] } }))
            .contains("must be different")
    );
    assert!(
        error(json!({ "tokenizer": { "root": [["a", { "token": "x", "bracket": "@x" }]] } }))
            .contains("'bracket' attribute")
    );
}

#[test]
fn utf16_go_back() {
    let tokenizer = tokenizer(
        "test",
        json!({ "tokenizer": { "root": [["é😀x", { "token": "a", "goBack": 3 }], ["é", "b"], ["😀x", "c"]] } }),
    );
    // goBack 3 UTF-16 units leaves `é` consumed.
    let result = types(&tokenizer, "é😀x");
    assert_eq!(result, vec![ty(0, "a.test"), ty(2, "c.test")]);
}

/// The C-like sample from Monaco's Monarch documentation, as JSON.
#[test]
fn monarch_documentation_sample() {
    let tokenizer = tokenizer(
        "mylang",
        json!({
            "defaultToken": "invalid",
            "keywords": ["class", "new", "return", "if", "else", "true", "false"],
            "typeKeywords": ["boolean", "double", "int"],
            "operators": ["=", ">", "<", "!", "==", "<=", ">=", "!=", "&&", "||", "+", "-", "*", "/"],
            "symbols": "[=><!~?:&|+\\-*\\/\\^%]+",
            "escapes": "\\\\(?:[abfnrtv\\\\\"']|x[0-9A-Fa-f]{1,4}|u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8})",
            "tokenizer": {
                "root": [
                    ["[a-z_$][\\w$]*", { "cases": {
                        "@typeKeywords": "keyword",
                        "@keywords": "keyword",
                        "@default": "identifier",
                    } }],
                    ["[A-Z][\\w\\$]*", "type.identifier"],
                    { "include": "@whitespace" },
                    ["[{}()\\[\\]]", "@brackets"],
                    ["[<>](?!@symbols)", "@brackets"],
                    ["@symbols", { "cases": { "@operators": "operator", "@default": "" } }],
                    ["@\\s*[a-zA-Z_\\$][\\w\\$]*", { "token": "annotation", "log": "annotation token: $0" }],
                    ["\\d*\\.\\d+([eE][\\-+]?\\d+)?", "number.float"],
                    ["0[xX][0-9a-fA-F]+", "number.hex"],
                    ["\\d+", "number"],
                    ["[;,.]", "delimiter"],
                    ["\"([^\"\\\\]|\\\\.)*$", "string.invalid"],
                    ["\"", { "token": "string.quote", "bracket": "@open", "next": "@string" }],
                    ["'[^\\\\']'", "string"],
                    ["(')(@escapes)(')", ["string", "string.escape", "string"]],
                    ["'", "string.invalid"],
                ],
                "comment": [
                    ["[^\\/*]+", "comment"],
                    ["\\/\\*", "comment", "@push"],
                    ["\\*/", "comment", "@pop"],
                    ["[\\/*]", "comment"],
                ],
                "string": [
                    ["[^\\\\\"]+", "string"],
                    ["@escapes", "string.escape"],
                    ["\\\\.", "string.escape.invalid"],
                    ["\"", { "token": "string.quote", "bracket": "@close", "next": "@pop" }],
                ],
                "whitespace": [
                    ["[ \\t\\r\\n]+", "white"],
                    ["\\/\\*", "comment", "@comment"],
                    ["\\/\\/.*$", "comment"],
                ],
            },
        }),
    );
    let lines = [
        "class Foo { int x = 0x1F; /* a /* nested",
        "*/ still */ s = \"a\\tb\\q\"; c = '\\n'; @Override",
    ];
    let tokens = get_tokens(&tokenizer, &lines);
    let pick = |line: &[(usize, String, String)]| -> Vec<(usize, String)> {
        line.iter().map(|(o, t, _)| (*o, t.clone())).collect()
    };
    assert_eq!(
        pick(&tokens[0]),
        vec![
            ty(0, "keyword.mylang"),
            ty(5, "white.mylang"),
            ty(6, "type.identifier.mylang"),
            ty(9, "white.mylang"),
            ty(10, "delimiter.curly.mylang"),
            ty(11, "white.mylang"),
            ty(12, "keyword.mylang"),
            ty(15, "white.mylang"),
            ty(16, "identifier.mylang"),
            ty(17, "white.mylang"),
            ty(18, "operator.mylang"),
            ty(19, "white.mylang"),
            ty(20, "number.hex.mylang"),
            ty(24, "delimiter.mylang"),
            ty(25, "white.mylang"),
            ty(26, "comment.mylang"),
        ]
    );
    assert_eq!(
        pick(&tokens[1]),
        vec![
            ty(0, "comment.mylang"),
            ty(11, "white.mylang"),
            ty(12, "identifier.mylang"),
            ty(13, "white.mylang"),
            ty(14, "operator.mylang"),
            ty(15, "white.mylang"),
            ty(16, "string.quote.mylang"),
            ty(17, "string.mylang"),
            ty(18, "string.escape.mylang"),
            ty(20, "string.mylang"),
            ty(21, "string.escape.invalid.mylang"),
            ty(23, "string.quote.mylang"),
            ty(24, "delimiter.mylang"),
            ty(25, "white.mylang"),
            ty(26, "identifier.mylang"),
            ty(27, "white.mylang"),
            ty(28, "operator.mylang"),
            ty(29, "white.mylang"),
            ty(30, "string.mylang"),
            ty(31, "string.escape.mylang"),
            ty(33, "string.mylang"),
            ty(34, "delimiter.mylang"),
            ty(35, "white.mylang"),
            ty(36, "annotation.mylang"),
        ]
    );
}

// ---- MonarchHighlighter ----

fn comment_language() -> Arc<LanguageRegistry> {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(
        Language::new("blocky", "Blocky")
            .with_monarch_grammar(&json!({
                "tokenizer": {
                    "root": [
                        ["/\\*", "comment", "@comment"],
                        ["[a-z]+", "identifier"],
                    ],
                    "comment": [
                        ["\\*/", "comment", "@pop"],
                        [".", "comment"],
                    ],
                },
            }))
            .unwrap(),
    );
    registry
}

fn highlighter(registry: &Arc<LanguageRegistry>) -> Arc<dyn Highlighter> {
    let language = registry.language_for_id("blocky").unwrap();
    (language.highlighter.as_ref().unwrap())(&language, registry)
}

fn names(
    highlighter: &dyn Highlighter,
    state: &HighlightState,
    buffer: &Buffer,
) -> Vec<(String, String)> {
    let text = buffer.text();
    highlighter
        .highlights(state, buffer.rope(), 0..buffer.rope().len())
        .into_iter()
        .map(|span| {
            (
                text[span.range].to_string(),
                span.highlight.name().to_string(),
            )
        })
        .collect()
}

use crate::highlighter::HighlightState;

#[test]
fn highlighter_spans_are_monaco_tokens() {
    let registry = comment_language();
    let highlighter = highlighter(&registry);
    let buffer = Buffer::new("ab /* c\nd */ e");
    let state = highlighter.update(None, &buffer.snapshot());
    let spans = highlighter.highlights(&state, buffer.rope(), 0..buffer.rope().len());
    assert!(spans.iter().all(|span| span.highlight.is_monaco_token()));
    assert_eq!(
        names(&*highlighter, &state, &buffer),
        vec![
            ("ab".into(), "identifier.blocky".into()),
            (" ".into(), "source.blocky".into()),
            ("/* c".into(), "comment.blocky".into()),
            ("d */".into(), "comment.blocky".into()),
            (" ".into(), "source.blocky".into()),
            ("e".into(), "identifier.blocky".into()),
        ]
    );
    // Monaco token names don't collide with tree-sitter captures.
    assert_ne!(
        HighlightId::intern("comment.blocky"),
        HighlightId::intern_monaco_token("comment.blocky")
    );
    assert!(!HighlightId::intern("comment.blocky").is_monaco_token());
}

#[test]
fn highlighter_retokenizes_until_states_converge() {
    let registry = comment_language();
    let highlighter = highlighter(&registry);
    let mut buffer = Buffer::new("a\nb\nc\nd");
    let old = buffer.snapshot();
    let state = highlighter.update(None, &old);

    // Opening a comment changes every following line.
    buffer.edit([(0..0, "/*")]).unwrap();
    let patch = buffer.edits_since(old.version());
    let state = highlighter.update(Some((&state, old.rope(), &patch)), &buffer.snapshot());
    assert_eq!(
        names(&*highlighter, &state, &buffer),
        vec![
            ("/*a".into(), "comment.blocky".into()),
            ("b".into(), "comment.blocky".into()),
            ("c".into(), "comment.blocky".into()),
            ("d".into(), "comment.blocky".into()),
        ]
    );

    // Editing inside the comment stops at the next line, whose state is
    // unchanged; the result matches tokenizing from scratch.
    let old = buffer.snapshot();
    buffer.edit([(5..5, "xx\n*/")]).unwrap();
    let patch = buffer.edits_since(old.version());
    let incremental = highlighter.update(Some((&state, old.rope(), &patch)), &buffer.snapshot());
    let scratch = highlighter.update(None, &buffer.snapshot());
    assert_eq!(
        names(&*highlighter, &incremental, &buffer),
        names(&*highlighter, &scratch, &buffer)
    );
    let incremental = incremental.downcast_ref::<MonarchHighlightState>().unwrap();
    let scratch = scratch.downcast_ref::<MonarchHighlightState>().unwrap();
    assert_eq!(incremental.line_count(), 5);
    for row in 0..5 {
        assert_eq!(incremental.line_tokens(row), scratch.line_tokens(row));
        assert_eq!(incremental.end_state(row), scratch.end_state(row));
    }

    // Edits deleting lines.
    let old = buffer.snapshot();
    let state = highlighter.update(None, &old);
    buffer.edit([(0..2, ""), (7..12, "")]).unwrap();
    let patch = buffer.edits_since(old.version());
    let incremental = highlighter.update(Some((&state, old.rope(), &patch)), &buffer.snapshot());
    let scratch = highlighter.update(None, &buffer.snapshot());
    assert_eq!(
        names(&*highlighter, &incremental, &buffer),
        names(&*highlighter, &scratch, &buffer)
    );
}

#[test]
fn highlighter_embeds_registered_monarch_languages() {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(
        Language::new("inner", "Inner")
            .with_mimetypes(&["text/x-inner"])
            .with_monarch_grammar(&json!({ "tokenizer": { "root": [["\\w+", "word"]] } }))
            .unwrap(),
    );
    registry.register(
        Language::new("outer", "Outer")
            .with_monarch_grammar(&json!({
                "tokenizer": {
                    "root": [["<", { "token": "tag", "next": "@inner", "nextEmbedded": "text/x-inner" }]],
                    "inner": [[">", { "token": "tag", "next": "@pop", "nextEmbedded": "@pop" }]],
                },
            }))
            .unwrap(),
    );
    let language = registry.language_for_id("outer").unwrap();
    let highlighter = (language.highlighter.as_ref().unwrap())(&language, &registry);
    let monarch = MonarchHighlighter::downcast(&*highlighter).unwrap();
    let tokens = monarch.tokenize_text("<ab>\n<\ncd\n>");
    let flat: Vec<Vec<(usize, &str, &str)>> = tokens
        .iter()
        .map(|line| {
            line.iter()
                .map(|t| (t.offset, t.token_type.as_str(), &*t.language))
                .collect()
        })
        .collect();
    assert_eq!(
        flat,
        vec![
            vec![
                (0, "tag.outer", "outer"),
                (1, "word.inner", "inner"),
                (3, "tag.outer", "outer")
            ],
            vec![(0, "tag.outer", "outer")],
            vec![(0, "word.inner", "inner")],
            vec![(0, "tag.outer", "outer")],
        ]
    );

    let buffer = Buffer::new("<ab>");
    let state = highlighter.update(None, &buffer.snapshot());
    let state = state.downcast_ref::<MonarchHighlightState>().unwrap();
    assert_eq!(state.language_at(0, 2).as_deref(), Some("inner"));
    assert_eq!(state.language_at(0, 3).as_deref(), Some("outer"));
}
