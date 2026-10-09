//! Token types for the language tests: the tiny tokenizers Monaco's tests
//! register (`setupAutoClosingLanguageTokenization` in
//! `src/vs/editor/test/browser/controller/cursor.test.ts` and
//! `registerTokenizationSupport` in
//! `src/vs/editor/contrib/indentation/test/browser/indentation.test.ts`,
//! MIT).

use crate::cursor::StandardTokenType;

/// One `(startIndex, type)` per token, as in Monaco's indentation tests.
pub type LineTokens = Vec<(u32, StandardTokenType)>;

#[derive(Clone, Debug, Default)]
pub enum Tokenizer {
    /// No tokenizer: everything is [`StandardTokenType::Other`].
    #[default]
    None,
    /// The stateful string/comment tokenizer of Monaco's auto-closing tests.
    AutoClosing,
    /// Fixed tokens per line (lines without an entry are `Other`).
    Fixed(Vec<LineTokens>),
}

impl Tokenizer {
    /// The token type of every UTF-16 unit of every line.
    pub fn tokenize(&self, lines: &[String]) -> Vec<Vec<StandardTokenType>> {
        match self {
            Tokenizer::None => lines
                .iter()
                .map(|l| vec![StandardTokenType::Other; l.encode_utf16().count()])
                .collect(),
            Tokenizer::AutoClosing => {
                let mut state = State::Base(None);
                lines
                    .iter()
                    .map(|line| {
                        let (types, next) = tokenize_auto_closing_line(line, state.clone());
                        state = next;
                        types
                    })
                    .collect()
            }
            Tokenizer::Fixed(tokens) => lines
                .iter()
                .enumerate()
                .map(|(row, line)| {
                    let len = line.encode_utf16().count();
                    let mut types = vec![StandardTokenType::Other; len];
                    if let Some(line_tokens) = tokens.get(row) {
                        for (i, &(start, token)) in line_tokens.iter().enumerate() {
                            let end = line_tokens
                                .get(i + 1)
                                .map_or(len, |&(next, _)| (next as usize).min(len));
                            for t in types.iter_mut().take(end).skip(start as usize) {
                                *t = token;
                            }
                        }
                    }
                    types
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug)]
enum State {
    Base(Option<Box<State>>),
    Str(char, Box<State>),
    BlockComment(Box<State>),
}

fn tokenize_auto_closing_line(line: &str, mut state: State) -> (Vec<StandardTokenType>, State) {
    use StandardTokenType::*;
    let chars: Vec<char> = line.chars().collect();
    let mut types: Vec<StandardTokenType> = Vec::new();
    let mut i = 0;
    let emit = |types: &mut Vec<StandardTokenType>, i: &mut usize, len: usize, t| {
        let end = (*i + len).min(chars.len());
        for c in &chars[*i..end] {
            for _ in 0..c.len_utf16() {
                types.push(t);
            }
        }
        *i = end;
    };
    while i < chars.len() {
        let rest = &chars[i..];
        match state.clone() {
            State::Base(parent) => {
                let run = rest
                    .iter()
                    .take_while(|c| !matches!(c, '\'' | '"' | '`' | '{' | '}' | '/'))
                    .count();
                if run > 0 {
                    emit(&mut types, &mut i, run, Other);
                } else if matches!(rest[0], '\'' | '"' | '`') {
                    let ch = rest[0];
                    emit(&mut types, &mut i, 1, String);
                    state = State::Str(ch, Box::new(state));
                } else if rest[0] == '{' {
                    emit(&mut types, &mut i, 1, Other);
                    state = State::Base(Some(Box::new(state)));
                } else if rest[0] == '}' {
                    emit(&mut types, &mut i, 1, Other);
                    state = parent.map(|p| *p).unwrap_or(State::Base(None));
                } else if rest.starts_with(&['/', '/']) {
                    emit(&mut types, &mut i, rest.len(), Comment);
                } else if rest.starts_with(&['/', '*']) {
                    emit(&mut types, &mut i, 2, Comment);
                    state = State::BlockComment(Box::new(state));
                } else {
                    emit(&mut types, &mut i, 1, Other);
                }
            }
            State::Str(quote, parent) => {
                let run = rest
                    .iter()
                    .take_while(|c| !matches!(c, '\\' | '\'' | '"' | '`' | '$'))
                    .count();
                if run > 0 {
                    emit(&mut types, &mut i, run, String);
                } else if rest[0] == '\\' {
                    emit(&mut types, &mut i, 2, String);
                } else if rest[0] == quote {
                    emit(&mut types, &mut i, 1, String);
                    state = *parent;
                } else if rest.starts_with(&['$', '{']) {
                    emit(&mut types, &mut i, 2, Other);
                    state = State::Base(Some(Box::new(state)));
                } else {
                    emit(&mut types, &mut i, 1, Other);
                }
            }
            State::BlockComment(parent) => {
                let run = rest.iter().take_while(|&&c| c != '*').count();
                if run > 0 {
                    // Monaco's test tokenizer marks block comment bodies as
                    // strings.
                    emit(&mut types, &mut i, run, String);
                } else if rest.starts_with(&['*', '/']) {
                    emit(&mut types, &mut i, 2, Comment);
                    state = *parent;
                } else {
                    emit(&mut types, &mut i, 1, Other);
                }
            }
        }
    }
    (types, state)
}
