//! Compiling a Monarch grammar (Monaco `monarchCompile.ts` and
//! `monarchCommon.ts`, MIT) into a [`Lexer`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use fancy_regex::Regex;
use serde_json::{Map, Value};

use super::MonarchError;
use super::js_regex;

/// Rules nested deeper than this through `include` are an error (Monaco
/// would overflow its stack).
const MAX_INCLUDE_DEPTH: usize = 64;

/// Dynamic (`$Sn`) regexes cached per rule, by their substituted source.
const MAX_DYNAMIC_REGEXES: usize = 64;

/// Monaco `ILexer`: the compiled grammar.
pub(crate) struct Lexer {
    pub language_id: Arc<str>,
    pub include_lf: bool,
    pub ignore_case: bool,
    pub max_stack: usize,
    pub start: String,
    pub token_postfix: String,
    pub default_token: String,
    pub uses_embedded: bool,
    pub tokenizer: HashMap<String, Vec<Rule>>,
    pub brackets: Vec<Bracket>,
    /// The action for unmatched text: `defaultToken`.
    pub default_action: Action,
}

/// Monaco `IBracket`.
pub(crate) struct Bracket {
    pub token: String,
    pub open: String,
    pub close: String,
}

/// A rule's regex: compiled once, or per state when it uses `$Sn`.
pub(crate) enum RuleRegex {
    Static {
        anchored: Regex,
        /// The JavaScript source without the `^(?:...)` wrapper.
        body: String,
        unanchored: OnceLock<Result<Regex, String>>,
    },
    Dynamic {
        body: String,
        cache: Mutex<HashMap<String, Arc<Regex>>>,
    },
}

/// Monaco `IRule`.
pub(crate) struct Rule {
    pub name: String,
    pub regex: RuleRegex,
    pub action: Action,
    pub match_only_at_line_start: bool,
}

/// Monaco `MonarchBracket` (parsed and checked, unused by the lexer, as in
/// Monaco).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BracketKind {
    None,
    Open,
    Close,
}

/// Monaco `IAction` with a token.
pub(crate) struct TokenAction {
    pub token: String,
    pub token_subst: bool,
    pub next: Option<String>,
    pub next_embedded: Option<String>,
    #[allow(dead_code)]
    pub bracket: BracketKind,
    pub log: Option<String>,
    pub switch_to: Option<String>,
    pub go_back: i64,
}

/// Monaco `FuzzyAction`.
pub(crate) enum Action {
    /// A bare token string (`'keyword'`, `'@rematch'`, `'@brackets'`).
    Token(String),
    /// `{ token, next, ... }`.
    Full(Box<TokenAction>),
    /// An array: one action per regex group.
    Group(Vec<Action>),
    /// `{ cases: { ... } }`.
    Cases(Box<Cases>),
}

/// A compiled `cases` object.
pub(crate) struct Cases {
    pub branches: Vec<Branch>,
    pub has_embedded_end_in_cases: bool,
}

/// One case: a guard (`None` for `@default`) and its action.
pub(crate) struct Branch {
    pub guard: Option<Guard>,
    pub value: Action,
}

pub(crate) enum Guard {
    Eos,
    Test {
        scrutinee: Scrutinee,
        tester: Tester,
    },
}

/// What a guard tests: `$#` (the match), `$n` or `$Sn`.
pub(crate) enum Scrutinee {
    Id,
    Match(usize),
    State(usize),
}

pub(crate) enum Tester {
    Words {
        words: HashSet<String>,
        negate: bool,
    },
    Regex {
        regex: Regex,
        negate: bool,
    },
    /// A regex with `$` substitutions, compiled when tested. Monaco ignores
    /// `!~` here and always tests for a match; so does this port.
    DynamicRegex(String),
    Equals {
        pattern: String,
        substitute: bool,
        negate: bool,
    },
}

/// Monaco's `lexerMin`: the grammar object itself (plus a few compiled
/// fields), which `@attr` references and guards read.
pub(crate) struct Attributes {
    json: Map<String, Value>,
    language_id: Arc<str>,
    default_token: String,
    ignore_case: bool,
}

enum Attr<'a> {
    String(&'a str),
    Other,
    Missing,
}

impl Attributes {
    fn get(&self, name: &str) -> Attr<'_> {
        match name {
            "languageId" => Attr::String(&self.language_id),
            "defaultToken" => Attr::String(&self.default_token),
            "includeLF" | "ignoreCase" | "unicode" | "noThrow" | "usesEmbedded" | "stateNames" => {
                Attr::Other
            }
            _ => match self.json.get(name) {
                Some(Value::String(s)) => Attr::String(s),
                Some(_) => Attr::Other,
                None => Attr::Missing,
            },
        }
    }

    pub(crate) fn string(&self, name: &str) -> Option<&str> {
        match self.get(name) {
            Attr::String(s) => Some(s),
            _ => None,
        }
    }
}

/// Monaco `createError`.
pub(crate) fn error(language_id: &str, message: impl std::fmt::Display) -> MonarchError {
    MonarchError(format!("{language_id}: {message}"))
}

/// Monaco `fixCase`.
pub(crate) fn fix_case(ignore_case: bool, s: &str) -> String {
    if ignore_case {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

/// Monaco `sanitize`: no `&<>'"_` in token names.
pub(crate) fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' | '<' | '>' | '\'' | '"' | '_' => '-',
            c => c,
        })
        .collect()
}

/// `[state, ...state.split('.')]`, for `$Sn`.
fn state_parts(state: &str) -> Vec<&str> {
    std::iter::once(state).chain(state.split('.')).collect()
}

/// Monaco `substituteMatches`: `$$`, `$#`, `$n`, `$Sn` and `$@attr` in a
/// token, state or guard string.
pub(crate) fn substitute_matches(
    attr: &dyn Fn(&str) -> Option<String>,
    ignore_case: bool,
    s: &str,
    id: &str,
    matches: &[String],
    state: &str,
) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"\$((\$)|(#)|(\d\d?)|[sS](\d\d?)|@([0-9A-Za-z_]+))").expect("valid")
    });
    if !s.contains('$') {
        return s.to_string();
    }
    re.replace_all(s, |caps: &regex::Captures| {
        if caps.get(2).is_some() {
            return "$".to_string();
        }
        if caps.get(3).is_some() {
            return fix_case(ignore_case, id);
        }
        if let Some(n) = caps.get(4).and_then(|n| n.as_str().parse::<usize>().ok())
            && n < matches.len()
        {
            return fix_case(ignore_case, &matches[n]);
        }
        if let Some(value) = caps.get(6).and_then(|name| attr(name.as_str())) {
            return value;
        }
        if let Some(n) = caps.get(5).and_then(|n| n.as_str().parse::<usize>().ok()) {
            let parts = state_parts(state);
            if n < parts.len() {
                return fix_case(ignore_case, parts[n]);
            }
        }
        String::new()
    })
    .into_owned()
}

/// Monaco `escapeRegExpCharacters`.
fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '{' | '}' | '*' | '+' | '?' | '|' | '^' | '$' | '.' | '[' | ']' | '(' | ')'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn sn_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\$[sS](\d\d?)").expect("valid"))
}

/// Monaco `substituteMatchesRe`: `$Sn` in a regex source.
fn substitute_matches_re(ignore_case: bool, s: &str, state: &str) -> String {
    sn_regex()
        .replace_all(s, |caps: &regex::Captures| {
            let parts = state_parts(state);
            match caps[1].parse::<usize>() {
                Ok(n) if n < parts.len() => escape_regex(&fix_case(ignore_case, parts[n])),
                _ => String::new(),
            }
        })
        .into_owned()
}

/// Compiles a translated JavaScript regex source.
fn build_regex(language_id: &str, source: &str, ignore_case: bool) -> Result<Regex, MonarchError> {
    let translated = js_regex::translate(source, ignore_case).map_err(|e| {
        error(
            language_id,
            format!("invalid regular expression /{source}/: {e}"),
        )
    })?;
    Regex::new(&translated).map_err(|e| {
        error(
            language_id,
            format!("invalid regular expression /{source}/: {e}"),
        )
    })
}

/// Monaco `compileRegExp` without the `$Sn` handling: expands `@attr`
/// references (up to 5 levels) and `@@`.
fn expand_attributes(attrs: &Attributes, source: &str) -> Result<String, MonarchError> {
    static ATTR: OnceLock<regex::Regex> = OnceLock::new();
    let attr_re = ATTR.get_or_init(|| regex::Regex::new(r"@([0-9A-Za-z_]+)").expect("valid"));
    let mut s = source.replace("@@", "\u{1}");
    for _ in 0..5 {
        let mut expanded = false;
        let mut failure = None;
        let next = attr_re
            .replace_all(&s, |caps: &regex::Captures| {
                expanded = true;
                let name = &caps[1];
                match attrs.get(name) {
                    Attr::String("") => String::new(),
                    Attr::String(sub) => format!("(?:{sub})"),
                    Attr::Other => {
                        failure.get_or_insert_with(|| {
                            format!("attribute reference '{name}' must be a string, used at: {s}")
                        });
                        String::new()
                    }
                    Attr::Missing => {
                        failure.get_or_insert_with(|| {
                            format!(
                                "language definition does not contain attribute '{name}', used at: {s}"
                            )
                        });
                        String::new()
                    }
                }
            })
            .into_owned();
        if let Some(failure) = failure {
            return Err(error(&attrs.language_id, failure));
        }
        s = next;
        if !expanded {
            break;
        }
    }
    Ok(s.replace('\u{1}', "@"))
}

impl Rule {
    /// The regex anchored at the start, for `state`.
    pub(crate) fn anchored(&self, lexer: &Lexer, state: &str) -> Result<Arc<Regex>, MonarchError> {
        match &self.regex {
            RuleRegex::Static { anchored, .. } => Ok(Arc::new(anchored.clone())),
            RuleRegex::Dynamic { body, cache } => {
                let source = format!(
                    "^(?:{})",
                    substitute_matches_re(lexer.ignore_case, body, state)
                );
                dynamic(lexer, cache, source)
            }
        }
    }

    /// Whether the rule is static and anchored, to avoid cloning.
    pub(crate) fn static_anchored(&self) -> Option<&Regex> {
        match &self.regex {
            RuleRegex::Static { anchored, .. } => Some(anchored),
            RuleRegex::Dynamic { .. } => None,
        }
    }

    /// The regex without its `^(?:...)` wrapper, for searching a line
    /// (Monaco `_findLeavingNestedLanguageOffset`).
    pub(crate) fn unanchored(
        &self,
        lexer: &Lexer,
        state: &str,
    ) -> Result<Arc<Regex>, MonarchError> {
        match &self.regex {
            RuleRegex::Static {
                body, unanchored, ..
            } => unanchored
                .get_or_init(|| {
                    build_regex(&lexer.language_id, body, lexer.ignore_case).map_err(|e| e.0)
                })
                .clone()
                .map(Arc::new)
                .map_err(MonarchError),
            RuleRegex::Dynamic { body, cache } => {
                let source = substitute_matches_re(lexer.ignore_case, body, state);
                dynamic(lexer, cache, source)
            }
        }
    }
}

fn dynamic(
    lexer: &Lexer,
    cache: &Mutex<HashMap<String, Arc<Regex>>>,
    source: String,
) -> Result<Arc<Regex>, MonarchError> {
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(regex) = cache.get(&source) {
        return Ok(regex.clone());
    }
    let regex = Arc::new(build_regex(&lexer.language_id, &source, lexer.ignore_case)?);
    if cache.len() >= MAX_DYNAMIC_REGEXES {
        cache.clear();
    }
    cache.insert(source, regex.clone());
    Ok(regex)
}

impl Cases {
    /// Monaco's compiled `cases` test function: the first matching case's
    /// action, or `defaultToken`.
    pub(crate) fn select<'a>(
        &'a self,
        lexer: &'a Lexer,
        attrs: &Attributes,
        id: &str,
        matches: &[String],
        state: &str,
        eos: bool,
    ) -> Result<&'a Action, MonarchError> {
        for branch in &self.branches {
            let matched = match &branch.guard {
                None => true,
                Some(guard) => guard.test(attrs, id, matches, state, eos)?,
            };
            if matched {
                return Ok(&branch.value);
            }
        }
        Ok(&lexer.default_action)
    }
}

impl Guard {
    fn test(
        &self,
        attrs: &Attributes,
        id: &str,
        matches: &[String],
        state: &str,
        eos: bool,
    ) -> Result<bool, MonarchError> {
        let (scrutinee, tester) = match self {
            Guard::Eos => return Ok(eos),
            Guard::Test { scrutinee, tester } => (scrutinee, tester),
        };
        // Monaco `selectScrutinee`.
        let s: &str = match *scrutinee {
            Scrutinee::Id => id,
            Scrutinee::Match(n) => matches.get(n).map_or("", String::as_str),
            Scrutinee::State(n) => {
                let parts = state_parts(state);
                parts.get(n).copied().unwrap_or("")
            }
        };
        let attr = |name: &str| attrs.string(name).map(str::to_string);
        Ok(match tester {
            Tester::Words { words, negate } => {
                let found = if attrs.ignore_case {
                    words.contains(&s.to_lowercase())
                } else {
                    words.contains(s)
                };
                found != *negate
            }
            Tester::Regex { regex, negate } => regex.is_match(s).unwrap_or(false) != *negate,
            Tester::DynamicRegex(pattern) => {
                let source = format!(
                    "^{}$",
                    substitute_matches(&attr, attrs.ignore_case, pattern, id, matches, state)
                );
                let source = expand_attributes(attrs, &source)?;
                let regex = build_regex(&attrs.language_id, &source, attrs.ignore_case)?;
                regex.is_match(s).unwrap_or(false)
            }
            Tester::Equals {
                pattern,
                substitute,
                negate,
            } => {
                let equal = if *substitute {
                    s == substitute_matches(&attr, attrs.ignore_case, pattern, id, matches, state)
                } else {
                    s == pattern
                };
                equal != *negate
            }
        })
    }
}

/// Monaco `findRules`: the rules of `state` or of its closest parent
/// (`a.b.c`, then `a.b`, then `a`).
pub(crate) fn find_rules<'a>(lexer: &'a Lexer, state: &str) -> Option<&'a [Rule]> {
    let mut state = state;
    while !state.is_empty() {
        if let Some(rules) = lexer.tokenizer.get(state) {
            return Some(rules);
        }
        state = &state[..state.rfind('.')?];
    }
    None
}

/// Monaco `stateExists`, against the grammar's states.
fn state_exists(states: &Map<String, Value>, state: &str) -> bool {
    let mut state = state;
    while !state.is_empty() {
        if states.get(state).is_some_and(|v| !v.is_null()) {
            return true;
        }
        match state.rfind('.') {
            Some(ix) => state = &state[..ix],
            None => return false,
        }
    }
    false
}

struct Compiler<'a> {
    attrs: &'a Attributes,
    states: &'a Map<String, Value>,
    ignore_case: bool,
    uses_embedded: bool,
}

impl Compiler<'_> {
    fn error(&self, message: impl std::fmt::Display) -> MonarchError {
        error(&self.attrs.language_id, message)
    }

    /// Monaco `compileRegExp` for a rule: `^(?:...)`, static or per state.
    fn rule_regex(&self, body: String) -> Result<RuleRegex, MonarchError> {
        if sn_regex().is_match(&body) {
            // Checked when first used, like Monaco.
            return Ok(RuleRegex::Dynamic {
                body,
                cache: Mutex::default(),
            });
        }
        let anchored = build_regex(
            &self.attrs.language_id,
            &format!("^(?:{body})"),
            self.ignore_case,
        )?;
        Ok(RuleRegex::Static {
            anchored,
            body,
            unanchored: OnceLock::new(),
        })
    }

    /// Monaco `Rule.setRegex`.
    fn set_regex(
        &self,
        rule_name: &mut String,
        regex: &Value,
    ) -> Result<(RuleRegex, bool), MonarchError> {
        let Some(source) = regex.as_str() else {
            return Err(self.error(format!(
                "rules must start with a match string or regular expression: {rule_name}"
            )));
        };
        let at_line_start = source.starts_with('^');
        *rule_name = format!("{rule_name}: {source}");
        let body = expand_attributes(
            self.attrs,
            if at_line_start { &source[1..] } else { source },
        )?;
        Ok((self.rule_regex(body)?, at_line_start))
    }

    /// Monaco `compileAction`.
    fn action(&mut self, rule_name: &str, action: Option<&Value>) -> Result<Action, MonarchError> {
        let action = match action {
            None | Some(Value::Null) | Some(Value::Bool(false)) => {
                return Ok(Action::Full(Box::new(TokenAction {
                    token: String::new(),
                    token_subst: false,
                    next: None,
                    next_embedded: None,
                    bracket: BracketKind::None,
                    log: None,
                    switch_to: None,
                    go_back: 0,
                })));
            }
            Some(action) => action,
        };
        match action {
            Value::String(token) if token.is_empty() => self.action(rule_name, None),
            Value::String(token) => Ok(Action::Token(token.clone())),
            Value::Object(object) if object.contains_key("token") && !object["token"].is_null() => {
                let Some(token) = object["token"].as_str() else {
                    return Err(self.error(format!(
                        "a 'token' attribute must be of type string, in rule: {rule_name}"
                    )));
                };
                let mut compiled = TokenAction {
                    token: token.to_string(),
                    token_subst: token.contains('$'),
                    next: None,
                    next_embedded: None,
                    bracket: BracketKind::None,
                    log: None,
                    switch_to: None,
                    go_back: 0,
                };
                if let Some(bracket) = object.get("bracket").and_then(Value::as_str) {
                    compiled.bracket = match bracket {
                        "@open" => BracketKind::Open,
                        "@close" => BracketKind::Close,
                        _ => {
                            return Err(self.error(format!(
                                "a 'bracket' attribute must be either '@open' or '@close', in rule: {rule_name}"
                            )));
                        }
                    };
                }
                match object.get("next") {
                    None | Some(Value::Null) | Some(Value::Bool(false)) => {}
                    Some(Value::String(next)) if next.is_empty() => {}
                    Some(Value::String(next)) => {
                        let mut next = next.clone();
                        if !matches!(next.as_str(), "@pop" | "@push" | "@popall") {
                            if let Some(rest) = next.strip_prefix('@') {
                                next = rest.to_string();
                            }
                            if !next.contains('$') {
                                let attr = |name: &str| self.attrs.string(name).map(str::to_string);
                                let state =
                                    substitute_matches(&attr, self.ignore_case, &next, "", &[], "");
                                if !state_exists(self.states, &state) {
                                    return Err(self.error(format!(
                                        "the next state '{}' is not defined in rule: {rule_name}",
                                        object["next"].as_str().unwrap_or_default()
                                    )));
                                }
                            }
                        }
                        compiled.next = Some(next);
                    }
                    Some(_) => {
                        return Err(self.error(format!(
                            "the next state must be a string value in rule: {rule_name}"
                        )));
                    }
                }
                if let Some(go_back) = object.get("goBack").and_then(Value::as_f64) {
                    compiled.go_back = go_back as i64;
                }
                if let Some(switch_to) = object.get("switchTo").and_then(Value::as_str) {
                    compiled.switch_to = Some(switch_to.to_string());
                }
                if let Some(log) = object.get("log").and_then(Value::as_str) {
                    compiled.log = Some(log.to_string());
                }
                if let Some(next_embedded) = object.get("nextEmbedded").and_then(Value::as_str) {
                    compiled.next_embedded = Some(next_embedded.to_string());
                    self.uses_embedded = true;
                }
                Ok(Action::Full(Box::new(compiled)))
            }
            Value::Array(actions) => Ok(Action::Group(
                actions
                    .iter()
                    .map(|action| self.action(rule_name, Some(action)))
                    .collect::<Result<_, _>>()?,
            )),
            Value::Object(object) if object.get("cases").is_some_and(|c| !c.is_null()) => {
                let mut branches = Vec::new();
                let mut has_embedded_end_in_cases = false;
                if let Some(cases) = object["cases"].as_object() {
                    for (key, value) in cases {
                        let value = self.action(rule_name, Some(value))?;
                        let guard = match key.as_str() {
                            "@default" | "@" | "" => None,
                            "@eos" => Some(Guard::Eos),
                            _ => Some(self.guard(rule_name, key)?),
                        };
                        if !has_embedded_end_in_cases {
                            has_embedded_end_in_cases = match &value {
                                Action::Full(full) => matches!(
                                    full.next_embedded.as_deref(),
                                    Some("@pop" | "@popall")
                                ),
                                Action::Cases(cases) => cases.has_embedded_end_in_cases,
                                _ => false,
                            };
                        }
                        branches.push(Branch {
                            guard,
                            value,
                        });
                    }
                }
                Ok(Action::Cases(Box::new(Cases {
                    branches,
                    has_embedded_end_in_cases,
                })))
            }
            _ => Err(self.error(format!(
                "an action must be a string, an object with a 'token' or 'cases' attribute, or an array of actions; in rule: {rule_name}"
            ))),
        }
    }

    /// Monaco `createGuard`.
    fn guard(&self, rule_name: &str, key: &str) -> Result<Guard, MonarchError> {
        static KEY: OnceLock<regex::Regex> = OnceLock::new();
        static WORD: OnceLock<regex::Regex> = OnceLock::new();
        static OP: OnceLock<regex::Regex> = OnceLock::new();
        static WORDS: OnceLock<regex::Regex> = OnceLock::new();
        let key_re =
            KEY.get_or_init(|| regex::Regex::new(r"^\$(([sS]?)(\d\d?)|#)(.*)$").expect("valid"));
        let word_re = WORD.get_or_init(|| regex::Regex::new(r"^[0-9A-Za-z_]*$").expect("valid"));
        let op_re =
            OP.get_or_init(|| regex::Regex::new(r"^(@|!@|~|!~|==|!=)(.*)$").expect("valid"));
        let words_re =
            WORDS.get_or_init(|| regex::Regex::new(r"^([0-9A-Za-z_]|\|)*$").expect("valid"));

        let mut scrutinee = Scrutinee::Id;
        let mut oppat = key;
        if let Some(caps) = key_re.captures(key) {
            if let Some(digits) = caps.get(3) {
                let n: usize = digits.as_str().parse().unwrap_or(0);
                scrutinee = if caps.get(2).is_some_and(|s| !s.as_str().is_empty()) {
                    Scrutinee::State(n)
                } else {
                    Scrutinee::Match(n)
                };
            }
            oppat = caps.get(4).map_or("", |m| m.as_str());
        }
        let (op, pat) = if oppat.is_empty() {
            ("!=", "")
        } else if word_re.is_match(oppat) {
            ("==", oppat)
        } else if let Some(caps) = op_re.captures(oppat) {
            (
                caps.get(1).map_or("~", |m| m.as_str()),
                caps.get(2).map_or("", |m| m.as_str()),
            )
        } else {
            ("~", oppat)
        };
        let lower = |words: Vec<String>| -> HashSet<String> {
            if self.ignore_case {
                words.into_iter().map(|w| w.to_lowercase()).collect()
            } else {
                words.into_iter().collect()
            }
        };
        let tester = if (op == "~" || op == "!~") && words_re.is_match(pat) {
            Tester::Words {
                words: lower(pat.split('|').map(str::to_string).collect()),
                negate: op == "!~",
            }
        } else if op == "@" || op == "!@" {
            let words = match self.attrs.json.get(pat) {
                None | Some(Value::Null) => {
                    return Err(self.error(format!(
                        "the @ match target '{pat}' is not defined, in rule: {rule_name}"
                    )));
                }
                Some(Value::Array(items)) if items.iter().all(Value::is_string) => items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                Some(_) => {
                    return Err(self.error(format!(
                        "the @ match target '{pat}' must be an array of strings, in rule: {rule_name}"
                    )));
                }
            };
            Tester::Words {
                words: lower(words),
                negate: op == "!@",
            }
        } else if op == "~" || op == "!~" {
            if !pat.contains('$') {
                let source = expand_attributes(self.attrs, &format!("^{pat}$"))?;
                Tester::Regex {
                    regex: build_regex(&self.attrs.language_id, &source, self.ignore_case)?,
                    negate: op == "!~",
                }
            } else {
                Tester::DynamicRegex(pat.to_string())
            }
        } else {
            Tester::Equals {
                pattern: fix_case(self.ignore_case, pat),
                substitute: pat.contains('$'),
                negate: op == "!=",
            }
        };
        Ok(Guard::Test { scrutinee, tester })
    }

    /// Monaco's `addRules`.
    fn add_rules(
        &mut self,
        state: &str,
        out: &mut Vec<Rule>,
        rules: &Value,
        depth: usize,
    ) -> Result<(), MonarchError> {
        let Some(rules) = rules.as_array() else {
            return Err(self.error(format!("the rules of '{state}' must be an array")));
        };
        if depth > MAX_INCLUDE_DEPTH {
            return Err(self.error(format!("includes nested too deeply at: {state}")));
        }
        for rule in rules {
            if let Some(include) = rule.get("include").filter(|i| !i.is_null()) {
                let Some(include) = include.as_str() else {
                    return Err(self.error(format!(
                        "an 'include' attribute must be a string at: {state}"
                    )));
                };
                let include = include.strip_prefix('@').unwrap_or(include);
                let Some(target) = self.states.get(include).filter(|t| !t.is_null()) else {
                    return Err(self.error(format!(
                        "include target '{include}' is not defined at: {state}"
                    )));
                };
                self.add_rules(&format!("{state}.{include}"), out, target, depth + 1)?;
                continue;
            }
            let mut name = state.to_string();
            let (regex, at_line_start, action) = match rule {
                Value::Array(parts) if (1..=3).contains(&parts.len()) => {
                    let (regex, at_line_start) = self.set_regex(&mut name, &parts[0])?;
                    let action = if parts.len() == 3 {
                        match &parts[1] {
                            Value::String(token) => {
                                let mut object = Map::new();
                                object.insert("token".into(), Value::String(token.clone()));
                                object.insert("next".into(), parts[2].clone());
                                self.action(&name, Some(&Value::Object(object)))?
                            }
                            Value::Object(object) => {
                                let mut object = object.clone();
                                object.insert("next".into(), parts[2].clone());
                                self.action(&name, Some(&Value::Object(object)))?
                            }
                            // An array keeps its own actions (Monaco sets
                            // `next` on the array, which is then ignored).
                            Value::Array(_) => self.action(&name, Some(&parts[1]))?,
                            _ => {
                                return Err(self.error(format!(
                                    "a next state as the last element of a rule can only be given if the action is either an object or a string, at: {state}"
                                )));
                            }
                        }
                    } else {
                        self.action(&name, parts.get(1))?
                    };
                    (regex, at_line_start, action)
                }
                Value::Object(object) => {
                    let Some(regex) = object.get("regex").filter(|r| !r.is_null()) else {
                        return Err(self.error(format!(
                            "a rule must either be an array, or an object with a 'regex' or 'include' field at: {state}"
                        )));
                    };
                    if let Some(rule_name) = object.get("name").and_then(Value::as_str) {
                        name = rule_name.to_string();
                    }
                    let (regex, at_line_start) = self.set_regex(&mut name, regex)?;
                    let action = self.action(&name, object.get("action"))?;
                    (regex, at_line_start, action)
                }
                _ => {
                    return Err(self.error(format!(
                        "a rule must either be an array, or an object with a 'regex' or 'include' field at: {state}"
                    )));
                }
            };
            out.push(Rule {
                name,
                regex,
                action,
                match_only_at_line_start: at_line_start,
            });
        }
        Ok(())
    }
}

/// Monaco `compile`: checks `json` and compiles it into a lexer, with the
/// lexer's attributes for guards and `@attr` references.
pub(crate) fn compile(
    language_id: &str,
    json: &Value,
) -> Result<(Lexer, Attributes), MonarchError> {
    let Some(object) = json.as_object() else {
        return Err(MonarchError(
            "Monarch: expecting a language definition object".into(),
        ));
    };
    let bool_attr = |name: &str| object.get(name).and_then(Value::as_bool).unwrap_or(false);
    let string_attr = |name: &str| object.get(name).and_then(Value::as_str).map(str::to_string);
    let language: Arc<str> = language_id.into();
    let ignore_case = bool_attr("ignoreCase");
    let include_lf = bool_attr("includeLF");
    let token_postfix = string_attr("tokenPostfix").unwrap_or_else(|| format!(".{language_id}"));
    let default_token = string_attr("defaultToken").unwrap_or_else(|| "source".into());
    let attrs = Attributes {
        json: object.clone(),
        language_id: language.clone(),
        default_token: default_token.clone(),
        ignore_case,
    };

    let Some(states) = object.get("tokenizer").and_then(Value::as_object) else {
        return Err(error(
            language_id,
            "a language definition must define the 'tokenizer' attribute as an object",
        ));
    };
    let mut compiler = Compiler {
        attrs: &attrs,
        states,
        ignore_case,
        uses_embedded: false,
    };
    let mut start = string_attr("start");
    let mut tokenizer = HashMap::new();
    for (key, rules) in states {
        if start.is_none() {
            start = Some(key.clone());
        }
        let mut compiled = Vec::new();
        compiler.add_rules(&format!("tokenizer.{key}"), &mut compiled, rules, 0)?;
        tokenizer.insert(key.clone(), compiled);
    }
    let uses_embedded = compiler.uses_embedded;

    let default_brackets = [
        ("{", "}", "delimiter.curly"),
        ("[", "]", "delimiter.square"),
        ("(", ")", "delimiter.parenthesis"),
        ("<", ">", "delimiter.angle"),
    ];
    let mut brackets = Vec::new();
    match object.get("brackets") {
        None | Some(Value::Null) => {
            for (open, close, token) in default_brackets {
                brackets.push(Bracket {
                    token: format!("{token}{token_postfix}"),
                    open: open.into(),
                    close: close.into(),
                });
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                let (open, close, token) = match item {
                    Value::Array(parts) if parts.len() == 3 => (&parts[0], &parts[1], &parts[2]),
                    Value::Object(desc) => (
                        desc.get("open").unwrap_or(&Value::Null),
                        desc.get("close").unwrap_or(&Value::Null),
                        desc.get("token").unwrap_or(&Value::Null),
                    ),
                    _ => (&Value::Null, &Value::Null, &Value::Null),
                };
                if open == close {
                    return Err(error(
                        language_id,
                        format!(
                            "open and close brackets in a 'brackets' attribute must be different: {}\n hint: use the 'bracket' attribute if matching on equal brackets is required.",
                            open.as_str().unwrap_or_default()
                        ),
                    ));
                }
                match (open.as_str(), close.as_str(), token.as_str()) {
                    (Some(open), Some(close), Some(token)) => brackets.push(Bracket {
                        token: format!("{token}{token_postfix}"),
                        open: fix_case(ignore_case, open),
                        close: fix_case(ignore_case, close),
                    }),
                    _ => {
                        return Err(error(
                            language_id,
                            "every element in the 'brackets' array must be a '{open,close,token}' object or array",
                        ));
                    }
                }
            }
        }
        Some(_) => {
            return Err(error(
                language_id,
                "the 'brackets' attribute must be defined as an array",
            ));
        }
    }

    let lexer = Lexer {
        language_id: language,
        include_lf,
        ignore_case,
        max_stack: 100,
        start: start.unwrap_or_default(),
        token_postfix,
        default_action: Action::Token(default_token.clone()),
        default_token,
        uses_embedded,
        tokenizer,
        brackets,
    };
    Ok((lexer, attrs))
}
