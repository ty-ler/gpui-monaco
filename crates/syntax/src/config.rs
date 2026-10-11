//! Per-language editing rules in VS Code's `language-configuration.json`
//! format, so VS Code's (MIT-licensed) configurations can be reused as-is.
//!
//! Patterns are JavaScript regular expressions; they are compiled with
//! `fancy-regex`, which supports the lookarounds these files commonly use.

use serde::Deserialize;

/// A parsed `language-configuration.json` (Monaco `LanguageConfiguration`).
/// Unknown keys are ignored. Every field is optional, as in Monaco: `None`
/// is "not set", so a configuration can be layered over another with
/// [`LanguageConfiguration::combine`], and an explicit empty list (such as
/// `"autoClosingPairs": []`) differs from a missing one.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageConfiguration {
    /// `comments`: line and block comment tokens; see
    /// [`LanguageConfiguration::comments`].
    #[serde(default)]
    pub comments: Option<Comments>,
    /// `brackets`: open/close pairs used for bracket matching and
    /// indentation; see [`LanguageConfiguration::brackets`].
    #[serde(default)]
    pub brackets: Option<Vec<(String, String)>>,
    /// `colorizedBracketPairs`: the pairs bracket pair colorization uses;
    /// `None` means [`LanguageConfiguration::brackets`].
    #[serde(default)]
    pub colorized_bracket_pairs: Option<Vec<(String, String)>>,
    /// `autoClosingPairs`: pairs closed automatically when the open text is
    /// typed. `None` uses the brackets; an empty list closes nothing.
    #[serde(default)]
    pub auto_closing_pairs: Option<Vec<AutoClosingPair>>,
    /// Characters before which auto-closing happens (Monaco `autoCloseBefore`).
    #[serde(default)]
    pub auto_close_before: Option<String>,
    /// `surroundingPairs`: pairs that wrap the selection when the open text
    /// is typed. `None` uses the auto-closing pairs.
    #[serde(default)]
    pub surrounding_pairs: Option<Vec<SurroundingPair>>,
    /// `wordPattern`: what counts as a word, for word navigation and selection.
    #[serde(default)]
    pub word_pattern: Option<Pattern>,
    /// `indentationRules`: how typing and Enter adjust indentation.
    #[serde(default)]
    pub indentation_rules: Option<IndentationRules>,
    /// `onEnterRules`: special Enter handling, such as continuing doc
    /// comments; see [`LanguageConfiguration::on_enter_rules`].
    #[serde(default)]
    pub on_enter_rules: Option<Vec<OnEnterRule>>,
    /// `folding`: indentation-based folding settings and region markers.
    #[serde(default)]
    pub folding: Option<Folding>,
    /// Monaco's deprecated `__electricCharacterSupport`: its `docComment`
    /// is auto-closed like an auto-closing pair.
    #[serde(default, rename = "__electricCharacterSupport")]
    pub electric_character_support: Option<ElectricCharacterSupport>,
}

/// Monaco's deprecated `__electricCharacterSupport`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElectricCharacterSupport {
    /// A doc comment's delimiters (Monaco `IDocComment`), such as `/**` and
    /// ` */`.
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
}

/// Monaco `IDocComment`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct DocComment {
    /// The text that opens it, e.g. `/**`.
    pub open: String,
    /// The text that closes it, e.g. ` */`.
    #[serde(default)]
    pub close: Option<String>,
}

/// The comments of a configuration without `comments`.
static NO_COMMENTS: Comments = Comments {
    line_comment: None,
    line_comment_no_indent: false,
    block_comment: None,
};

impl LanguageConfiguration {
    /// [`comments`](Self::comments), or none.
    pub fn comments(&self) -> &Comments {
        self.comments.as_ref().unwrap_or(&NO_COMMENTS)
    }

    /// [`brackets`](Self::brackets), or none.
    pub fn brackets(&self) -> &[(String, String)] {
        self.brackets.as_deref().unwrap_or_default()
    }

    /// [`auto_closing_pairs`](Self::auto_closing_pairs) as given, or none.
    pub fn auto_closing_pairs(&self) -> &[AutoClosingPair] {
        self.auto_closing_pairs.as_deref().unwrap_or_default()
    }

    /// [`surrounding_pairs`](Self::surrounding_pairs) as given, or none.
    pub fn surrounding_pairs(&self) -> &[SurroundingPair] {
        self.surrounding_pairs.as_deref().unwrap_or_default()
    }

    /// [`on_enter_rules`](Self::on_enter_rules), or none.
    pub fn on_enter_rules(&self) -> &[OnEnterRule] {
        self.on_enter_rules.as_deref().unwrap_or_default()
    }

    /// `self` with every field `newer` sets taken from it (Monaco
    /// `combineLanguageConfigurations`, which takes each field from the
    /// newest configuration defining it). An empty `autoCloseBefore`
    /// counts as unset, as in Monaco.
    pub fn combine(&self, newer: &LanguageConfiguration) -> LanguageConfiguration {
        fn pick<T: Clone>(newer: &Option<T>, older: &Option<T>) -> Option<T> {
            newer.clone().or_else(|| older.clone())
        }
        LanguageConfiguration {
            comments: pick(&newer.comments, &self.comments),
            brackets: pick(&newer.brackets, &self.brackets),
            colorized_bracket_pairs: pick(
                &newer.colorized_bracket_pairs,
                &self.colorized_bracket_pairs,
            ),
            auto_closing_pairs: pick(&newer.auto_closing_pairs, &self.auto_closing_pairs),
            auto_close_before: newer
                .auto_close_before
                .clone()
                .filter(|before| !before.is_empty())
                .or_else(|| self.auto_close_before.clone()),
            surrounding_pairs: pick(&newer.surrounding_pairs, &self.surrounding_pairs),
            word_pattern: pick(&newer.word_pattern, &self.word_pattern),
            indentation_rules: pick(&newer.indentation_rules, &self.indentation_rules),
            on_enter_rules: pick(&newer.on_enter_rules, &self.on_enter_rules),
            folding: pick(&newer.folding, &self.folding),
            electric_character_support: pick(
                &newer.electric_character_support,
                &self.electric_character_support,
            ),
        }
    }

    /// Every regex in the configuration, named by its JSON key (e.g. to
    /// report ones that didn't compile, see [`Pattern::error`]).
    pub fn patterns(&self) -> Vec<(&'static str, &Pattern)> {
        let mut patterns = Vec::new();
        if let Some(word) = &self.word_pattern {
            patterns.push(("wordPattern", word));
        }
        if let Some(rules) = &self.indentation_rules {
            patterns.push(("increaseIndentPattern", &rules.increase_indent_pattern));
            patterns.push(("decreaseIndentPattern", &rules.decrease_indent_pattern));
            if let Some(p) = &rules.indent_next_line_pattern {
                patterns.push(("indentNextLinePattern", p));
            }
            if let Some(p) = &rules.un_indented_line_pattern {
                patterns.push(("unIndentedLinePattern", p));
            }
        }
        for rule in self.on_enter_rules() {
            patterns.push(("onEnterRules.beforeText", &rule.before_text));
            if let Some(p) = &rule.after_text {
                patterns.push(("onEnterRules.afterText", p));
            }
            if let Some(p) = &rule.previous_line_text {
                patterns.push(("onEnterRules.previousLineText", p));
            }
        }
        if let Some(markers) = self.folding.as_ref().and_then(|f| f.markers.as_ref()) {
            patterns.push(("folding.markers.start", &markers.start));
            patterns.push(("folding.markers.end", &markers.end));
        }
        patterns
    }

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
    /// JavaScript regex flags. `i`, `m` and `s` are honored; `g`, `y`, `d`,
    /// `u` and `v` change nothing here (matching is always Unicode-aware).
    pub flags: String,
    regex: Option<fancy_regex::Regex>,
    error: Option<String>,
}

impl Pattern {
    /// Compiles a JavaScript pattern with `flags`. A pattern that fails to
    /// compile is kept, but [`Pattern::regex`] returns `None`, it never
    /// matches, and [`Pattern::error`] says why.
    pub fn new(source: impl Into<String>, flags: impl Into<String>) -> Self {
        let source = source.into();
        let flags = flags.into();
        let mut prefix = String::new();
        let mut error = None;
        for flag in flags.chars() {
            match flag {
                'i' | 'm' | 's' => {
                    prefix.push_str("(?");
                    prefix.push(flag);
                    prefix.push(')');
                }
                'g' | 'y' | 'd' | 'u' | 'v' => {}
                _ => error = Some(format!("unsupported regex flag `{flag}`")),
            }
        }
        let regex = match error {
            Some(_) => None,
            None => match fancy_regex::Regex::new(&format!("{prefix}{}", translate_js(&source))) {
                Ok(regex) => Some(regex),
                Err(err) => {
                    let mut message = err.to_string();
                    let mut source = std::error::Error::source(&err);
                    while let Some(inner) = source {
                        message = format!("{message}: {inner}");
                        source = inner.source();
                    }
                    error = Some(message);
                    None
                }
            },
        };
        Self {
            source,
            flags,
            regex,
            error,
        }
    }

    /// The compiled regex, or `None` if the JavaScript pattern uses syntax
    /// `fancy-regex` cannot compile.
    pub fn regex(&self) -> Option<&fancy_regex::Regex> {
        self.regex.as_ref()
    }

    /// Why the pattern didn't compile, if it didn't.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the pattern matches somewhere in `text`. Patterns that failed to
    /// compile never match.
    pub fn is_match(&self, text: &str) -> bool {
        self.regex
            .as_ref()
            .is_some_and(|regex| regex.is_match(text).unwrap_or(false))
    }
}

/// Whether `chars` continues with a class escape such as `\d`.
fn is_class_escape_next(chars: &std::iter::Peekable<std::str::Chars>) -> bool {
    let mut ahead = chars.clone();
    ahead.next() == Some('\\') && matches!(ahead.next(), Some('w' | 'W' | 'd' | 'D' | 's' | 'S'))
}

/// Rewrites JavaScript regex syntax that means something else (or nothing)
/// to `fancy-regex`: `[^]` (any character) and `[]` (nothing), and, inside
/// character classes, a literal `[`, hyphens next to class escapes, and the
/// `&&`, `--` and `~~` that Rust reads as set operations.
fn translate_js(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push(c);
                let Some(next) = chars.next() else {
                    continue;
                };
                out.push(next);
                // JavaScript (Annex B) reads `-` after a class escape, as in
                // `[\w-.]`, as a literal hyphen; Rust rejects the range.
                let class_escape = matches!(next, 'w' | 'W' | 'd' | 'D' | 's' | 'S');
                if in_class && class_escape && chars.peek() == Some(&'-') {
                    chars.next();
                    out.push_str(r"\-");
                }
            }
            '-' if in_class && is_class_escape_next(&chars) => out.push_str(r"\-"),
            '[' if !in_class => {
                let rest: String = chars.clone().take(2).collect();
                if rest.starts_with("^]") {
                    out.push_str(r"[\s\S]");
                    chars.nth(1);
                } else if rest.starts_with(']') {
                    out.push_str(r"[^\s\S]");
                    chars.next();
                } else {
                    in_class = true;
                    out.push(c);
                    // Keep a negation; `[^]` and `[]` were handled above.
                    if chars.peek() == Some(&'^') {
                        out.push('^');
                        chars.next();
                    }
                }
            }
            '[' => out.push_str(r"\["),
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            '&' | '-' | '~' if in_class && chars.peek() == Some(&c) => {
                chars.next();
                out.push('\\');
                out.push(c);
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
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
    fn javascript_patterns() {
        // Flags: `g` and `u` change nothing; unknown ones are reported.
        let p = Pattern::new("ab", "giu");
        assert!(p.is_match("xABy") && p.error().is_none());
        assert!(Pattern::new("ab", "q").error().is_some());
        // `[^]` is any character, `[]` none.
        assert!(Pattern::new("a[^]b", "").is_match("a\nb"));
        assert!(!Pattern::new("a[]", "").is_match("a"));
        // A literal `[` and `&&` inside a class.
        let class = Pattern::new("^[[&&]+$", "");
        assert!(class.is_match("[&&[") && class.error().is_none());
        assert!(!class.is_match("a"));
        // A hyphen next to a class escape is literal (Annex B).
        let word = Pattern::new(r"^[_:\w-.\d]+$", "");
        assert!(word.error().is_none(), "{:?}", word.error());
        assert!(word.is_match("a-b.c_1") && !word.is_match("a b"));
        assert!(Pattern::new(r"^[\w-?]+$", "").is_match("a-?"));
        assert!(Pattern::new(r"^[.-\d]+$", "").is_match("-.1"));
        // Escapes are kept as written.
        assert!(Pattern::new(r"\[\]", "").is_match("[]"));
        // Unsupported syntax reports why.
        let bad = Pattern::new("(?<=a+", "");
        assert!(bad.regex().is_none() && bad.error().is_some());
    }

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
        assert_eq!(config.comments().line_comment.as_deref(), Some("//"));
        assert_eq!(config.brackets().len(), 2);
        assert_eq!(
            config.auto_closing_pairs()[2].not_in,
            vec![NotIn::String, NotIn::Comment]
        );
        assert_eq!(config.surrounding_pairs()[1].open, "'");
        let rules = config.indentation_rules.clone().unwrap();
        assert!(rules.increase_indent_pattern.is_match("fn main() {"));
        assert!(rules.decrease_indent_pattern.is_match("    }"));
        // Lookahead from a real VS Code rule compiles.
        assert!(
            config.on_enter_rules()[0]
                .before_text
                .is_match("  /** docs")
        );
        assert!(
            !config.on_enter_rules()[0]
                .before_text
                .is_match("  /** docs */")
        );
    }

    #[test]
    fn missing_and_empty_fields_differ() {
        let config = LanguageConfiguration::from_json(
            r#"{
                "autoClosingPairs": [],
                "__electricCharacterSupport": { "docComment": { "open": "/**", "close": " */" } }
            }"#,
        )
        .unwrap();
        assert_eq!(config.auto_closing_pairs.as_deref(), Some(&[][..]));
        assert!(config.brackets.is_none() && config.comments.is_none());
        assert_eq!(
            config
                .electric_character_support
                .and_then(|support| support.doc_comment),
            Some(DocComment {
                open: "/**".into(),
                close: Some(" */".into()),
            })
        );
    }

    /// Monaco `combineLanguageConfigurations`.
    #[test]
    fn combining_takes_the_newest_field() {
        let base = LanguageConfiguration::from_json(
            r##"{ "brackets": [["(", ")"]], "comments": { "lineComment": "#" }, "autoCloseBefore": ";" }"##,
        )
        .unwrap();
        let newer = LanguageConfiguration::from_json(
            r#"{ "comments": { "lineComment": "//" }, "autoCloseBefore": "" }"#,
        )
        .unwrap();
        let combined = base.combine(&newer);
        assert_eq!(combined.comments().line_comment.as_deref(), Some("//"));
        assert_eq!(combined.brackets(), [("(".to_string(), ")".to_string())]);
        assert_eq!(combined.auto_close_before.as_deref(), Some(";"));
    }
}
