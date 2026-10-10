//! Per-language editing rules in VS Code's `language-configuration.json`
//! format, so VS Code's (MIT-licensed) configurations can be reused as-is.
//!
//! Patterns are JavaScript regular expressions; they are compiled with
//! `fancy-regex`, which supports the lookarounds these files commonly use.

use serde::Deserialize;

/// A parsed `language-configuration.json`. Unknown keys are ignored and missing
/// ones default to empty.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageConfiguration {
    /// `comments`: line and block comment tokens.
    #[serde(default)]
    pub comments: Comments,
    /// `brackets`: open/close pairs used for bracket matching and indentation.
    #[serde(default)]
    pub brackets: Vec<(String, String)>,
    /// `autoClosingPairs`: pairs closed automatically when the open text is typed.
    #[serde(default)]
    pub auto_closing_pairs: Vec<AutoClosingPair>,
    /// Characters before which auto-closing happens (Monaco `autoCloseBefore`).
    #[serde(default)]
    pub auto_close_before: Option<String>,
    /// `surroundingPairs`: pairs that wrap the selection when the open text is typed.
    #[serde(default)]
    pub surrounding_pairs: Vec<SurroundingPair>,
    /// `wordPattern`: what counts as a word, for word navigation and selection.
    #[serde(default)]
    pub word_pattern: Option<Pattern>,
    /// `indentationRules`: how typing and Enter adjust indentation.
    #[serde(default)]
    pub indentation_rules: Option<IndentationRules>,
    /// `onEnterRules`: special Enter handling, such as continuing doc comments.
    #[serde(default)]
    pub on_enter_rules: Vec<OnEnterRule>,
    /// `folding`: indentation-based folding settings and region markers.
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

/// The `comments` section of a language configuration.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", from = "RawComments")]
pub struct Comments {
    /// `lineComment`, e.g. `//`.
    pub line_comment: Option<String>,
    /// `lineComment.noIndent`: line comments go at column 1, not at the
    /// line's indentation.
    pub line_comment_no_indent: bool,
    /// `blockComment` as `(open, close)`, e.g. `("/*", "*/")`.
    pub block_comment: Option<(String, String)>,
}

/// `lineComment` is either the token or `{ "comment": …, "noIndent": … }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawLineComment {
    Token(String),
    Object {
        comment: Option<String>,
        #[serde(default, rename = "noIndent")]
        no_indent: bool,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawComments {
    #[serde(default)]
    line_comment: Option<RawLineComment>,
    #[serde(default)]
    block_comment: Option<(String, String)>,
}

impl From<RawComments> for Comments {
    fn from(raw: RawComments) -> Self {
        let (line_comment, line_comment_no_indent) = match raw.line_comment {
            Some(RawLineComment::Token(token)) => (Some(token), false),
            Some(RawLineComment::Object { comment, no_indent }) => (comment, no_indent),
            None => (None, false),
        };
        Self {
            line_comment,
            line_comment_no_indent,
            block_comment: raw.block_comment,
        }
    }
}

/// Where an auto-closing pair is not inserted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotIn {
    /// Inside a string token.
    String,
    /// Inside a comment token.
    Comment,
    /// Inside a regular expression literal.
    Regex,
    /// Any other value; ignored.
    #[serde(other)]
    Other,
}

/// An `autoClosingPairs` entry, given as `["open", "close"]` or
/// `{ "open", "close", "notIn" }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoClosingPair {
    /// Text whose typing triggers auto-closing.
    pub open: String,
    /// Text inserted after the cursor.
    pub close: String,
    /// Contexts where the pair is not auto-closed (`notIn`).
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

/// A `surroundingPairs` entry, given as `["open", "close"]` or
/// `{ "open", "close" }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurroundingPair {
    /// Text inserted before the selection.
    pub open: String,
    /// Text inserted after the selection.
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
    /// The pattern source, as written in JavaScript.
    pub source: String,
    /// JavaScript regex flags. `i`, `m` and `s` are honored; others are ignored.
    pub flags: String,
    regex: Option<fancy_regex::Regex>,
}

impl Pattern {
    /// Compiles a JavaScript pattern with `flags`. A pattern that fails to compile
    /// is kept, but [`Pattern::regex`] returns `None` and it never matches.
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

/// The `indentationRules` section of a language configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndentationRules {
    /// `increaseIndentPattern`: a line matching this indents the lines after it.
    pub increase_indent_pattern: Pattern,
    /// `decreaseIndentPattern`: a line matching this is outdented.
    pub decrease_indent_pattern: Pattern,
    /// `indentNextLinePattern`: a line matching this indents only the next line.
    #[serde(default)]
    pub indent_next_line_pattern: Option<Pattern>,
    /// `unIndentedLinePattern`: a line matching this keeps its indentation and is
    /// ignored by the other rules.
    #[serde(default)]
    pub un_indented_line_pattern: Option<Pattern>,
}

/// What Enter does to indentation (VS Code `IndentAction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndentAction {
    /// Keep the previous line's indentation.
    None,
    /// Indent the new line once relative to the previous line.
    Indent,
    /// Insert two lines: an indented one for the cursor and one at the previous
    /// line's indentation (e.g. between brackets).
    IndentOutdent,
    /// Outdent the new line once relative to the previous line.
    Outdent,
}

/// The `action` of an [`OnEnterRule`].
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnterAction {
    /// `indent`: how to indent the new line.
    pub indent: IndentAction,
    /// `appendText`: text to insert after the new line's indentation.
    #[serde(default)]
    pub append_text: Option<String>,
    /// `removeText`: number of characters to remove from the new line's indentation.
    #[serde(default)]
    pub remove_text: Option<u32>,
}

/// An `onEnterRules` entry: applies [`EnterAction`] when Enter is pressed and
/// all of its patterns match.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnEnterRule {
    /// `beforeText`: must match the line's text before the cursor.
    pub before_text: Pattern,
    /// `afterText`: must match the line's text after the cursor.
    #[serde(default)]
    pub after_text: Option<Pattern>,
    /// `previousLineText`: must match the line above the cursor's line.
    #[serde(default)]
    pub previous_line_text: Option<Pattern>,
    /// `action`: what to do.
    pub action: EnterAction,
}

/// The `folding` section of a language configuration.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folding {
    /// `offSide`: the language uses the off-side rule (e.g. Python), so blank
    /// lines below an indented block are not folded with it.
    #[serde(default)]
    pub off_side: bool,
    /// `markers`: patterns for explicit region markers such as `#region`.
    #[serde(default)]
    pub markers: Option<FoldingMarkers>,
}

/// The `folding.markers` patterns.
#[derive(Clone, Debug, Deserialize)]
pub struct FoldingMarkers {
    /// `start`: a line matching this opens a folding region.
    pub start: Pattern,
    /// `end`: a line matching this closes the region opened by the nearest `start`.
    pub end: Pattern,
}

/// Removes `//` and `/* */` comments and trailing commas outside strings,
/// turning JSONC (VS Code's configuration format) into JSON.
pub fn strip_jsonc(json: &str) -> String {
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
