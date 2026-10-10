//! Port of Monaco's TextMate snippet parser,
//! `vs/editor/contrib/snippet/browser/snippetParser.ts` (MIT, Copyright (c)
//! Microsoft Corporation).
//!
//! Monaco builds a tree of `Marker` objects with parent pointers and mutates
//! it in place. Here the tree lives in an arena owned by [`TextmateSnippet`]:
//! nodes are addressed by [`MarkerId`], each knows its parent and children, and
//! the tree operations Monaco puts on markers (`toString`, `len`, `children`,
//! `choice`, `clone`, ...) are methods on the snippet that take a [`MarkerId`].
//! Nodes are never freed; markers dropped by [`TextmateSnippet::replace`] or a
//! failed parse stay in the arena, detached, like unreachable JS objects.
//!
//! [`Transform`] and [`FormatString`] are plain values owned by the
//! [`Placeholder`] or [`Variable`] they transform rather than arena nodes:
//! Monaco never walks into them, and they never need offsets or parents.
//! [`Choice`] options are likewise kept in the choice, not as children.
//!
//! Units: [`Marker::len`], [`TextmateSnippet::offset`] and
//! [`TextmateSnippet::full_len`] count UTF-16 code units, like the JS string
//! lengths Monaco uses, so they line up with the editor's UTF-16 columns.
//! [`Scanner`] and [`Token`] positions are byte offsets into the source (all
//! token delimiters are ASCII, so they are always char boundaries).
//!
//! Differences from Monaco:
//! - Transform regexes compile with [`syntax::Pattern`] (JS syntax translated
//!   to `fancy-regex`), so exotic JS regex syntax may be rejected (making the
//!   transform plain text, as an invalid JS regex does) or match slightly
//!   differently; a global replace steps past empty matches by a char, not a
//!   UTF-16 unit. Named groups don't shift the format-string group indices
//!   (in JS they make `replace` pass an extra argument).
//! - Case shorthands use Rust's locale-independent case mapping where Monaco
//!   calls `toLocaleUpperCase`/`toLocaleLowerCase`.
//! - [`TextmateSnippet::offset`] returns `None` where Monaco returns `-1`.
//! - `Clone` is exact; Monaco's `Transform.clone` keeps only the `i` and `g`
//!   flags (its `toTextmateString` still does, and so does ours).
//! - The cached `placeholderInfo` is recomputed on every call instead.
//! - `Marker.snippet` is not ported: every marker belongs to its arena's
//!   snippet; [`TextmateSnippet::parent`] tells whether it is attached.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::LazyLock;

use syntax::Pattern;
use syntax::fancy_regex;

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

/// The kind of a [`Token`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenType {
    /// `$`
    Dollar,
    /// `:`
    Colon,
    /// `,`
    Comma,
    /// `{`
    CurlyOpen,
    /// `}`
    CurlyClose,
    /// `\`
    Backslash,
    /// `/`
    Forwardslash,
    /// `|`
    Pipe,
    /// A run of ASCII digits.
    Int,
    /// `[_a-zA-Z][_a-zA-Z0-9]*`
    VariableName,
    /// A run of any other characters.
    Format,
    /// `+`
    Plus,
    /// `-`
    Dash,
    /// `?`
    QuestionMark,
    /// End of input.
    Eof,
}

/// A token produced by [`Scanner::next`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    /// What kind of token this is (Monaco's `type`).
    pub kind: TokenType,
    /// Byte offset of the token in the scanned text.
    pub pos: usize,
    /// Length of the token in bytes; 0 only for [`TokenType::Eof`].
    pub len: usize,
}

/// Splits snippet source into [`Token`]s.
#[derive(Clone, Debug, Default)]
pub struct Scanner {
    /// The text being scanned.
    pub value: String,
    /// Byte offset of the next token.
    pub pos: usize,
}

impl Scanner {
    /// A scanner over the empty string.
    pub fn new() -> Self {
        Self::default()
    }

    /// The single-character token `ch` starts, if any.
    fn static_token(ch: u8) -> Option<TokenType> {
        Some(match ch {
            b'$' => TokenType::Dollar,
            b':' => TokenType::Colon,
            b',' => TokenType::Comma,
            b'{' => TokenType::CurlyOpen,
            b'}' => TokenType::CurlyClose,
            b'\\' => TokenType::Backslash,
            b'/' => TokenType::Forwardslash,
            b'|' => TokenType::Pipe,
            b'+' => TokenType::Plus,
            b'-' => TokenType::Dash,
            b'?' => TokenType::QuestionMark,
            _ => return None,
        })
    }

    /// Whether `ch` is an ASCII digit.
    pub fn is_digit_character(ch: u8) -> bool {
        ch.is_ascii_digit()
    }

    /// Whether `ch` can start a variable name: `_` or an ASCII letter.
    pub fn is_variable_character(ch: u8) -> bool {
        ch == b'_' || ch.is_ascii_alphabetic()
    }

    /// Restarts scanning at the beginning of `value`.
    pub fn text(&mut self, value: &str) {
        self.value = value.to_string();
        self.pos = 0;
    }

    /// The source text of `token`.
    pub fn token_text(&self, token: &Token) -> &str {
        &self.value[token.pos..token.pos + token.len]
    }

    /// Scans the next token; returns [`TokenType::Eof`] forever at the end.
    #[allow(
        clippy::should_implement_trait,
        reason = "Monaco's name; the token stream never ends, so it isn't an Iterator"
    )]
    pub fn next(&mut self) -> Token {
        let bytes = self.value.as_bytes();
        let pos = self.pos;
        if pos >= bytes.len() {
            return Token {
                kind: TokenType::Eof,
                pos,
                len: 0,
            };
        }
        let at = |i: usize| bytes.get(i).copied();
        let ch = bytes[pos];

        let (kind, len) = if let Some(kind) = Self::static_token(ch) {
            (kind, 1)
        } else if Self::is_digit_character(ch) {
            let mut len = 1;
            while at(pos + len).is_some_and(Self::is_digit_character) {
                len += 1;
            }
            (TokenType::Int, len)
        } else if Self::is_variable_character(ch) {
            let mut len = 1;
            while at(pos + len)
                .is_some_and(|c| Self::is_variable_character(c) || Self::is_digit_character(c))
            {
                len += 1;
            }
            (TokenType::VariableName, len)
        } else {
            // Non-ASCII bytes all land here, so a format token never splits a
            // UTF-8 sequence.
            let mut len = 1;
            while at(pos + len).is_some_and(|c| {
                Self::static_token(c).is_none()
                    && !Self::is_digit_character(c)
                    && !Self::is_variable_character(c)
            }) {
                len += 1;
            }
            (TokenType::Format, len)
        };
        self.pos += len;
        Token { kind, pos, len }
    }
}

// ---------------------------------------------------------------------------
// Markers
// ---------------------------------------------------------------------------

/// Index of a marker in a [`TextmateSnippet`]'s arena.
///
/// Ids are only meaningful for the snippet that created them; methods that
/// take one panic if it is out of range for that snippet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MarkerId(usize);

/// UTF-16 length of `value` (JS `string.length`).
fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

/// Backslash-escapes every char of `value` that is in `special`.
fn escape_chars(value: &str, special: &[char]) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if special.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Literal text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    /// The text.
    pub value: String,
}

impl Text {
    /// Text with `value`.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    /// Escapes `$`, `}` and `\` so `value` reads back as plain text.
    pub fn escape(value: &str) -> String {
        escape_chars(value, &['$', '}', '\\'])
    }

    /// Length in UTF-16 code units.
    #[allow(
        clippy::len_without_is_empty,
        reason = "Monaco's `Marker.len()`, used for offsets"
    )]
    pub fn len(&self) -> usize {
        utf16_len(&self.value)
    }

    /// The text in snippet syntax (escaped).
    pub fn to_textmate_string(&self) -> String {
        Self::escape(&self.value)
    }
}

/// A tabstop or placeholder: `$1`, `${1}`, `${1:default}`, `${1|a,b|}`,
/// `${1/regex/format/flags}`. Its children (in the snippet) are its default
/// value.
#[derive(Clone, Debug)]
pub struct Placeholder {
    /// The tabstop index; 0 is the final tabstop.
    pub index: usize,
    /// The transform applied to the placeholder's value, if any.
    pub transform: Option<Transform>,
}

impl Placeholder {
    /// A placeholder for tabstop `index` with no transform.
    pub fn new(index: usize) -> Self {
        Self {
            index,
            transform: None,
        }
    }

    /// Orders placeholders by index, with the final tabstop (`$0`) last.
    pub fn compare_by_index(a: &Placeholder, b: &Placeholder) -> Ordering {
        if a.index == b.index {
            Ordering::Equal
        } else if a.is_final_tabstop() {
            Ordering::Greater
        } else if b.is_final_tabstop() {
            Ordering::Less
        } else {
            a.index.cmp(&b.index)
        }
    }

    /// Whether this is `$0`, where the cursor ends up.
    pub fn is_final_tabstop(&self) -> bool {
        self.index == 0
    }
}

/// The choices of `${1|one,two|}`; the first option is the inserted text.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    /// The options, unescaped.
    pub options: Vec<Text>,
}

impl Choice {
    /// Length of the first option in UTF-16 code units.
    #[allow(
        clippy::len_without_is_empty,
        reason = "Monaco's `Marker.len()`, used for offsets"
    )]
    pub fn len(&self) -> usize {
        self.options.first().map_or(0, Text::len)
    }

    /// The options in snippet syntax: `,`, `|` and `\` escaped, comma-joined.
    pub fn to_textmate_string(&self) -> String {
        self.options
            .iter()
            .map(|option| escape_chars(&option.value, &['|', ',', '\\']))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// A variable: `$name`, `${name}`, `${name:default}`,
/// `${name/regex/format/flags}`. Its children (in the snippet) are its default
/// value, or its resolved value once resolved.
#[derive(Clone, Debug)]
pub struct Variable {
    /// The variable name, e.g. `TM_SELECTED_TEXT`.
    pub name: String,
    /// The transform applied to the variable's value, if any.
    pub transform: Option<Transform>,
}

impl Variable {
    /// A variable called `name` with no transform.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transform: None,
        }
    }

    /// The variable's value: `resolver`'s value, run through the transform if
    /// there is one. With a transform this is always `Some` (an unknown
    /// variable transforms the empty string), as in Monaco.
    ///
    /// [`TextmateSnippet::resolve_variable`] stores it in the tree.
    pub fn resolve(&self, resolver: &dyn VariableResolver) -> Option<String> {
        let mut value = resolver.resolve(self);
        if let Some(transform) = &self.transform {
            value = Some(transform.resolve(value.as_deref().unwrap_or("")));
        }
        value
    }
}

/// Provides values for snippet variables.
pub trait VariableResolver {
    /// The value of `variable`, or `None` if it is unknown.
    fn resolve(&self, variable: &Variable) -> Option<String>;
}

impl<F: Fn(&Variable) -> Option<String>> VariableResolver for F {
    fn resolve(&self, variable: &Variable) -> Option<String> {
        self(variable)
    }
}

/// A part of a [`Transform`]'s format: literal text or a group reference.
#[derive(Clone, Debug)]
pub enum TransformPart {
    /// Literal text.
    Text(Text),
    /// A capture group reference such as `$1` or `${1:/upcase}`.
    FormatString(FormatString),
}

/// `/regex/format/flags`: replaces matches of `regexp` in a value with the
/// format.
#[derive(Clone, Debug)]
pub struct Transform {
    /// The regex, with its JavaScript flags. Only `g` (replace all matches) and
    /// `i` survive [`Transform::to_textmate_string`], as in Monaco.
    pub regexp: Pattern,
    /// The format, in order.
    pub children: Vec<TransformPart>,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            regexp: Pattern::new("", ""),
            children: Vec::new(),
        }
    }
}

impl Transform {
    /// A transform with an empty regex and format.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a format part, merging adjacent text like `Marker.appendChild`.
    pub fn append_child(&mut self, child: TransformPart) {
        if let (TransformPart::Text(text), Some(TransformPart::Text(last))) =
            (&child, self.children.last_mut())
        {
            last.value.push_str(&text.value);
        } else {
            self.children.push(child);
        }
    }

    fn is_global(&self) -> bool {
        self.regexp.flags.contains('g')
    }

    fn ignore_case(&self) -> bool {
        self.regexp.flags.contains('i')
    }

    /// Applies the transform to `value`: like JS `value.replace(regexp, ...)`,
    /// replacing the first match (every match with the `g` flag) with the
    /// format. If nothing matches and the format has an else branch, the
    /// result is the format resolved with no groups.
    pub fn resolve(&self, value: &str) -> String {
        let mut did_match = false;
        let mut ret = String::new();
        let mut last = 0;
        if let Some(regex) = self.regexp.regex() {
            // A match error (backtrack limit) ends the replacing, like no match.
            for captures in regex.captures_iter(value).map_while(Result::ok) {
                let Some(whole) = captures.get(0) else {
                    break;
                };
                did_match = true;
                ret.push_str(&value[last..whole.start()]);
                let groups: Vec<Option<&str>> = (0..captures.len())
                    .map(|i| captures.get(i).map(|m| m.as_str()))
                    .collect();
                ret.push_str(&self.replace(&groups));
                last = whole.end();
                if !self.is_global() {
                    break;
                }
            }
        }
        ret.push_str(&value[last..]);
        let has_else = self.children.iter().any(|child| {
            matches!(child, TransformPart::FormatString(format)
                if format.else_value.as_deref().is_some_and(|v| !v.is_empty()))
        });
        if !did_match && has_else {
            ret = self.replace(&[]);
        }
        ret
    }

    /// The format with `groups` (match first, then capture groups) filled in.
    fn replace(&self, groups: &[Option<&str>]) -> String {
        let mut ret = String::new();
        for child in &self.children {
            match child {
                TransformPart::FormatString(format) => {
                    let group = groups.get(format.index).copied().flatten().unwrap_or("");
                    ret.push_str(&format.resolve(Some(group)));
                }
                TransformPart::Text(text) => ret.push_str(&text.value),
            }
        }
        ret
    }

    /// The transform in snippet syntax: `/regex/format/flags`.
    pub fn to_textmate_string(&self) -> String {
        let mut ret = format!("/{}/", regexp_source(&self.regexp.source));
        for child in &self.children {
            match child {
                TransformPart::Text(text) => ret.push_str(&text.to_textmate_string()),
                TransformPart::FormatString(format) => ret.push_str(&format.to_textmate_string()),
            }
        }
        ret.push('/');
        if self.ignore_case() {
            ret.push('i');
        }
        if self.is_global() {
            ret.push('g');
        }
        ret
    }
}

/// What JS `RegExp.prototype.source` returns for `source`: `(?:)` when empty,
/// `/` outside classes and line terminators escaped.
fn regexp_source(source: &str) -> String {
    if source.is_empty() {
        return "(?:)".to_string();
    }
    let mut out = String::with_capacity(source.len());
    let mut in_class = false;
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push(c);
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '[' => {
                in_class = true;
                out.push(c);
            }
            ']' => {
                in_class = false;
                out.push(c);
            }
            '/' if !in_class => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

/// A capture group reference in a [`Transform`]'s format: `$1`, `${1}`,
/// `${1:/upcase}`, `${1:+if}`, `${1:-else}`, `${1:else}`, `${1:?if:else}`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatString {
    /// The capture group; 0 is the whole match.
    pub index: usize,
    /// A case shorthand: `upcase`, `downcase`, `capitalize`, `pascalcase`,
    /// `camelcase`, `kebabcase` or `snakecase` (others leave the value as is).
    pub shorthand_name: Option<String>,
    /// Inserted instead of the group when it matched something.
    pub if_value: Option<String>,
    /// Inserted instead of the group when it matched nothing.
    pub else_value: Option<String>,
}

static WORDS: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"[\p{L}0-9]+").expect("valid regex"));
static KEBAB_WORDS: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(
        r"\p{Lu}{2,}(?=\p{Lu}\p{Ll}+[0-9]*|[\s_-]|$)|\p{Lu}?\p{Ll}+[0-9]*|\p{Lu}(?=\p{Lu}\p{Ll})|\p{Lu}(?=[\s_-]|$)|[0-9]+",
    )
    .expect("valid regex")
});
static LOWER_UPPER: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(\p{Ll})(\p{Lu})").expect("valid regex"));
static SPACES_DASHES: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"[\s\-]+").expect("valid regex"));

/// `word` with its first char mapped by `first`.
fn map_first(word: &str, first: impl Fn(char) -> String) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => first(c) + chars.as_str(),
        None => String::new(),
    }
}

impl FormatString {
    /// A format string; mirrors Monaco's constructor.
    pub fn new(
        index: usize,
        shorthand_name: Option<&str>,
        if_value: Option<&str>,
        else_value: Option<&str>,
    ) -> Self {
        Self {
            index,
            shorthand_name: shorthand_name.map(str::to_string),
            if_value: if_value.map(str::to_string),
            else_value: else_value.map(str::to_string),
        }
    }

    /// What to insert for a group whose text is `value` (`None` or empty when
    /// the group didn't participate).
    pub fn resolve(&self, value: Option<&str>) -> String {
        // JS truthiness: the empty string counts as no value.
        let value = value.filter(|v| !v.is_empty());
        let shorthand = |f: fn(&str) -> String| value.map(f).unwrap_or_default();
        match self.shorthand_name.as_deref() {
            Some("upcase") => shorthand(str::to_uppercase),
            Some("downcase") => shorthand(str::to_lowercase),
            Some("capitalize") => shorthand(|v| map_first(v, |c| c.to_uppercase().collect())),
            Some("pascalcase") => shorthand(Self::to_pascal_case),
            Some("camelcase") => shorthand(Self::to_camel_case),
            Some("kebabcase") => shorthand(Self::to_kebab_case),
            Some("snakecase") => shorthand(Self::to_snake_case),
            _ => match (value, &self.if_value, &self.else_value) {
                (Some(_), Some(if_value), _) => if_value.clone(),
                (None, _, Some(else_value)) => else_value.clone(),
                _ => value.unwrap_or_default().to_string(),
            },
        }
    }

    // Word-based case transforms rely on upper/lowercase distinctions; for
    // scripts without case they are effectively no-ops.
    fn to_kebab_case(value: &str) -> String {
        if !WORDS.is_match(value) {
            return value.to_string();
        }
        // Monaco next checks for a single letter or digit, which can't fail
        // once `WORDS` matched, so that branch is omitted.
        let cleaned = value.trim().trim_start_matches('_').trim_end_matches('_');
        let words: Vec<&str> = KEBAB_WORDS
            .find_iter(cleaned)
            .map_while(Result::ok)
            .map(|m| m.as_str())
            .collect();
        if words.is_empty() {
            return cleaned
                .split(|c: char| c.is_whitespace() || c == '_' || c == '-')
                .filter(|word| !word.is_empty())
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
                .join("-");
        }
        words
            .iter()
            .map(|word| word.to_lowercase())
            .collect::<Vec<_>>()
            .join("-")
    }

    fn to_pascal_case(value: &str) -> String {
        if !WORDS.is_match(value) {
            return value.to_string();
        }
        WORDS
            .find_iter(value)
            .map(|word| map_first(word.as_str(), |c| c.to_uppercase().collect()))
            .collect()
    }

    fn to_camel_case(value: &str) -> String {
        if !WORDS.is_match(value) {
            return value.to_string();
        }
        WORDS
            .find_iter(value)
            .enumerate()
            .map(|(index, word)| {
                if index == 0 {
                    map_first(word.as_str(), |c| c.to_lowercase().collect())
                } else {
                    map_first(word.as_str(), |c| c.to_uppercase().collect())
                }
            })
            .collect()
    }

    fn to_snake_case(value: &str) -> String {
        let split = LOWER_UPPER.replace_all(value, "${1}_${2}");
        SPACES_DASHES.replace_all(&split, "_").to_lowercase()
    }

    /// The format string in snippet syntax, always in `${...}` form.
    pub fn to_textmate_string(&self) -> String {
        let non_empty = |v: &Option<String>| v.clone().filter(|v| !v.is_empty());
        let mut value = format!("${{{}", self.index);
        if let Some(shorthand) = non_empty(&self.shorthand_name) {
            value.push_str(&format!(":/{shorthand}"));
        } else {
            match (non_empty(&self.if_value), non_empty(&self.else_value)) {
                (Some(if_value), Some(else_value)) => {
                    value.push_str(&format!(":?{if_value}:{else_value}"))
                }
                (Some(if_value), None) => value.push_str(&format!(":+{if_value}")),
                (None, Some(else_value)) => value.push_str(&format!(":-{else_value}")),
                (None, None) => {}
            }
        }
        value.push('}');
        value
    }
}

/// A node of a [`TextmateSnippet`] (Monaco's `Marker` subclasses).
#[derive(Clone, Debug)]
pub enum Marker {
    /// Literal text; has no children.
    Text(Text),
    /// A tabstop or placeholder; children are its default value.
    Placeholder(Placeholder),
    /// The choices of a placeholder; has no children.
    Choice(Choice),
    /// A variable; children are its default or resolved value.
    Variable(Variable),
    /// The root of a snippet.
    TextmateSnippet,
}

impl Marker {
    /// The marker's own length in UTF-16 code units: the text of a
    /// [`Marker::Text`], the first option of a [`Marker::Choice`], else 0
    /// (container text is counted by its children).
    #[allow(
        clippy::len_without_is_empty,
        reason = "Monaco's `Marker.len()`, used for offsets"
    )]
    pub fn len(&self) -> usize {
        match self {
            Marker::Text(text) => text.len(),
            Marker::Choice(choice) => choice.len(),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    marker: Marker,
    parent: Option<MarkerId>,
    children: Vec<MarkerId>,
}

/// [`TextmateSnippet::placeholder_info`]'s result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaceholderInfo {
    /// Every placeholder, in document (pre-order) order.
    pub all: Vec<MarkerId>,
    /// The first placeholder with the highest index.
    pub last: Option<MarkerId>,
}

/// A parsed snippet: the arena of markers and its root.
#[derive(Clone, Debug)]
pub struct TextmateSnippet {
    nodes: Vec<Node>,
}

impl Default for TextmateSnippet {
    fn default() -> Self {
        Self::new()
    }
}

const ROOT: MarkerId = MarkerId(0);

impl TextmateSnippet {
    /// An empty snippet.
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                marker: Marker::TextmateSnippet,
                parent: None,
                children: Vec::new(),
            }],
        }
    }

    /// The root marker; its children are the snippet's top-level markers.
    pub fn root(&self) -> MarkerId {
        ROOT
    }

    /// Adds `marker` to the arena, detached and childless.
    pub fn create(&mut self, marker: Marker) -> MarkerId {
        self.nodes.push(Node {
            marker,
            parent: None,
            children: Vec::new(),
        });
        MarkerId(self.nodes.len() - 1)
    }

    /// The marker `id`.
    pub fn marker(&self, id: MarkerId) -> &Marker {
        &self.nodes[id.0].marker
    }

    /// The marker `id`, mutably.
    pub fn marker_mut(&mut self, id: MarkerId) -> &mut Marker {
        &mut self.nodes[id.0].marker
    }

    /// The placeholder `id`, if it is one.
    pub fn placeholder(&self, id: MarkerId) -> Option<&Placeholder> {
        match self.marker(id) {
            Marker::Placeholder(placeholder) => Some(placeholder),
            _ => None,
        }
    }

    /// The variable `id`, if it is one.
    pub fn variable(&self, id: MarkerId) -> Option<&Variable> {
        match self.marker(id) {
            Marker::Variable(variable) => Some(variable),
            _ => None,
        }
    }

    /// The parent of `id`; `None` for the root and detached markers.
    pub fn parent(&self, id: MarkerId) -> Option<MarkerId> {
        self.nodes[id.0].parent
    }

    /// The children of `id`.
    pub fn children(&self, id: MarkerId) -> &[MarkerId] {
        &self.nodes[id.0].children
    }

    /// Appends `child` to `parent`, as `Marker.appendChild`: text following
    /// text is merged into it (leaving `child` detached), and a
    /// [`Marker::Choice`] parent takes text as a new option and ignores
    /// anything else.
    pub fn append_child(&mut self, parent: MarkerId, child: MarkerId) {
        if let Marker::Text(text) = &self.nodes[child.0].marker {
            let text = text.value.clone();
            if let Marker::Choice(choice) = &mut self.nodes[parent.0].marker {
                choice.options.push(Text::new(text));
                self.nodes[child.0].parent = Some(parent);
                return;
            }
            if let Some(&last) = self.nodes[parent.0].children.last()
                && let Marker::Text(last) = &mut self.nodes[last.0].marker
            {
                last.value.push_str(&text);
                return;
            }
        } else if matches!(self.nodes[parent.0].marker, Marker::Choice(_)) {
            return;
        }
        self.nodes[child.0].parent = Some(parent);
        self.nodes[parent.0].children.push(child);
    }

    /// Replaces `child` in its parent's children with `others`, which take it
    /// as their parent. `others` should be detached, e.g. from
    /// [`TextmateSnippet::create`], [`TextmateSnippet::clone_marker`] or
    /// [`TextmateSnippet::import`]. Does nothing if `child` has no parent.
    pub fn replace(&mut self, child: MarkerId, others: &[MarkerId]) {
        let Some(parent) = self.parent(child) else {
            return;
        };
        let Some(idx) = self.children(parent).iter().position(|&c| c == child) else {
            return;
        };
        let mut new_children = self.children(parent).to_vec();
        new_children.splice(idx..=idx, others.iter().copied());
        self.nodes[parent.0].children = new_children;
        for &other in others {
            self.fix_parent(other, parent);
        }
    }

    /// Re-points `id` and its subtree at their parents (Monaco's `_fixParent`).
    fn fix_parent(&mut self, id: MarkerId, parent: MarkerId) {
        self.nodes[id.0].parent = Some(parent);
        for child in self.nodes[id.0].children.clone() {
            self.fix_parent(child, id);
        }
    }

    /// Replaces `child` with copies of `other`'s top-level markers, which is
    /// how Monaco nests one snippet into another.
    pub fn replace_with_snippet(&mut self, child: MarkerId, other: &TextmateSnippet) {
        let others: Vec<MarkerId> = other
            .children(other.root())
            .iter()
            .map(|&id| self.import(other, id))
            .collect();
        self.replace(child, &others);
    }

    /// Deep-copies `other`'s marker `id` into this arena, detached.
    pub fn import(&mut self, other: &TextmateSnippet, id: MarkerId) -> MarkerId {
        let copy = self.create(other.marker(id).clone());
        for &child in other.children(id) {
            let child = self.import(other, child);
            self.nodes[child.0].parent = Some(copy);
            self.nodes[copy.0].children.push(child);
        }
        copy
    }

    /// Deep-copies marker `id` within this arena (Monaco's `Marker.clone`); the
    /// copy is detached.
    pub fn clone_marker(&mut self, id: MarkerId) -> MarkerId {
        let copy = self.create(self.nodes[id.0].marker.clone());
        for child in self.nodes[id.0].children.clone() {
            let child = self.clone_marker(child);
            self.nodes[child.0].parent = Some(copy);
            self.nodes[copy.0].children.push(child);
        }
        copy
    }

    /// The last marker reached by always taking the last child from `id`.
    pub fn right_most_descendant(&self, mut id: MarkerId) -> MarkerId {
        while let Some(&last) = self.children(id).last() {
            id = last;
        }
        id
    }

    /// The choice of placeholder `id`, if a choice is its only child.
    pub fn choice(&self, id: MarkerId) -> Option<&Choice> {
        if !matches!(self.marker(id), Marker::Placeholder(_)) {
            return None;
        }
        match self.children(id) {
            [only] => match self.marker(*only) {
                Marker::Choice(choice) => Some(choice),
                _ => None,
            },
            _ => None,
        }
    }

    /// Visits the markers below the root in pre-order, stopping as soon as
    /// `visitor` returns false.
    pub fn walk(&self, visitor: impl FnMut(MarkerId) -> bool) {
        self.walk_from(self.children(ROOT), visitor);
    }

    fn walk_from(&self, markers: &[MarkerId], mut visitor: impl FnMut(MarkerId) -> bool) {
        let mut stack: Vec<MarkerId> = markers.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            if !visitor(id) {
                break;
            }
            stack.extend(self.children(id).iter().rev());
        }
    }

    /// All placeholders, and the first one with the highest index.
    pub fn placeholder_info(&self) -> PlaceholderInfo {
        let mut all = Vec::new();
        let mut last: Option<(MarkerId, usize)> = None;
        self.walk(|id| {
            if let Some(placeholder) = self.placeholder(id) {
                all.push(id);
                if last.is_none_or(|(_, index)| index < placeholder.index) {
                    last = Some((id, placeholder.index));
                }
            }
            true
        });
        PlaceholderInfo {
            all,
            last: last.map(|(id, _)| id),
        }
    }

    /// All placeholders, in document order.
    pub fn placeholders(&self) -> Vec<MarkerId> {
        self.placeholder_info().all
    }

    /// Where `marker` starts in the snippet's text, in UTF-16 code units, or
    /// `None` if it isn't in the tree.
    pub fn offset(&self, marker: MarkerId) -> Option<usize> {
        let mut pos = 0;
        let mut found = false;
        self.walk(|candidate| {
            if candidate == marker {
                found = true;
                return false;
            }
            pos += self.marker(candidate).len();
            true
        });
        found.then_some(pos)
    }

    /// The length of `marker`'s text, children included, in UTF-16 code units.
    pub fn full_len(&self, marker: MarkerId) -> usize {
        let mut ret = 0;
        self.walk_from(&[marker], |id| {
            ret += self.marker(id).len();
            true
        });
        ret
    }

    /// The placeholders containing `placeholder`, innermost first.
    pub fn enclosing_placeholders(&self, placeholder: MarkerId) -> Vec<MarkerId> {
        let mut ret = Vec::new();
        let mut parent = self.parent(placeholder);
        while let Some(id) = parent {
            if self.placeholder(id).is_some() {
                ret.push(id);
            }
            parent = self.parent(id);
        }
        ret
    }

    /// Resolves variable `id` with `resolver` (Monaco's `Variable.resolve`):
    /// on success its children become the value as one text marker. Returns
    /// whether it resolved.
    pub fn resolve_variable(&mut self, id: MarkerId, resolver: &dyn VariableResolver) -> bool {
        let Some(value) = self.variable(id).and_then(|v| v.resolve(resolver)) else {
            return false;
        };
        let text = self.create(Marker::Text(Text::new(value)));
        self.nodes[text.0].parent = Some(id);
        self.nodes[id.0].children = vec![text];
        true
    }

    /// Resolves every variable in the snippet; see
    /// [`TextmateSnippet::resolve_variable`].
    pub fn resolve_variables(&mut self, resolver: &dyn VariableResolver) -> &mut Self {
        let mut stack: Vec<MarkerId> = self.children(ROOT).iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            if self.variable(id).is_some() {
                self.resolve_variable(id, resolver);
            }
            stack.extend(self.children(id).iter().rev());
        }
        self
    }

    /// The text `id` inserts (Monaco's `Marker.toString`).
    pub fn marker_to_string(&self, id: MarkerId) -> String {
        let mut out = String::new();
        self.write_string(id, &mut out);
        out
    }

    fn write_string(&self, id: MarkerId, out: &mut String) {
        match self.marker(id) {
            Marker::Text(text) => out.push_str(&text.value),
            Marker::Choice(choice) => {
                if let Some(first) = choice.options.first() {
                    out.push_str(&first.value);
                }
            }
            _ => {
                for &child in self.children(id) {
                    self.write_string(child, out);
                }
            }
        }
    }

    /// `id` in snippet syntax (Monaco's `Marker.toTextmateString`).
    pub fn marker_to_textmate_string(&self, id: MarkerId) -> String {
        let children = || -> String {
            self.children(id)
                .iter()
                .map(|&child| self.marker_to_textmate_string(child))
                .collect()
        };
        let transform_string = |t: &Option<Transform>| {
            t.as_ref()
                .map(Transform::to_textmate_string)
                .unwrap_or_default()
        };
        match self.marker(id) {
            Marker::Text(text) => text.to_textmate_string(),
            Marker::Choice(choice) => choice.to_textmate_string(),
            Marker::Placeholder(p) => {
                let transform = transform_string(&p.transform);
                if self.children(id).is_empty() && p.transform.is_none() {
                    format!("${}", p.index)
                } else if self.children(id).is_empty() {
                    format!("${{{}{transform}}}", p.index)
                } else if let Some(choice) = self.choice(id) {
                    format!(
                        "${{{}|{}|{transform}}}",
                        p.index,
                        choice.to_textmate_string()
                    )
                } else {
                    format!("${{{}:{}{transform}}}", p.index, children())
                }
            }
            Marker::Variable(v) => {
                let transform = transform_string(&v.transform);
                if self.children(id).is_empty() {
                    format!("${{{}{transform}}}", v.name)
                } else {
                    format!("${{{}:{}{transform}}}", v.name, children())
                }
            }
            Marker::TextmateSnippet => children(),
        }
    }

    /// The whole snippet in snippet syntax.
    pub fn to_textmate_string(&self) -> String {
        self.marker_to_textmate_string(ROOT)
    }
}

/// Formats the text the snippet inserts (Monaco's `toString`).
impl fmt::Display for TextmateSnippet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.marker_to_string(ROOT))
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parses TextMate snippet syntax into a [`TextmateSnippet`].
///
/// Malformed constructs are never errors: they parse as plain text.
#[derive(Clone, Debug)]
pub struct SnippetParser {
    scanner: Scanner,
    token: Token,
}

impl Default for SnippetParser {
    fn default() -> Self {
        Self::new()
    }
}

/// A placeholder/format index; JS `Number(digits)` never fails, so saturate.
fn parse_index(digits: &str) -> usize {
    digits.parse().unwrap_or(usize::MAX)
}

/// Whether JS `new RegExp` would accept `pattern`: it compiles and its flags
/// are known and not repeated.
fn is_valid_regexp(pattern: &Pattern) -> bool {
    let mut seen = HashSet::new();
    pattern.regex().is_some() && pattern.flags.chars().all(|flag| seen.insert(flag))
}

impl SnippetParser {
    /// A parser.
    pub fn new() -> Self {
        Self {
            scanner: Scanner::new(),
            token: Token {
                kind: TokenType::Eof,
                pos: 0,
                len: 0,
            },
        }
    }

    /// Escapes `$`, `}` and `\` so `value` inserts literally.
    pub fn escape(value: &str) -> String {
        Text::escape(value)
    }

    /// The text `value` inserts, without tabstops, placeholders or variables.
    pub fn as_insert_text(value: &str) -> String {
        SnippetParser::new().parse(value, false, false).to_string()
    }

    /// Whether `template` likely uses the `CLIPBOARD` variable.
    pub fn guess_needs_clipboard(template: &str) -> bool {
        template.contains("$CLIPBOARD") || template.contains("${CLIPBOARD")
    }

    /// Parses `value`. `insert_final_tabstop` appends `$0` if the snippet has
    /// placeholders but no `$0`; `enforce_final_tabstop` appends it even
    /// without placeholders.
    pub fn parse(
        &mut self,
        value: &str,
        insert_final_tabstop: bool,
        enforce_final_tabstop: bool,
    ) -> TextmateSnippet {
        let mut snippet = TextmateSnippet::new();
        self.parse_fragment(value, &mut snippet);
        self.ensure_final_tabstop(&mut snippet, enforce_final_tabstop, insert_final_tabstop);
        snippet
    }

    /// Parses `value` and appends it to `snippet`'s top level, then gives
    /// empty placeholders the default value of the first placeholder with the
    /// same index. Returns the new top-level markers.
    pub fn parse_fragment(&mut self, value: &str, snippet: &mut TextmateSnippet) -> Vec<MarkerId> {
        let offset = snippet.children(ROOT).len();
        self.scanner.text(value);
        self.token = self.scanner.next();
        while self.parse_marker(snippet, ROOT) {}

        // The first placeholder of an index that has a value defines the
        // value for all placeholders with that index. `None` marks `$0`,
        // which never gets one.
        let mut default_values: HashMap<usize, Option<Vec<MarkerId>>> = HashMap::new();
        let mut incomplete = Vec::new();
        snippet.walk(|id| {
            if let Some(placeholder) = snippet.placeholder(id) {
                if placeholder.is_final_tabstop() {
                    default_values.insert(0, None);
                } else if !default_values.contains_key(&placeholder.index)
                    && !snippet.children(id).is_empty()
                {
                    // A snapshot, like the JS array reference: `replace`
                    // installs new child lists rather than editing this one.
                    default_values.insert(placeholder.index, Some(snippet.children(id).to_vec()));
                } else {
                    incomplete.push(id);
                }
            }
            true
        });

        let mut stack = HashSet::new();
        for placeholder in incomplete {
            Self::fill_in_incomplete_placeholder(snippet, &default_values, placeholder, &mut stack);
        }

        snippet.children(ROOT)[offset.min(snippet.children(ROOT).len())..].to_vec()
    }

    fn fill_in_incomplete_placeholder(
        snippet: &mut TextmateSnippet,
        default_values: &HashMap<usize, Option<Vec<MarkerId>>>,
        placeholder: MarkerId,
        stack: &mut HashSet<usize>,
    ) {
        let Some(original) = snippet.placeholder(placeholder) else {
            return;
        };
        let Some(Some(values)) = default_values.get(&original.index) else {
            return;
        };
        let clone = snippet.create(Marker::Placeholder(original.clone()));
        for &child in values {
            let new_child = snippet.clone_marker(child);
            snippet.append_child(clone, new_child);

            // "Recurse" on children that are again placeholders; `stack`
            // breaks cycles such as `${1:$2} ${2:$1}`.
            if let Some(index) = snippet.placeholder(new_child).map(|p| p.index)
                && default_values.contains_key(&index)
                && !stack.contains(&index)
            {
                stack.insert(index);
                Self::fill_in_incomplete_placeholder(snippet, default_values, new_child, stack);
                stack.remove(&index);
            }
        }
        snippet.replace(placeholder, &[clone]);
    }

    /// Appends `$0` to `snippet` if `enforce_final_tabstop`, or if
    /// `insert_final_tabstop` and it has placeholders, unless it has one.
    pub fn ensure_final_tabstop(
        &self,
        snippet: &mut TextmateSnippet,
        enforce_final_tabstop: bool,
        insert_final_tabstop: bool,
    ) {
        let placeholders = snippet.placeholders();
        if enforce_final_tabstop || insert_final_tabstop && !placeholders.is_empty() {
            let has_final = placeholders
                .iter()
                .any(|&id| snippet.placeholder(id).is_some_and(|p| p.index == 0));
            if !has_final {
                // The snippet uses placeholders but has no final tabstop
                // defined -> insert at the end.
                let final_tabstop = snippet.create(Marker::Placeholder(Placeholder::new(0)));
                snippet.append_child(ROOT, final_tabstop);
            }
        }
    }

    /// Consumes the current token if it is `kind`.
    fn accept(&mut self, kind: TokenType) -> bool {
        self.accept_text(kind).is_some()
    }

    /// Consumes the current token if it is `kind`, returning its text.
    fn accept_text(&mut self, kind: TokenType) -> Option<String> {
        if self.token.kind != kind {
            return None;
        }
        Some(self.accept_any())
    }

    /// Consumes the current token whatever it is, returning its text (empty
    /// at the end).
    fn accept_any(&mut self) -> String {
        let text = self.scanner.token_text(&self.token).to_string();
        self.token = self.scanner.next();
        text
    }

    /// Rewinds to `token`; always false so callers can `return` it.
    fn back_to(&mut self, token: Token) -> bool {
        self.scanner.pos = token.pos + token.len;
        self.token = token;
        false
    }

    /// The text up to the next `kind` token, unescaping `\$`, `\}` and `\\`;
    /// `None` at the end or on any other escape.
    fn until(&mut self, kind: TokenType) -> Option<String> {
        let start = self.token;
        while self.token.kind != kind {
            match self.token.kind {
                TokenType::Eof => return None,
                TokenType::Backslash => {
                    let next = self.scanner.next();
                    if !matches!(
                        next.kind,
                        TokenType::Dollar | TokenType::CurlyClose | TokenType::Backslash
                    ) {
                        return None;
                    }
                }
                _ => {}
            }
            self.token = self.scanner.next();
        }
        let raw = &self.scanner.value[start.pos..self.token.pos];
        let mut value = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\'
                && let Some(&next) = chars.peek()
                && matches!(next, '$' | '}' | '\\')
            {
                value.push(next);
                chars.next();
            } else {
                value.push(c);
            }
        }
        self.token = self.scanner.next();
        Some(value)
    }

    fn parse_marker(&mut self, snippet: &mut TextmateSnippet, marker: MarkerId) -> bool {
        self.parse_escaped(snippet, marker)
            || self.parse_tabstop_or_variable_name(snippet, marker)
            || self.parse_complex_placeholder(snippet, marker)
            || self.parse_complex_variable(snippet, marker)
            || self.parse_anything(snippet, marker)
    }

    fn append_text(snippet: &mut TextmateSnippet, parent: MarkerId, value: String) {
        let text = snippet.create(Marker::Text(Text::new(value)));
        snippet.append_child(parent, text);
    }

    /// `\$`, `\\`, `\}` -> just text.
    fn parse_escaped(&mut self, snippet: &mut TextmateSnippet, marker: MarkerId) -> bool {
        let Some(backslash) = self.accept_text(TokenType::Backslash) else {
            return false;
        };
        // Saw a backslash: append the escaped token or that backslash.
        let value = self
            .accept_text(TokenType::Dollar)
            .or_else(|| self.accept_text(TokenType::CurlyClose))
            .or_else(|| self.accept_text(TokenType::Backslash))
            .unwrap_or(backslash);
        Self::append_text(snippet, marker, value);
        true
    }

    /// `$foo` -> variable, `$1` -> tabstop.
    fn parse_tabstop_or_variable_name(
        &mut self,
        snippet: &mut TextmateSnippet,
        parent: MarkerId,
    ) -> bool {
        let token = self.token;
        let value = if self.accept(TokenType::Dollar) {
            self.accept_text(TokenType::VariableName)
                .or_else(|| self.accept_text(TokenType::Int))
        } else {
            None
        };
        let Some(value) = value else {
            return self.back_to(token);
        };
        let marker = if value.bytes().all(|b| b.is_ascii_digit()) {
            Marker::Placeholder(Placeholder::new(parse_index(&value)))
        } else {
            Marker::Variable(Variable::new(value))
        };
        let child = snippet.create(marker);
        snippet.append_child(parent, child);
        true
    }

    /// `${1:<children>}`, `${1}`, `${1|a,b|}`, `${1/regex/format/flags}` ->
    /// placeholder.
    fn parse_complex_placeholder(
        &mut self,
        snippet: &mut TextmateSnippet,
        parent: MarkerId,
    ) -> bool {
        let token = self.token;
        let index = if self.accept(TokenType::Dollar) && self.accept(TokenType::CurlyOpen) {
            self.accept_text(TokenType::Int)
        } else {
            None
        };
        let Some(index) = index else {
            return self.back_to(token);
        };

        let value = Placeholder::new(parse_index(&index));
        let is_final = value.is_final_tabstop();
        let placeholder = snippet.create(Marker::Placeholder(value));

        if self.accept(TokenType::Colon) {
            // ${1:<children>}
            loop {
                // ...} -> done
                if self.accept(TokenType::CurlyClose) {
                    snippet.append_child(parent, placeholder);
                    return true;
                }
                if self.parse_marker(snippet, placeholder) {
                    continue;
                }
                // Fallback: unterminated, so it was text after all.
                Self::append_text(snippet, parent, format!("${{{index}:"));
                for child in snippet.children(placeholder).to_vec() {
                    snippet.append_child(parent, child);
                }
                return true;
            }
        } else if !is_final && self.accept(TokenType::Pipe) {
            // ${1|one,two,three|}
            let choice = snippet.create(Marker::Choice(Choice::default()));
            loop {
                if self.parse_choice_element(snippet, choice) {
                    if self.accept(TokenType::Comma) {
                        // opt, -> more
                        continue;
                    }
                    if self.accept(TokenType::Pipe) {
                        snippet.append_child(placeholder, choice);
                        if self.accept(TokenType::CurlyClose) {
                            // ..|} -> done
                            snippet.append_child(parent, placeholder);
                            return true;
                        }
                    }
                }
                return self.back_to(token);
            }
        } else if self.accept(TokenType::Forwardslash) {
            // ${1/<regex>/<format>/<options>}
            if self.parse_transform(snippet, placeholder) {
                snippet.append_child(parent, placeholder);
                return true;
            }
            self.back_to(token)
        } else if self.accept(TokenType::CurlyClose) {
            // ${1}
            snippet.append_child(parent, placeholder);
            true
        } else {
            // ${1 <- missing curly or colon
            self.back_to(token)
        }
    }

    fn parse_choice_element(&mut self, snippet: &mut TextmateSnippet, parent: MarkerId) -> bool {
        let token = self.token;
        let mut values = String::new();
        while !matches!(self.token.kind, TokenType::Comma | TokenType::Pipe) {
            let value = if let Some(backslash) = self.accept_text(TokenType::Backslash) {
                // \, \|, or \\
                self.accept_text(TokenType::Comma)
                    .or_else(|| self.accept_text(TokenType::Pipe))
                    .or_else(|| self.accept_text(TokenType::Backslash))
                    .unwrap_or(backslash)
            } else {
                self.accept_any()
            };
            if value.is_empty() {
                // EOF
                return self.back_to(token);
            }
            values.push_str(&value);
        }
        if values.is_empty() {
            return self.back_to(token);
        }
        Self::append_text(snippet, parent, values);
        true
    }

    /// `${foo:<children>}`, `${foo}`, `${foo/regex/format/flags}` -> variable.
    fn parse_complex_variable(&mut self, snippet: &mut TextmateSnippet, parent: MarkerId) -> bool {
        let token = self.token;
        let name = if self.accept(TokenType::Dollar) && self.accept(TokenType::CurlyOpen) {
            self.accept_text(TokenType::VariableName)
        } else {
            None
        };
        let Some(name) = name else {
            return self.back_to(token);
        };

        let variable = snippet.create(Marker::Variable(Variable::new(name.clone())));

        if self.accept(TokenType::Colon) {
            // ${foo:<children>}
            loop {
                // ...} -> done
                if self.accept(TokenType::CurlyClose) {
                    snippet.append_child(parent, variable);
                    return true;
                }
                if self.parse_marker(snippet, variable) {
                    continue;
                }
                // Fallback: unterminated, so it was text after all.
                Self::append_text(snippet, parent, format!("${{{name}:"));
                for child in snippet.children(variable).to_vec() {
                    snippet.append_child(parent, child);
                }
                return true;
            }
        } else if self.accept(TokenType::Forwardslash) {
            // ${foo/<regex>/<format>/<options>}
            if self.parse_transform(snippet, variable) {
                snippet.append_child(parent, variable);
                return true;
            }
            self.back_to(token)
        } else if self.accept(TokenType::CurlyClose) {
            // ${foo}
            snippet.append_child(parent, variable);
            true
        } else {
            // ${foo <- missing curly or colon
            self.back_to(token)
        }
    }

    /// `...<regex>/<format>/<options>}` after `${1/` or `${foo/`; sets the
    /// transform of `parent` (a placeholder or variable).
    fn parse_transform(&mut self, snippet: &mut TextmateSnippet, parent: MarkerId) -> bool {
        let mut transform = Transform::new();
        let mut regex_value = String::new();
        let mut regex_options = String::new();

        // (1) /regex
        loop {
            if self.accept(TokenType::Forwardslash) {
                break;
            }
            if let Some(backslash) = self.accept_text(TokenType::Backslash) {
                let escaped = self
                    .accept_text(TokenType::Forwardslash)
                    .unwrap_or(backslash);
                regex_value.push_str(&escaped);
                continue;
            }
            if self.token.kind != TokenType::Eof {
                regex_value.push_str(&self.accept_any());
                continue;
            }
            return false;
        }

        // (2) /format
        loop {
            if self.accept(TokenType::Forwardslash) {
                break;
            }
            if let Some(backslash) = self.accept_text(TokenType::Backslash) {
                let escaped = self
                    .accept_text(TokenType::Backslash)
                    .or_else(|| self.accept_text(TokenType::Forwardslash))
                    .unwrap_or(backslash);
                transform.append_child(TransformPart::Text(Text::new(escaped)));
                continue;
            }
            if self.parse_format_string(&mut transform) {
                continue;
            }
            // Monaco's `_parseAnything` on the transform.
            if self.token.kind != TokenType::Eof {
                let text = self.accept_any();
                transform.append_child(TransformPart::Text(Text::new(text)));
                continue;
            }
            return false;
        }

        // (3) /option
        loop {
            if self.accept(TokenType::CurlyClose) {
                break;
            }
            if self.token.kind != TokenType::Eof {
                regex_options.push_str(&self.accept_any());
                continue;
            }
            return false;
        }

        let regexp = Pattern::new(regex_value, regex_options);
        if !is_valid_regexp(&regexp) {
            return false;
        }
        transform.regexp = regexp;

        match snippet.marker_mut(parent) {
            Marker::Placeholder(placeholder) => placeholder.transform = Some(transform),
            Marker::Variable(variable) => variable.transform = Some(transform),
            _ => return false,
        }
        true
    }

    fn parse_format_string(&mut self, parent: &mut Transform) -> bool {
        let token = self.token;
        if !self.accept(TokenType::Dollar) {
            return false;
        }
        let complex = self.accept(TokenType::CurlyOpen);
        let Some(index) = self.accept_text(TokenType::Int) else {
            return self.back_to(token);
        };
        let index = parse_index(&index);
        let mut push = |format: FormatString| {
            parent.append_child(TransformPart::FormatString(format));
            true
        };

        if !complex || self.accept(TokenType::CurlyClose) {
            // $1, ${1}
            return push(FormatString::new(index, None, None, None));
        }
        if !self.accept(TokenType::Colon) {
            return self.back_to(token);
        }

        let non_empty = |v: Option<String>| v.filter(|v| !v.is_empty());
        if self.accept(TokenType::Forwardslash) {
            // ${1:/upcase}
            let shorthand = self.accept_text(TokenType::VariableName);
            if let Some(shorthand) = shorthand
                && self.accept(TokenType::CurlyClose)
            {
                return push(FormatString::new(index, Some(&shorthand), None, None));
            }
        } else if self.accept(TokenType::Plus) {
            // ${1:+<if>}
            if let Some(if_value) = non_empty(self.until(TokenType::CurlyClose)) {
                return push(FormatString::new(index, None, Some(&if_value), None));
            }
        } else if self.accept(TokenType::Dash) {
            // ${2:-<else>}
            if let Some(else_value) = non_empty(self.until(TokenType::CurlyClose)) {
                return push(FormatString::new(index, None, None, Some(&else_value)));
            }
        } else if self.accept(TokenType::QuestionMark) {
            // ${2:?<if>:<else>}
            if let Some(if_value) = non_empty(self.until(TokenType::Colon))
                && let Some(else_value) = non_empty(self.until(TokenType::CurlyClose))
            {
                return push(FormatString::new(
                    index,
                    None,
                    Some(&if_value),
                    Some(&else_value),
                ));
            }
        } else {
            // ${1:<else>}
            if let Some(else_value) = non_empty(self.until(TokenType::CurlyClose)) {
                return push(FormatString::new(index, None, None, Some(&else_value)));
            }
        }
        self.back_to(token)
    }

    fn parse_anything(&mut self, snippet: &mut TextmateSnippet, marker: MarkerId) -> bool {
        if self.token.kind == TokenType::Eof {
            return false;
        }
        let text = self.accept_any();
        Self::append_text(snippet, marker, text);
        true
    }
}
