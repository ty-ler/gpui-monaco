//! The language configurations Monaco's tests register, translated to
//! [`syntax::LanguageConfiguration`]: the inline ones of
//! `src/vs/editor/test/browser/controller/cursor.test.ts`, Monaco's
//! `plaintext` defaults from
//! `src/vs/editor/common/languages/languageConfigurationRegistry.ts`, and the
//! test modes of `src/vs/editor/test/common/modes/supports/*.ts` (MIT).
//!
//! JavaScript regexes are copied verbatim except where `fancy-regex` needs a
//! literal `{` escaped.

use syntax::{
    AutoClosingPair, Comments, EnterAction, IndentAction, IndentationRules, LanguageConfiguration,
    NotIn, OnEnterRule, Pattern, SurroundingPair,
};

pub fn re(source: &str) -> Pattern {
    let pattern = Pattern::new(source, "");
    assert!(
        pattern.regex().is_some(),
        "pattern does not compile: {source}"
    );
    pattern
}

pub fn re_i(source: &str) -> Pattern {
    let pattern = Pattern::new(source, "i");
    assert!(
        pattern.regex().is_some(),
        "pattern does not compile: {source}"
    );
    pattern
}

pub fn pair(open: &str, close: &str) -> AutoClosingPair {
    AutoClosingPair {
        open: open.into(),
        close: close.into(),
        not_in: Vec::new(),
    }
}

pub fn pair_not_in(open: &str, close: &str, not_in: &[NotIn]) -> AutoClosingPair {
    AutoClosingPair {
        open: open.into(),
        close: close.into(),
        not_in: not_in.to_vec(),
    }
}

pub fn surround(open: &str, close: &str) -> SurroundingPair {
    SurroundingPair {
        open: open.into(),
        close: close.into(),
    }
}

pub fn brackets(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(o, c)| (o.to_string(), c.to_string()))
        .collect()
}

pub fn standard_brackets() -> Vec<(String, String)> {
    brackets(&[("{", "}"), ("[", "]"), ("(", ")")])
}

pub fn rules(increase: Pattern, decrease: Pattern) -> IndentationRules {
    IndentationRules {
        increase_indent_pattern: increase,
        decrease_indent_pattern: decrease,
        indent_next_line_pattern: None,
        un_indented_line_pattern: None,
    }
}

pub fn on_enter(
    before: Pattern,
    after: Option<Pattern>,
    previous: Option<Pattern>,
    indent: IndentAction,
    append_text: Option<&str>,
    remove_text: Option<u32>,
) -> OnEnterRule {
    OnEnterRule {
        before_text: before,
        after_text: after,
        previous_line_text: previous,
        action: EnterAction {
            indent,
            append_text: append_text.map(str::to_string),
            remove_text,
        },
    }
}

// ---------------------------------------------------------------------------
// cursor.test.ts, suite 'Editor Controller'

/// Monaco's built-in `plaintext` configuration.
pub fn plaintext() -> LanguageConfiguration {
    LanguageConfiguration {
        brackets: brackets(&[("(", ")"), ("[", "]"), ("{", "}")]),
        surrounding_pairs: vec![
            surround("{", "}"),
            surround("[", "]"),
            surround("(", ")"),
            surround("<", ">"),
            surround("\"", "\""),
            surround("'", "'"),
            surround("`", "`"),
        ],
        ..Default::default()
    }
}

/// `surroundingLanguage`.
pub fn surrounding_language() -> LanguageConfiguration {
    LanguageConfiguration {
        auto_closing_pairs: vec![pair("(", ")")],
        ..Default::default()
    }
}

/// `emptyClosingSurroundLanguage`.
pub fn empty_closing_surround_language() -> LanguageConfiguration {
    LanguageConfiguration {
        surrounding_pairs: vec![surround("<", "")],
        ..Default::default()
    }
}

/// `indentRulesLanguage`.
pub fn indent_rules_language() -> LanguageConfiguration {
    LanguageConfiguration {
        indentation_rules: Some(IndentationRules {
            decrease_indent_pattern: re(
                r"^\s*((?!\S.*\/[*]).*[*]\/\s*)?[})\]]|^\s*(case\b.*|default):\s*(\/\/.*|\/[*].*[*]\/\s*)?$",
            ),
            increase_indent_pattern: re(
                r#"^((?!\/\/).)*(\{[^}"'`]*|\([^)"']*|\[[^\]"']*|^\s*(\{\}|\(\)|\[\]|(case\b.*|default):))\s*(\/\/.*|\/[*].*[*]\/\s*)?$"#,
            ),
            indent_next_line_pattern: Some(re(
                r"^\s*(for|while|if|else)\b(?!.*[;{}]\s*(\/\/.*|\/[*].*[*]\/\s*)?$)",
            )),
            un_indented_line_pattern: Some(re(
                r#"^(?!.*([;{}]|\S:)\s*(\/\/.*|\/[*].*[*]\/\s*)?$)(?!.*(\{[^}"']*|\([^)"']*|\[[^\]"']*|^\s*(\{\}|\(\)|\[\]|(case\b.*|default):))\s*(\/\/.*|\/[*].*[*]\/\s*)?$)(?!^\s*((?!\S.*\/[*]).*[*]\/\s*)?[})\]]|^\s*(case\b.*|default):\s*(\/\/.*|\/[*].*[*]\/\s*)?$)(?!^\s*(for|while|if|else)\b(?!.*[;{}]\s*(\/\/.*|\/[*].*[*]\/\s*)?$))"#,
            )),
        }),
        ..Default::default()
    }
}

/// `electricCharLanguage`; its legacy `docComment` adds the `/**` pair to
/// the brackets' auto-closing pairs.
pub fn electric_char_language() -> LanguageConfiguration {
    LanguageConfiguration {
        brackets: standard_brackets(),
        auto_closing_pairs: vec![
            pair("{", "}"),
            pair("[", "]"),
            pair("(", ")"),
            pair("/**", " */"),
        ],
        ..Default::default()
    }
}

fn auto_closing_pairs(with_begin: bool) -> Vec<AutoClosingPair> {
    let mut pairs = vec![
        pair("{", "}"),
        pair("[", "]"),
        pair("(", ")"),
        pair_not_in("'", "'", &[NotIn::String, NotIn::Comment]),
        pair_not_in("\"", "\"", &[NotIn::String]),
        pair_not_in("`", "`", &[NotIn::String, NotIn::Comment]),
        pair_not_in("/**", " */", &[NotIn::String]),
    ];
    if with_begin {
        pairs.push(pair_not_in("begin", "end", &[NotIn::String]));
    }
    // The legacy `docComment` support appends `/**` once more.
    pairs.push(pair("/**", " */"));
    pairs
}

/// `autoClosingLanguage`.
pub fn auto_closing_language() -> LanguageConfiguration {
    LanguageConfiguration {
        comments: Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("/*".into(), "*/".into())),
        },
        auto_closing_pairs: auto_closing_pairs(true),
        ..Default::default()
    }
}

/// `autoClosingLanguage` after `setAutoClosingLanguageEnabledSet(chars)`.
pub fn auto_closing_language_with_auto_close_before(chars: &str) -> LanguageConfiguration {
    let mut pairs = auto_closing_pairs(false);
    pairs.pop();
    pairs.push(pair("/**", " */"));
    LanguageConfiguration {
        auto_close_before: Some(chars.into()),
        auto_closing_pairs: pairs,
        ..auto_closing_language()
    }
}

/// `setupOnEnterLanguage(indentAction)`.
pub fn on_enter_language(indent: IndentAction) -> LanguageConfiguration {
    LanguageConfiguration {
        on_enter_rules: vec![on_enter(re(".*"), None, None, indent, None, None)],
        ..Default::default()
    }
}

/// `setupIndentRulesLanguage(id, rules)`.
pub fn indent_rules_only(increase: Pattern, decrease: Pattern) -> LanguageConfiguration {
    LanguageConfiguration {
        indentation_rules: Some(rules(increase, decrease)),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// test/common/modes/supports (the languages of indentation.test.ts)

pub fn javascript_auto_closing_pairs() -> Vec<AutoClosingPair> {
    vec![
        pair("{", "}"),
        pair("[", "]"),
        pair("(", ")"),
        pair_not_in("'", "'", &[NotIn::String, NotIn::Comment]),
        pair_not_in("\"", "\"", &[NotIn::String]),
        pair_not_in("`", "`", &[NotIn::String, NotIn::Comment]),
        pair_not_in("/**", " */", &[NotIn::String]),
    ]
}

pub fn javascript_indentation_rules() -> IndentationRules {
    IndentationRules {
        decrease_indent_pattern: re(r"^((?!.*?\/\*).*\*\/)?\s*[\}\]\)].*$"),
        increase_indent_pattern: re(
            r#"^((?!\/\/).)*(\{([^}"'`]*|(\t|[ ])*\/\/.*)|\([^)"'`]*|\[[^\]"'`]*)$"#,
        ),
        un_indented_line_pattern: Some(re(
            r"^(\t|[ ])*[ ]\*[^/]*\*\/\s*$|^(\t|[ ])*[ ]\*\/\s*$|^(\t|[ ])*[ ]\*([ ]([^\*]|\*(?!\/))*)?$",
        )),
        indent_next_line_pattern: Some(re(
            r"^((.*=>\s*)|((.*[^\w]+|\s*)(if|while|for)\s*\(.*\)\s*))$",
        )),
    }
}

pub fn javascript_on_enter_rules() -> Vec<OnEnterRule> {
    vec![
        // e.g. /** | */
        on_enter(
            re(r"^\s*\/\*\*(?!\/)([^\*]|\*(?!\/))*$"),
            Some(re(r"^\s*\*\/$")),
            None,
            IndentAction::IndentOutdent,
            Some(" * "),
            None,
        ),
        // e.g. /** ...|
        on_enter(
            re(r"^\s*\/\*\*(?!\/)([^\*]|\*(?!\/))*$"),
            None,
            None,
            IndentAction::None,
            Some(" * "),
            None,
        ),
        // e.g.  * ...|
        on_enter(
            re(r"^(\t|[ ])*[ ]\*([ ]([^\*]|\*(?!\/))*)?$"),
            None,
            Some(re(r"(?=^(\s*(\/\*\*|\*)).*)(?=(?!(\s*\*\/)))")),
            IndentAction::None,
            Some("* "),
            None,
        ),
        // e.g.  */|
        on_enter(
            re(r"^(\t|[ ])*[ ]\*\/\s*$"),
            None,
            None,
            IndentAction::None,
            None,
            Some(1),
        ),
        // e.g.  *-----*/|
        on_enter(
            re(r"^(\t|[ ])*[ ]\*[^/]*\*\/\s*$"),
            None,
            None,
            IndentAction::None,
            None,
            Some(1),
        ),
        on_enter(
            re(r"^\s*(\bcase\s.+:|\bdefault:)$"),
            Some(re(r"^(?!\s*(\bcase\b|\bdefault\b))")),
            None,
            IndentAction::Indent,
            None,
            None,
        ),
        on_enter(
            re(r"^\s+([^{i\s]|i(?!f\b))"),
            None,
            Some(re(r"^\s*(((else ?)?if|for|while)\s*\(.*\)\s*|else\s*)$")),
            IndentAction::Outdent,
            None,
            None,
        ),
        // Indent when pressing enter from inside ()
        on_enter(
            re(r"^.*\([^\)]*$"),
            Some(re(r"^\s*\).*$")),
            None,
            IndentAction::IndentOutdent,
            Some("\t"),
            None,
        ),
        // Indent when pressing enter from inside {}
        on_enter(
            re(r"^.*\{[^\}]*$"),
            Some(re(r"^\s*\}.*$")),
            None,
            IndentAction::IndentOutdent,
            Some("\t"),
            None,
        ),
        // Indent when pressing enter from inside []
        on_enter(
            re(r"^.*\[[^\]]*$"),
            Some(re(r"^\s*\].*$")),
            None,
            IndentAction::IndentOutdent,
            Some("\t"),
            None,
        ),
    ]
}

/// `Language.TypeScript` of indentation.test.ts.
pub fn typescript_test_language() -> LanguageConfiguration {
    LanguageConfiguration {
        brackets: brackets(&[("${", "}"), ("{", "}"), ("[", "]"), ("(", ")")]),
        comments: Comments {
            line_comment: Some("//".into()),
            line_comment_no_indent: false,
            block_comment: Some(("/*".into(), "*/".into())),
        },
        auto_closing_pairs: javascript_auto_closing_pairs(),
        indentation_rules: Some(javascript_indentation_rules()),
        on_enter_rules: javascript_on_enter_rules(),
        ..Default::default()
    }
}

/// `Language.Ruby` of indentation.test.ts.
pub fn ruby_test_language() -> LanguageConfiguration {
    LanguageConfiguration {
        brackets: standard_brackets(),
        indentation_rules: Some(rules(
            re(
                r#"^\s*((begin|class|(private|protected)\s+def|def|else|elsif|ensure|for|if|module|rescue|unless|until|when|in|while|case)|([^#]*\sdo\b)|([^#]*=\s*(case|if|unless)))\b([^#\{;]|(\"|'|\/).*\4)*(#.*)?$"#,
            ),
            re(
                r"^\s*([}\]]([,)]?\s*(#|$)|\.[a-zA-Z_]\w*\b)|(end|rescue|ensure|else|elsif)\b|(in|when)\s)",
            ),
        )),
        ..Default::default()
    }
}

/// `Language.PHP` of indentation.test.ts.
pub fn php_test_language() -> LanguageConfiguration {
    LanguageConfiguration {
        brackets: standard_brackets(),
        indentation_rules: Some(rules(
            re(
                r"(\{(?!.*\}).*|\(|\[|((else(\s)?)?if|else|for(each)?|while|switch|case).*:)\s*((\/[/*].*|)?$|\?>)",
            ),
            re(
                r"^(.*\*\/)?\s*((\})|(\)+[;,])|(\]\)*[;,])|\b(else:)|\b((end(if|for(each)?|while|switch));))",
            ),
        )),
        on_enter_rules: vec![
            on_enter(
                re(r"^\s*\/\*\*(?!\/)([^\*]|\*(?!\/))*$"),
                Some(re(r"^\s*\*\/$")),
                None,
                IndentAction::IndentOutdent,
                Some(" * "),
                None,
            ),
            on_enter(
                re(r"^\s*\/\*\*(?!\/)([^\*]|\*(?!\/))*$"),
                None,
                None,
                IndentAction::None,
                Some(" * "),
                None,
            ),
            on_enter(
                re(r"^(\t|(\ \ ))*\ \*(\ ([^\*]|\*(?!\/))*)?$"),
                None,
                None,
                IndentAction::None,
                Some("* "),
                None,
            ),
            on_enter(
                re(r"^(\t|(\ \ ))*\ \*\/\s*$"),
                None,
                None,
                IndentAction::None,
                None,
                Some(1),
            ),
            on_enter(
                re(r"^(\t|(\ \ ))*\ \*[^/]*\*\/\s*$"),
                None,
                None,
                IndentAction::None,
                None,
                Some(1),
            ),
            on_enter(
                re(r"^\s+([^{i\s]|i(?!f\b))"),
                None,
                Some(re(
                    r"^\s*(((else ?)?if|for(each)?|while)\s*\(.*\)\s*|else\s*)$",
                )),
                IndentAction::Outdent,
                None,
                None,
            ),
        ],
        ..Default::default()
    }
}

/// `Language.VB` of indentation.test.ts.
pub fn vb_test_language() -> LanguageConfiguration {
    LanguageConfiguration {
        brackets: standard_brackets(),
        indentation_rules: Some(rules(
            re_i(
                r"^\s*((If|ElseIf).*Then(?!.*End\s+If)\s*(('|REM).*)?|(Else|While|For|Do|Select\s+Case|Case|Sub|Function|Class|Module|Enum|Structure|Interface|Namespace|With|Try|Catch|Finally|SyncLock|Using|Property|Get|Set|AddHandler|RaiseEvent|RemoveHandler|Event|Operator)\b(?!.*\bEnd\s+(If|Sub|Function|Class|Module|Enum|Structure|Interface|Namespace|With|Select|Try|While|For|Property|Get|Set|SyncLock|Using|AddHandler|RaiseEvent|RemoveHandler|Event|Operator)\b).*(('|REM).*)?)$",
            ),
            re_i(
                r"^\s*((End\s+(If|Sub|Function|Class|Module|Enum|Structure|Interface|Namespace|With|Select|Try|While|For|Property|Get|Set|SyncLock|Using|AddHandler|RaiseEvent|RemoveHandler|Event|Operator))|Else|ElseIf|Case|Catch|Finally|Loop|Next|Wend|Until)\b",
            ),
        )),
        on_enter_rules: vec![on_enter(
            re_i(
                r"^\s*((End\s+(If|Sub|Function|Class|Module|Enum|Structure|Interface|Namespace|With|Select|Try|While|For|Property|Get|Set|SyncLock|Using|AddHandler|RaiseEvent|RemoveHandler|Event|Operator))|Loop|Next|Wend|Until)\b.*$",
            ),
            None,
            None,
            IndentAction::None,
            None,
            None,
        )],
        ..Default::default()
    }
}

#[test]
fn test_language_patterns_compile() {
    // `re` asserts that every pattern compiles.
    for config in [
        plaintext(),
        indent_rules_language(),
        electric_char_language(),
        auto_closing_language(),
        typescript_test_language(),
        ruby_test_language(),
        php_test_language(),
        vb_test_language(),
    ] {
        let _ = config;
    }
}
