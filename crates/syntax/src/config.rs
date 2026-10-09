//! Per-language editing rules in VS Code's `language-configuration.json`
//! format, so VS Code's (MIT-licensed) configurations can be reused as-is.
//!
//! Patterns are JavaScript regular expressions; they are compiled with
//! `fancy-regex`, which supports the lookarounds these files commonly use.

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageConfiguration {
    #[serde(default)]
    pub comments: Comments,
    #[serde(default)]
    pub brackets: Vec<(String, String)>,
    #[serde(default)]
    pub auto_closing_pairs: Vec<AutoClosingPair>,
    /// Characters before which auto-closing happens (Monaco `autoCloseBefore`).
    #[serde(default)]
    pub auto_close_before: Option<String>,
    #[serde(default)]
    pub surrounding_pairs: Vec<SurroundingPair>,
    #[serde(default)]
    pub word_pattern: Option<Pattern>,
    #[serde(default)]
    pub indentation_rules: Option<IndentationRules>,
    #[serde(default)]
    pub on_enter_rules: Vec<OnEnterRule>,
    #[serde(default)]
    pub folding: Option<Folding>,
}

impl LanguageConfiguration {
    /// Parses a `language-configuration.json` (JSON with comments and trailing
    /// commas tolerated).
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(&strip_jsonc(json))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comments {
    pub line_comment: Option<String>,
    pub block_comment: Option<(String, String)>,
}

/// Where an auto-closing pair is not inserted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotIn {
    String,
    Comment,
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoClosingPair {
    pub open: String,
    pub close: String,
    pub not_in: Vec<NotIn>,
}

impl<'de> Deserialize<'de> for AutoClosingPair {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Pair(String, String),
            Object {
                open: String,
                close: String,
                #[serde(default, rename = "notIn")]
                not_in: Vec<NotIn>,
            },
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Pair(open, close) => AutoClosingPair {
                open,
                close,
                not_in: Vec::new(),
            },
            Raw::Object {
                open,
                close,
                not_in,
            } => AutoClosingPair {
                open,
                close,
                not_in,
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurroundingPair {
    pub open: String,
    pub close: String,
}

impl<'de> Deserialize<'de> for SurroundingPair {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Pair(String, String),
            Object { open: String, close: String },
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Pair(open, close) | Raw::Object { open, close } => SurroundingPair { open, close },
        })
    }
}

/// A JavaScript regex, given either as a string or `{ "pattern", "flags" }`.
#[derive(Clone, Debug)]
pub struct Pattern {
    pub source: String,
    pub flags: String,
    regex: Option<fancy_regex::Regex>,
}

impl Pattern {
    pub fn new(source: impl Into<String>, flags: impl Into<String>) -> Self {
        let source = source.into();
        let flags = flags.into();
        let mut prefix = String::new();
        if flags.contains('i') {
            prefix.push_str("(?i)");
        }
        if flags.contains('m') {
            prefix.push_str("(?m)");
        }
        if flags.contains('s') {
            prefix.push_str("(?s)");
        }
        let regex = fancy_regex::Regex::new(&format!("{prefix}{source}")).ok();
        Self {
            source,
            flags,
            regex,
        }
    }

    /// The compiled regex, or `None` if the JavaScript pattern uses syntax
    /// `fancy-regex` cannot compile.
    pub fn regex(&self) -> Option<&fancy_regex::Regex> {
        self.regex.as_ref()
    }

    /// Whether the pattern matches somewhere in `text`. Patterns that failed to
    /// compile never match.
    pub fn is_match(&self, text: &str) -> bool {
        self.regex
            .as_ref()
            .is_some_and(|regex| regex.is_match(text).unwrap_or(false))
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Source(String),
            Object {
                pattern: String,
                #[serde(default)]
                flags: String,
            },
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Source(source) => Pattern::new(source, ""),
            Raw::Object { pattern, flags } => Pattern::new(pattern, flags),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndentationRules {
    pub increase_indent_pattern: Pattern,
    pub decrease_indent_pattern: Pattern,
    #[serde(default)]
    pub indent_next_line_pattern: Option<Pattern>,
    #[serde(default)]
    pub un_indented_line_pattern: Option<Pattern>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndentAction {
    None,
    Indent,
    IndentOutdent,
    Outdent,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnterAction {
    pub indent: IndentAction,
    #[serde(default)]
    pub append_text: Option<String>,
    #[serde(default)]
    pub remove_text: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnEnterRule {
    pub before_text: Pattern,
    #[serde(default)]
    pub after_text: Option<Pattern>,
    #[serde(default)]
    pub previous_line_text: Option<Pattern>,
    pub action: EnterAction,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folding {
    #[serde(default)]
    pub off_side: bool,
    #[serde(default)]
    pub markers: Option<FoldingMarkers>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FoldingMarkers {
    pub start: Pattern,
    pub end: Pattern,
}

/// Removes `//` and `/* */` comments and trailing commas outside strings.
fn strip_jsonc(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let mut chars = json.chars().peekable();
    let mut in_string = false;
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for next in chars.by_ref() {
                    if prev == '*' && next == '/' {
                        break;
                    }
                    prev = next;
                }
            }
            _ => out.push(ch),
        }
    }
    // Trailing commas: `,` followed by whitespace and `]` or `}`.
    let bytes: Vec<char> = out.chars().collect();
    let mut result = String::with_capacity(out.len());
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if in_string {
            result.push(ch);
            if ch == '\\' && i + 1 < bytes.len() {
                result.push(bytes[i + 1]);
                i += 1;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            result.push(ch);
        } else if ch == ',' {
            let next = bytes[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some(']') | Some('}')) {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vscode_style_configuration() {
        let json = r#"{
            // comment
            "comments": { "lineComment": "//", "blockComment": ["/*", "*/"] },
            "brackets": [["{", "}"], ["[", "]"]],
            "autoClosingPairs": [
                { "open": "{", "close": "}" },
                ["[", "]"],
                { "open": "\"", "close": "\"", "notIn": ["string", "comment"] },
            ],
            "surroundingPairs": [["(", ")"], { "open": "'", "close": "'" }],
            "wordPattern": { "pattern": "(-?\\d*\\.\\d\\w*)|([^\\s]+)", "flags": "g" },
            "indentationRules": {
                "increaseIndentPattern": "^.*\\{[^}\"']*$",
                "decreaseIndentPattern": "^(.*\\*/)?\\s*\\}.*$"
            },
            "onEnterRules": [
                { "beforeText": "^\\s*/\\*\\*(?!/)([^\\*]|\\*(?!/))*$", "action": { "indent": "none", "appendText": " * " } }
            ],
        }"#;
        let config = LanguageConfiguration::from_json(json).unwrap();
        assert_eq!(config.comments.line_comment.as_deref(), Some("//"));
        assert_eq!(config.brackets.len(), 2);
        assert_eq!(
            config.auto_closing_pairs[2].not_in,
            vec![NotIn::String, NotIn::Comment]
        );
        assert_eq!(config.surrounding_pairs[1].open, "'");
        let rules = config.indentation_rules.unwrap();
        assert!(rules.increase_indent_pattern.is_match("fn main() {"));
        assert!(rules.decrease_indent_pattern.is_match("    }"));
        // Lookahead from a real VS Code rule compiles.
        assert!(config.on_enter_rules[0].before_text.is_match("  /** docs"));
        assert!(
            !config.on_enter_rules[0]
                .before_text
                .is_match("  /** docs */")
        );
    }
}
