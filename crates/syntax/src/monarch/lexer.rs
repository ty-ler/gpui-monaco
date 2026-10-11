//! Running a compiled Monarch grammar over a line (Monaco
//! `monarchLexer.ts`, MIT).

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;

use super::compile::{
    Action, Attributes, Lexer, Rule, error, find_rules, fix_case, sanitize, substitute_matches,
};
use super::{EmbeddedLanguages, MonarchError, MonarchState, MonarchToken, StackState};

/// Monaco `EmbeddedLanguageData`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EmbeddedData {
    pub language_id: Arc<str>,
    /// The embedded tokenizer's state; `None` is Monaco's `NullState`.
    pub state: Option<MonarchState>,
}

/// Monaco `MonarchClassicTokensCollector`.
pub(crate) struct Collector {
    pub tokens: Vec<MonarchToken>,
    language: Option<Arc<str>>,
    last_type: Option<String>,
    last_language: Option<Arc<str>>,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            language: None,
            last_type: None,
            last_language: None,
        }
    }

    fn enter_language(&mut self, language_id: &Arc<str>) {
        self.language = Some(language_id.clone());
    }

    fn emit(&mut self, offset: usize, token_type: String) {
        if self.last_type.as_deref() == Some(token_type.as_str())
            && self.last_language == self.language
        {
            return;
        }
        let language = self.language.clone().unwrap_or_else(|| "".into());
        self.last_type = Some(token_type.clone());
        self.last_language = self.language.clone();
        self.tokens.push(MonarchToken {
            offset,
            token_type,
            language,
        });
    }

    fn nested_language_tokenize(
        &mut self,
        line: &str,
        has_eol: bool,
        data: &EmbeddedData,
        offset_delta: usize,
        embedded: &dyn EmbeddedLanguages,
    ) -> Result<Option<MonarchState>, MonarchError> {
        let Some(tokenizer) = embedded.tokenizer(&data.language_id) else {
            self.enter_language(&data.language_id);
            self.emit(offset_delta, String::new());
            return Ok(data.state.clone());
        };
        let state = data
            .state
            .clone()
            .unwrap_or_else(|| tokenizer.initial_state());
        let nested = tokenizer.try_tokenize(line, has_eol, &state, embedded)?;
        self.tokens
            .extend(nested.tokens.into_iter().map(|token| MonarchToken {
                offset: token.offset + offset_delta,
                ..token
            }));
        self.last_type = None;
        self.last_language = None;
        self.language = None;
        Ok(Some(nested.end_state))
    }
}

/// `pos` moved back by `units` UTF-16 code units, not before 0.
fn back_utf16(line: &str, pos: usize, units: i64) -> usize {
    let mut remaining = units;
    let mut pos = pos.min(line.len());
    for c in line[..pos].chars().rev() {
        if remaining <= 0 {
            break;
        }
        remaining -= c.len_utf16() as i64;
        pos -= c.len_utf8();
    }
    pos
}

/// A pending group match: the actions for the regex's groups.
struct GroupMatching<'a> {
    matches: Vec<String>,
    rule: Option<&'a Rule>,
    groups: VecDeque<(&'a Action, String)>,
}

/// The result of resolving an action: a token string or a group.
enum Outcome<'a> {
    Token(String),
    Group(&'a [Action]),
}

pub(crate) struct Run<'a> {
    pub lexer: &'a Lexer,
    pub attrs: &'a Attributes,
    pub embedded: &'a dyn EmbeddedLanguages,
}

impl<'a> Run<'a> {
    fn error(&self, message: impl std::fmt::Display) -> MonarchError {
        error(&self.lexer.language_id, message)
    }

    fn rule_name(rule: Option<&Rule>) -> &str {
        rule.map_or("(unknown)", |rule| rule.name.as_str())
    }

    /// The compiled lexer's string fields, for `$@attr` in actions.
    fn runtime_attr(&self, name: &str) -> Option<String> {
        match name {
            "languageId" => Some(self.lexer.language_id.to_string()),
            "start" => Some(self.lexer.start.clone()),
            "tokenPostfix" => Some(self.lexer.token_postfix.clone()),
            "defaultToken" => Some(self.lexer.default_token.clone()),
            _ => None,
        }
    }

    fn substitute(&self, s: &str, id: &str, matches: &[String], state: &str) -> String {
        substitute_matches(
            &|name| self.runtime_attr(name),
            self.lexer.ignore_case,
            s,
            id,
            matches,
            state,
        )
    }

    /// Monaco `_tokenize`.
    pub fn tokenize(
        &self,
        line: &str,
        has_eol: bool,
        state: &MonarchState,
        collector: &mut Collector,
    ) -> Result<MonarchState, MonarchError> {
        if state.0.embedded.is_some() {
            self.nested_tokenize(line, has_eol, &state.0, 0, collector)
        } else {
            self.my_tokenize(line, has_eol, &state.0, 0, collector)
        }
    }

    /// Monaco `_findLeavingNestedLanguageOffset`.
    fn find_leaving_nested_language_offset(
        &self,
        line: &str,
        state: &StackState,
    ) -> Result<Option<usize>, MonarchError> {
        let top = state.top();
        let rules = find_rules(self.lexer, top)
            .ok_or_else(|| self.error(format!("tokenizer state is not defined: {top}")))?;
        let mut pop_offset: Option<usize> = None;
        let mut has_embedded_pop_rule = false;
        for rule in rules {
            let pops = match &rule.action {
                Action::Full(full) => full.next_embedded.as_deref() == Some("@pop"),
                Action::Cases(cases) => cases.has_embedded_end_in_cases,
                _ => false,
            };
            if !pops {
                continue;
            }
            has_embedded_pop_rule = true;
            let regex = rule.unanchored(self.lexer, top)?;
            let Some(found) = regex.find(line).ok().flatten().map(|m| m.start()) else {
                continue;
            };
            if found != 0 && rule.match_only_at_line_start {
                continue;
            }
            if pop_offset.is_none_or(|offset| found < offset) {
                pop_offset = Some(found);
            }
        }
        if !has_embedded_pop_rule {
            return Err(self.error(format!(
                "no rule containing nextEmbedded: \"@pop\" in tokenizer embedded state: {top}"
            )));
        }
        Ok(pop_offset)
    }

    /// Monaco `_nestedTokenize`.
    fn nested_tokenize(
        &self,
        line: &str,
        has_eol: bool,
        state: &StackState,
        offset_delta: usize,
        collector: &mut Collector,
    ) -> Result<MonarchState, MonarchError> {
        let data = state.embedded.as_ref().expect("in an embedded language");
        let Some(pop_offset) = self.find_leaving_nested_language_offset(line, state)? else {
            // The embedded language doesn't end on this line.
            let nested_end = collector.nested_language_tokenize(
                line,
                has_eol,
                data,
                offset_delta,
                self.embedded,
            )?;
            return Ok(MonarchState::new(
                state.stack.clone(),
                Some(EmbeddedData {
                    language_id: data.language_id.clone(),
                    state: nested_end,
                }),
            ));
        };
        let nested_line = &line[..pop_offset];
        if !nested_line.is_empty() {
            collector.nested_language_tokenize(
                nested_line,
                false,
                data,
                offset_delta,
                self.embedded,
            )?;
        }
        self.my_tokenize(
            &line[pop_offset..],
            has_eol,
            state,
            offset_delta + pop_offset,
            collector,
        )
    }

    /// Monaco `_getNestedEmbeddedLanguageData`.
    fn nested_embedded_language_data(&self, name: &str) -> EmbeddedData {
        // Language names, MIME types and ids.
        match self.embedded.language_id(name) {
            Some(language_id) => EmbeddedData {
                state: self
                    .embedded
                    .tokenizer(&language_id)
                    .map(|tokenizer| tokenizer.initial_state()),
                language_id,
            },
            None => EmbeddedData {
                language_id: name.into(),
                state: None,
            },
        }
    }

    /// Monaco `_myTokenize`.
    fn my_tokenize(
        &self,
        line_without_lf: &str,
        has_eol: bool,
        line_state: &StackState,
        offset_delta: usize,
        collector: &mut Collector,
    ) -> Result<MonarchState, MonarchError> {
        let lexer = self.lexer;
        collector.enter_language(&lexer.language_id);
        let line_without_lf_len = line_without_lf.len();
        let line: Cow<str> = if has_eol && lexer.include_lf {
            Cow::Owned(format!("{line_without_lf}\n"))
        } else {
            Cow::Borrowed(line_without_lf)
        };
        let line_len = line.len();

        let mut embedded = line_state.embedded.clone();
        let mut stack = line_state.stack.clone();
        let mut pos = 0;
        let mut group_matching: Option<GroupMatching<'a>> = None;
        // Evaluate rules at least once for an empty line
        // (microsoft/monaco-editor#1235).
        let mut force_evaluation = true;

        while force_evaluation || pos < line_len {
            let pos0 = pos;
            let stack_len0 = stack.len();
            let group_len0 = group_matching.as_ref().map_or(0, |g| g.groups.len());
            let state: Arc<str> = stack.last().cloned().unwrap_or_else(|| "".into());

            let mut matches: Option<Vec<String>> = None;
            let mut matched: Option<String> = None;
            let mut action: Option<&'a Action> = None;
            let mut rule: Option<&'a Rule> = None;
            let mut entering_embedded_language: Option<String> = None;

            if let Some(group) = group_matching.as_mut() {
                // Process group matches first.
                matches = Some(group.matches.clone());
                let (group_action, group_matched) =
                    group.groups.pop_front().expect("a pending group");
                matched = Some(group_matched);
                action = Some(group_action);
                rule = group.rule;
                if group.groups.is_empty() {
                    group_matching = None;
                }
            } else {
                if !force_evaluation && pos >= line_len {
                    break;
                }
                force_evaluation = false;
                let rules = find_rules(lexer, &state).ok_or_else(|| {
                    self.error(format!("tokenizer state is not defined: {state}"))
                })?;
                let rest = &line[pos..];
                for candidate in rules {
                    if pos != 0 && candidate.match_only_at_line_start {
                        continue;
                    }
                    let dynamic;
                    let regex = match candidate.static_anchored() {
                        Some(regex) => regex,
                        None => {
                            dynamic = candidate.anchored(lexer, &state)?;
                            &*dynamic
                        }
                    };
                    let captures = regex.captures(rest).map_err(|e| {
                        self.error(format!(
                            "regular expression failed in rule {}: {e}",
                            candidate.name
                        ))
                    })?;
                    if let Some(captures) = captures {
                        let groups: Vec<String> = captures
                            .iter()
                            .map(|group| group.map_or_else(String::new, |g| g.as_str().to_string()))
                            .collect();
                        matched = Some(groups[0].clone());
                        matches = Some(groups);
                        action = Some(&candidate.action);
                        rule = Some(candidate);
                        break;
                    }
                }
            }

            let mut matches = matches.unwrap_or_else(|| vec![String::new()]);
            let mut matched = matched.unwrap_or_default();
            let mut action = match action {
                Some(action) => action,
                None => {
                    // Nothing matched: advance one character with the
                    // default token.
                    if let Some(c) = line[pos..].chars().next() {
                        matched = c.to_string();
                        matches = vec![matched.clone()];
                    }
                    &lexer.default_action
                }
            };

            pos += matched.len();

            // Resolve `cases`.
            while let Action::Cases(cases) = action {
                action = cases.select(
                    lexer,
                    self.attrs,
                    &matched,
                    &matches,
                    &state,
                    pos == line_len,
                )?;
            }

            let outcome = match action {
                Action::Token(token) => Outcome::Token(token.clone()),
                Action::Group(group) => Outcome::Group(group),
                Action::Cases(_) => unreachable!("resolved above"),
                Action::Full(full) => {
                    let token = if full.token_subst {
                        self.substitute(&full.token, &matched, &matches, &state)
                    } else {
                        full.token.clone()
                    };

                    if let Some(next_embedded) = &full.next_embedded {
                        if next_embedded == "@pop" {
                            if embedded.is_none() {
                                return Err(
                                    self.error("cannot pop embedded language if not inside one")
                                );
                            }
                            embedded = None;
                        } else if embedded.is_some() {
                            return Err(self.error(
                                "cannot enter embedded language from within an embedded language",
                            ));
                        } else {
                            entering_embedded_language =
                                Some(self.substitute(next_embedded, &matched, &matches, &state));
                        }
                    }

                    if full.go_back != 0 {
                        pos = back_utf16(&line, pos, full.go_back);
                    }

                    if let Some(switch_to) = full.switch_to.as_deref().filter(|s| !s.is_empty()) {
                        let next_state = self.substitute(switch_to, &matched, &matches, &state);
                        let next_state = next_state.strip_prefix('@').unwrap_or(&next_state);
                        if find_rules(lexer, next_state).is_none() {
                            return Err(self.error(format!(
                                "trying to switch to a state '{next_state}' that is undefined in rule: {}",
                                Self::rule_name(rule)
                            )));
                        }
                        stack.pop();
                        stack.push(next_state.into());
                    } else if let Some(next) = &full.next {
                        match next.as_str() {
                            "@push" => {
                                if stack.len() >= lexer.max_stack {
                                    return Err(self.error(format!(
                                        "maximum tokenizer stack size reached: [{},{},...]",
                                        state,
                                        stack.get(stack.len().wrapping_sub(2)).map_or("", |s| s)
                                    )));
                                }
                                stack.push(state.clone());
                            }
                            "@pop" => {
                                if stack.len() <= 1 {
                                    return Err(self.error(format!(
                                        "trying to pop an empty stack in rule: {}",
                                        Self::rule_name(rule)
                                    )));
                                }
                                stack.pop();
                            }
                            "@popall" => stack.truncate(1),
                            _ => {
                                let next_state = self.substitute(next, &matched, &matches, &state);
                                let next_state =
                                    next_state.strip_prefix('@').unwrap_or(&next_state);
                                if find_rules(lexer, next_state).is_none() {
                                    return Err(self.error(format!(
                                        "trying to set a next state '{next_state}' that is undefined in rule: {}",
                                        Self::rule_name(rule)
                                    )));
                                }
                                stack.push(next_state.into());
                            }
                        }
                    }

                    if let Some(message) = full.log.as_deref().filter(|s| !s.is_empty()) {
                        log::info!(
                            "{}: {}",
                            lexer.language_id,
                            self.substitute(message, &matched, &matches, &state)
                        );
                    }
                    Outcome::Token(token)
                }
            };

            match outcome {
                Outcome::Group(group) => {
                    if group_matching
                        .as_ref()
                        .is_some_and(|g| !g.groups.is_empty())
                    {
                        return Err(self.error(format!(
                            "groups cannot be nested: {}",
                            Self::rule_name(rule)
                        )));
                    }
                    if matches.len() != group.len() + 1 {
                        return Err(self.error(format!(
                            "matched number of groups does not match the number of actions in rule: {}",
                            Self::rule_name(rule)
                        )));
                    }
                    let total: usize = matches[1..].iter().map(String::len).sum();
                    if total != matched.len() {
                        return Err(self.error(format!(
                            "with groups, all characters should be matched in consecutive groups in rule: {}",
                            Self::rule_name(rule)
                        )));
                    }
                    pos -= matched.len();
                    group_matching = Some(GroupMatching {
                        groups: group.iter().zip(matches[1..].iter().cloned()).collect(),
                        matches,
                        rule,
                    });
                    continue;
                }
                Outcome::Token(mut result) => {
                    if result == "@rematch" {
                        pos -= matched.len();
                        matched = String::new();
                        result = String::new();
                        // `nextEmbedded` still switches the language.
                        if let Some(language) = entering_embedded_language {
                            return self.enter_embedded_language(
                                &language,
                                pos,
                                line_len,
                                line_without_lf,
                                has_eol,
                                stack,
                                offset_delta,
                                collector,
                            );
                        }
                    }

                    // Check progress.
                    if matched.is_empty() {
                        let group_len = group_matching.as_ref().map_or(0, |g| g.groups.len());
                        if line_len == 0
                            || stack_len0 != stack.len()
                            || stack.last().map(|s| &**s) != Some(&*state)
                            || group_len != group_len0
                        {
                            continue;
                        }
                        return Err(self.error(format!(
                            "no progress in tokenizer in rule: {}",
                            Self::rule_name(rule)
                        )));
                    }

                    let token_type = if let Some(rest) = result.strip_prefix("@brackets") {
                        let Some(token) = find_bracket(lexer, &matched) else {
                            return Err(self.error(format!(
                                "@brackets token returned but no bracket defined as: {matched}"
                            )));
                        };
                        sanitize(&format!("{token}{rest}"))
                    } else if result.is_empty() {
                        String::new()
                    } else {
                        sanitize(&format!("{result}{}", lexer.token_postfix))
                    };
                    if pos0 < line_without_lf_len {
                        collector.emit(pos0 + offset_delta, token_type);
                    }
                }
            }

            if let Some(language) = entering_embedded_language {
                return self.enter_embedded_language(
                    &language,
                    pos,
                    line_len,
                    line_without_lf,
                    has_eol,
                    stack,
                    offset_delta,
                    collector,
                );
            }
        }

        Ok(MonarchState::new(stack, embedded))
    }

    /// Monaco's `computeNewStateForEmbeddedLanguage`.
    #[allow(clippy::too_many_arguments)]
    fn enter_embedded_language(
        &self,
        language: &str,
        pos: usize,
        line_len: usize,
        line_without_lf: &str,
        has_eol: bool,
        stack: Vec<Arc<str>>,
        offset_delta: usize,
        collector: &mut Collector,
    ) -> Result<MonarchState, MonarchError> {
        let data = self.nested_embedded_language_data(language);
        if pos < line_len {
            // The embedded language starts on this line.
            let rest = &line_without_lf[pos.min(line_without_lf.len())..];
            let state = StackState {
                stack,
                embedded: Some(data),
            };
            self.nested_tokenize(rest, has_eol, &state, offset_delta + pos, collector)
        } else {
            Ok(MonarchState::new(stack, Some(data)))
        }
    }
}

/// Monaco `findBracket`: the token of the bracket `matched` opens or
/// closes.
fn find_bracket<'a>(lexer: &'a Lexer, matched: &str) -> Option<&'a str> {
    if matched.is_empty() {
        return None;
    }
    let matched = fix_case(lexer.ignore_case, matched);
    lexer
        .brackets
        .iter()
        .find(|bracket| bracket.open == matched || bracket.close == matched)
        .map(|bracket| bracket.token.as_str())
}
